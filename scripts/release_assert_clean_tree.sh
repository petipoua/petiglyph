#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

failures=0

report_failure() {
  echo "release hygiene: $*" >&2
  failures=1
}

status="$(git status --short)"
if [[ -n "$status" ]]; then
  report_failure "working tree is not clean"
  echo "$status" >&2
fi

if ! ./scripts/release_sync_readmes.sh --check; then
  report_failure "package README is not synchronized"
fi

if ! command -v cargo >/dev/null 2>&1; then
  report_failure "cargo is required to inspect package contents"
else
  package_list="$(cargo package --list --allow-dirty)"
  scratch_paths="$(
    printf '%s\n' "$package_list" \
      | grep -E '^(test-[^/]*|test[0-9][^/]*|test_parse[^/]*|test_ws[^/]*)(/|$)' \
      | grep -Ev '^test-assets(/|$)' \
      || true
  )"
  if [[ -n "$scratch_paths" ]]; then
    report_failure "cargo package would include scratch paths"
    printf '%s\n' "$scratch_paths" >&2
  fi
fi

staged_npm_bins="$(find npm -path 'npm/petiglyph-*/bin/*' -type f ! -name '.gitkeep' -print | sort)"
if [[ -n "$staged_npm_bins" ]]; then
  report_failure "generated npm platform binaries remain from local release staging"
  printf '%s\n' "$staged_npm_bins" >&2
fi

unignored_artifacts=()
while IFS= read -r -d '' artifact; do
  rel="${artifact#./}"
  if ! git check-ignore -q -- "$rel"; then
    unignored_artifacts+=("$rel")
  fi
done < <(
  find . \
    -path './.git' -prune -o \
    -path './target' -prune -o \
    -path './.makepkg' -prune -o \
    -path './node_modules' -prune -o \
    -type f \( \
      -name 'petiglyph-*.tar.gz' -o \
      -name 'petiglyph-*.zip' -o \
      -name 'petiglyph-*.tgz' -o \
      -name 'petiglyph-*.crate' -o \
      -name 'petiglyph-*.pkg.tar.zst' -o \
      -name 'petiglyph-debug-*.pkg.tar.zst' -o \
      -name '*.whl' \
    \) -print0
)

if ((${#unignored_artifacts[@]} > 0)); then
  report_failure "local package artifacts exist outside ignored paths"
  printf '%s\n' "${unignored_artifacts[@]}" >&2
fi

if ((failures != 0)); then
  exit 1
fi

echo "Release package hygiene checks passed"
