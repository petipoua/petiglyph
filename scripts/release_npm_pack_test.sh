#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

dist_dir="${1:-dist-release}"

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required" >&2
  exit 1
fi

./scripts/release_sync_versions.sh
./scripts/release_stage_npm_artifacts.sh "$dist_dir"

tarball_dir="$(mktemp -d)"
workdir="$(mktemp -d)"
trap 'rm -rf "$tarball_dir" "$workdir"' EXIT

while IFS= read -r bin_path; do
  [[ -n "${bin_path}" ]] || continue
  if [[ ! -f "${bin_path}" ]]; then
    echo "Missing staged binary: $bin_path" >&2
    exit 1
  fi
done < <("$repo_root/scripts/distribution_matrix.py" --npm-bin-paths)

for pkg in npm/*; do
  if [[ -f "$pkg/package.json" ]]; then
    echo "Checking package contents: $pkg"
    (
      cd "$pkg"
      npm pack --dry-run >/dev/null
      npm pack --pack-destination "$tarball_dir" >/dev/null
    )
  fi
done

meta_tgz="$(ls "$tarball_dir"/petiglyph-[0-9]*.tgz 2>/dev/null | head -n1 || true)"
first_platform_dir="$("$repo_root/scripts/distribution_matrix.py" --npm-package-dirs | head -n1 || true)"
if [[ -z "$first_platform_dir" ]]; then
  echo "No platform package directories found in distribution matrix" >&2
  exit 1
fi
first_platform_slug="$(basename "$first_platform_dir")"
platform_tgz="$(ls "$tarball_dir"/petiglyph-"$first_platform_slug"-*.tgz 2>/dev/null | head -n1 || true)"

if [[ -z "$meta_tgz" || -z "$platform_tgz" ]]; then
  echo "Expected meta and at least one platform tarball in $tarball_dir" >&2
  exit 1
fi

pushd "$workdir" >/dev/null
npm init -y >/dev/null
npm install "$platform_tgz" "$meta_tgz" >/dev/null
npx petiglyph --help >/dev/null
popd >/dev/null

echo "npm local pack/install smoke test passed"
