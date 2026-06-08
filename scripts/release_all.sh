#!/usr/bin/env bash
set -euo pipefail

invocation_dir="$PWD"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/release_all.sh vX.Y.Z [--notes-file PATH] [--yes]

Publishes or resumes the committed release at HEAD in this order:
  1. Create and push a signed annotated tag, or verify the existing tag.
  2. Run or resume release.yml until the GitHub artifacts exist.
  3. Verify and publish the GitHub Release.
  4. Wait for the npm and PyPI publication workflows.
  5. Publish or verify the AUR package.
  6. Verify all public channels report X.Y.Z.

The version must already be synchronized and committed. GitHub starts the npm
and PyPI workflows concurrently when the draft release is published; this
script waits for both before publishing to the AUR. Rerunning the same tag
resumes incomplete steps, but the tag must still resolve to the same HEAD.

Options:
  --notes-file PATH  Replace the draft body before publication. PATH may be
                     absolute or relative to the directory where the script
                     was invoked.
  --yes              Skip interactive confirmation. A draft release still
                     requires --notes-file.
USAGE
}

die() {
  echo "release: $*" >&2
  exit 1
}

log() {
  printf '\n==> %s\n' "$*" >&2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

cargo_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1
}

clean_staged_npm_binaries() {
  local binary=""
  local removed=0

  while IFS= read -r -d '' binary; do
    git check-ignore -q -- "$binary" \
      || die "refusing to remove non-ignored npm binary: $binary"
    rm -f -- "$binary"
    removed=$((removed + 1))
  done < <(
    find npm \
      -path 'npm/petiglyph-*/bin/*' \
      -type f \
      ! -name '.gitkeep' \
      -print0
  )

  if ((removed > 0)); then
    log "Removed $removed generated npm platform binaries from local staging"
  fi
}

wait_for_new_run() {
  local workflow="$1"
  local sha="$2"
  local previous_id="${3:-}"
  local run_id=""

  log "Waiting for $workflow to start"
  for _ in {1..60}; do
    run_id="$(
      gh run list \
        --workflow "$workflow" \
        --commit "$sha" \
        --limit 20 \
        --json databaseId \
        --jq '.[0].databaseId // empty'
    )"
    if [[ -n "$run_id" && "$run_id" != "$previous_id" ]]; then
      echo "$run_id"
      return 0
    fi
    sleep 5
  done

  die "timed out waiting for a new $workflow run at $sha"
}

ensure_workflow_success() {
  local workflow="$1"
  local tag="$2"
  local sha="$3"
  local run_json=""
  local run_id=""
  local status=""
  local conclusion=""

  run_json="$(
    gh run list \
      --workflow "$workflow" \
      --commit "$sha" \
      --limit 20 \
      --json databaseId,status,conclusion \
      --jq '.[0] // empty'
  )"

  if [[ -n "$run_json" ]]; then
    run_id="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["databaseId"])' <<<"$run_json")"
    status="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' <<<"$run_json")"
    conclusion="$(
      python3 -c 'import json,sys; print(json.load(sys.stdin).get("conclusion") or "")' \
        <<<"$run_json"
    )"

    if [[ "$status" != "completed" ]]; then
      log "Watching in-progress $workflow run $run_id"
      gh run watch "$run_id" --exit-status
      echo "$run_id"
      return 0
    fi

    if [[ "$conclusion" == "success" ]]; then
      log "$workflow already succeeded in run $run_id"
      echo "$run_id"
      return 0
    fi

    log "Rerunning failed $workflow run $run_id"
    gh run rerun "$run_id" --failed
    gh run watch "$run_id" --exit-status
    echo "$run_id"
    return 0
  fi

  # GitHub may take a few seconds to expose an event-triggered run. Give that
  # run time to appear before creating a workflow_dispatch retry.
  for _ in {1..12}; do
    sleep 5
    if workflow_has_run "$workflow" "$sha"; then
      ensure_workflow_success "$workflow" "$tag" "$sha"
      return 0
    fi
  done

  log "Dispatching $workflow for $tag"
  gh workflow run "$workflow" --ref "$tag" -f "tag=$tag"
  run_id="$(wait_for_new_run "$workflow" "$sha")"
  gh run watch "$run_id" --exit-status
  echo "$run_id"
}

remote_tag_commit() {
  local tag="$1"
  local commit=""

  commit="$(
    git ls-remote origin "refs/tags/$tag^{}" \
      | awk 'NR == 1 {print $1}'
  )"
  if [[ -z "$commit" ]]; then
    commit="$(
      git ls-remote origin "refs/tags/$tag" \
        | awk 'NR == 1 {print $1}'
    )"
  fi
  printf '%s\n' "$commit"
}

workflow_has_run() {
  local workflow="$1"
  local sha="$2"
  [[ -n "$(
    gh run list \
      --workflow "$workflow" \
      --commit "$sha" \
      --limit 1 \
      --json databaseId \
      --jq '.[0].databaseId // empty'
  )" ]]
}

npm_any_package_exists() {
  local package_dir=""
  local package_name=""

  while IFS= read -r package_dir; do
    [[ -n "$package_dir" ]] || continue
    package_name="$(node -p "require('./$package_dir/package.json').name")"
    if npm view "$package_name@$version" version >/dev/null 2>&1; then
      return 0
    fi
  done < <(
    {
      ./scripts/distribution_matrix.py --npm-package-dirs
      printf '%s\n' npm/petiglyph
    }
  )
  return 1
}

verify_registry_versions() {
  local expected="$1"
  local npm_name=""
  local npm_version=""

  log "Verifying npm, PyPI, and AUR report $expected"
  while IFS= read -r package_dir; do
    [[ -n "$package_dir" ]] || continue
    npm_name="$(node -p "require('./$package_dir/package.json').name")"
    npm_version=""
    for _ in {1..30}; do
      npm_version="$(npm view "$npm_name@$expected" version 2>/dev/null || true)"
      if [[ "$npm_version" == "$expected" ]]; then
        break
      fi
      sleep 10
    done
    [[ "$npm_version" == "$expected" ]] \
      || die "npm does not report $npm_name@$expected"
  done < <(
    {
      ./scripts/distribution_matrix.py --npm-package-dirs
      printf '%s\n' npm/petiglyph
    }
  )

  VERSION="$expected" python3 <<'PY'
import json
import os
import time
import urllib.request

version = os.environ["VERSION"]
checks = (
    (
        "PyPI",
        f"https://pypi.org/pypi/petiglyph/{version}/json",
        lambda payload: payload["info"]["version"] == version,
    ),
    (
        "AUR",
        "https://aur.archlinux.org/rpc/v5/info?arg[]=petiglyph",
        lambda payload: any(
            result.get("Version", "").rsplit("-", 1)[0] == version
            for result in payload.get("results", [])
        ),
    ),
)

for name, url, matches in checks:
    last_error = None
    for _ in range(30):
        try:
            with urllib.request.urlopen(url, timeout=20) as response:
                if matches(json.load(response)):
                    break
        except Exception as error:
            last_error = error
        time.sleep(10)
    else:
        detail = f": {last_error}" if last_error else ""
        raise SystemExit(f"release: {name} does not report {version}{detail}")
PY
}

aur_pkgrel_for_version() {
  VERSION="$1" python3 <<'PY'
import json
import os
import urllib.request

version = os.environ["VERSION"]
with urllib.request.urlopen(
    "https://aur.archlinux.org/rpc/v5/info?arg[]=petiglyph",
    timeout=20,
) as response:
    payload = json.load(response)

for result in payload.get("results", []):
    aur_version = result.get("Version", "")
    upstream, separator, pkgrel = aur_version.rpartition("-")
    if separator and upstream == version and pkgrel.isdigit():
        print(pkgrel)
        break
PY
}

tag=""
assume_yes=0
notes_file=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --notes-file)
      [[ $# -ge 2 ]] || die "missing value for --notes-file"
      notes_file="$2"
      if [[ "$notes_file" != /* ]]; then
        notes_file="$invocation_dir/$notes_file"
      fi
      shift 2
      ;;
    --yes)
      assume_yes=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    -*)
      die "unknown option: $1"
      ;;
    *)
      [[ -z "$tag" ]] || die "unexpected extra argument: $1"
      tag="$1"
      shift
      ;;
  esac
done

[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] \
  || die "tag must match vX.Y.Z"
version="${tag#v}"
if [[ -n "$notes_file" ]]; then
  [[ -f "$notes_file" ]] || die "release notes file not found: $notes_file"
fi

for command in cargo curl gh git makepkg node npm python3 sha256sum; do
  require_command "$command"
done

[[ "$(git branch --show-current)" == "main" ]] \
  || die "HEAD must be on the main branch"

log "Checking repository and release metadata"
clean_staged_npm_binaries
./scripts/release_assert_clean_tree.sh
gh auth status >/dev/null
git fetch --quiet origin main --tags
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] \
  || die "HEAD must equal origin/main"
[[ "$(cargo_version)" == "$version" ]] \
  || die "Cargo.toml version $(cargo_version) does not match $tag"
./scripts/distribution_matrix.py --check-sync
git ls-remote ssh://aur@aur.archlinux.org/petiglyph.git >/dev/null \
  || die "cannot access the petiglyph AUR repository over SSH"

pkgver="$(sed -n 's/^pkgver=//p' PKGBUILD | head -n1)"
[[ "$pkgver" == "$version" ]] \
  || die "PKGBUILD pkgver $pkgver does not match $version"

head_sha="$(git rev-parse HEAD)"
remote_tag_sha="$(remote_tag_commit "$tag")"
if [[ -n "$remote_tag_sha" && "$remote_tag_sha" != "$head_sha" ]]; then
  die "remote $tag resolves to $remote_tag_sha, not HEAD $head_sha"
fi
if git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
  local_tag_sha="$(git rev-list -n1 "$tag")"
  [[ "$local_tag_sha" == "$head_sha" ]] \
    || die "local $tag resolves to $local_tag_sha, not HEAD $head_sha"
fi

if ((assume_yes == 0)); then
  [[ -t 0 ]] || die "confirmation requires a terminal; pass --yes for automation"
  printf '\nPublish or resume %s from %s to GitHub, npm, PyPI, and AUR? [y/N] ' \
    "$tag" "${head_sha:0:12}"
  read -r answer
  [[ "$answer" =~ ^[Yy]$ ]] || die "release cancelled"
fi

tag_created=0
tag_pushed=0
dist_dir=""
aur_backup_dir=""
cleanup_unpushed_tag() {
  if ((tag_created == 1 && tag_pushed == 0)); then
    if ! git ls-remote --exit-code --tags origin "refs/tags/$tag" \
      >/dev/null 2>&1; then
      git tag -d "$tag" >/dev/null 2>&1 || true
    fi
  fi
}

cleanup() {
  set +e
  if [[ -n "$aur_backup_dir" && -d "$aur_backup_dir" ]]; then
    cp "$aur_backup_dir/PKGBUILD" "$repo_root/PKGBUILD"
    cp "$aur_backup_dir/.SRCINFO" "$repo_root/.SRCINFO"
    rm -rf "$aur_backup_dir"
  fi
  if [[ -n "$dist_dir" && -d "$dist_dir" ]]; then
    rm -rf "$dist_dir"
  fi
  cleanup_unpushed_tag
}
trap cleanup EXIT

if [[ -z "$remote_tag_sha" ]]; then
  log "Creating signed tag $tag at $head_sha"
  if ! git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
    git tag -s "$tag" -m "petiglyph $tag" "$head_sha"
    tag_created=1
  fi
  git push origin "refs/tags/$tag"
  tag_pushed=1
else
  log "Tag $tag already resolves to HEAD; resuming release"
fi

release_run_id="$(ensure_workflow_success release.yml "$tag" "$head_sha")"

log "Verifying GitHub Release artifacts"
gh release view "$tag" >/dev/null 2>&1 \
  || die "$tag artifact workflow succeeded but no GitHub Release exists"
is_draft="$(gh release view "$tag" --json isDraft --jq '.isDraft')"
dist_dir="$(mktemp -d)"
gh release download "$tag" --dir "$dist_dir"
archive_count="$(
  find "$dist_dir" -maxdepth 1 -type f \
    \( -name "petiglyph-$tag-*.tar.gz" -o -name "petiglyph-$tag-*.zip" \) \
    | wc -l
)"
[[ "$archive_count" -eq 8 ]] \
  || die "expected 8 release archives, found $archive_count"
[[ -f "$dist_dir/SHA256SUMS" ]] || die "GitHub Release is missing SHA256SUMS"
(cd "$dist_dir" && sha256sum -c SHA256SUMS)
gh release verify "$tag" >/dev/null

if [[ -n "$notes_file" ]]; then
  log "Applying release notes from $notes_file"
  gh release edit "$tag" --notes-file "$notes_file"
elif [[ "$is_draft" == "true" ]]; then
  if ((assume_yes == 1)); then
    die "--yes requires --notes-file while the GitHub Release is still a draft"
  fi
  release_url="$(gh release view "$tag" --json url --jq '.url')"
  printf '\nEdit and save the release notes at:\n%s\n' "$release_url" >&2
  printf 'Press Enter after all template placeholders have been replaced. ' >&2
  read -r
fi

release_body="$(gh release view "$tag" --json body --jq '.body')"
if grep -Eq '<[^>]+>' <<<"$release_body"; then
  die "release notes still contain template placeholders"
fi

if [[ "$is_draft" == "true" ]]; then
  log "Publishing GitHub Release $tag"
  gh release edit "$tag" --draft=false
else
  log "GitHub Release $tag is already published"
fi

if npm_any_package_exists \
  && ! workflow_has_run npm-publish.yml "$head_sha"; then
  die "npm already contains $version without a matching workflow run for $tag"
fi
if curl -fsS "https://pypi.org/pypi/petiglyph/$version/json" >/dev/null 2>&1 \
  && ! workflow_has_run pypi-publish.yml "$head_sha"; then
  die "PyPI already contains $version without a matching workflow run for $tag"
fi

npm_run_id="$(ensure_workflow_success npm-publish.yml "$tag" "$head_sha")"
pypi_run_id="$(ensure_workflow_success pypi-publish.yml "$tag" "$head_sha")"

aur_pkgrel="$(aur_pkgrel_for_version "$version")"
aur_pkgrel="${aur_pkgrel:-1}"
log "Publishing or verifying AUR package $version-$aur_pkgrel"
aur_backup_dir="$(mktemp -d)"
cp PKGBUILD "$aur_backup_dir/PKGBUILD"
cp .SRCINFO "$aur_backup_dir/.SRCINFO"
./scripts/release_publish_aur.sh "$version" --pkgrel "$aur_pkgrel"
cp "$aur_backup_dir/PKGBUILD" PKGBUILD
cp "$aur_backup_dir/.SRCINFO" .SRCINFO
rm -rf "$aur_backup_dir"
aur_backup_dir=""

gh release view "$tag" --json isDraft --jq \
  'if .isDraft then error("release is still a draft") else empty end'
verify_registry_versions "$version"

log "Release $tag published successfully"
