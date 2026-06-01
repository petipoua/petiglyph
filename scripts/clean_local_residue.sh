#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/clean_local_residue.sh [--apply]

Cleans common local ignored residue in this repository.

Default mode is dry-run (prints what would be removed).
Pass --apply to actually delete files/directories.
EOF
}

apply=0
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi
if [[ "${1:-}" == "--apply" ]]; then
  apply=1
elif [[ $# -gt 0 ]]; then
  echo "Unknown argument: $1" >&2
  usage >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

declare -a paths=(
  ".makepkg"
  "pkg"
  "target"
  "node_modules"
  ".venv"
  "venv"
  ".pytest_cache"
  ".mypy_cache"
  ".ruff_cache"
)

shopt -s nullglob
for file in petiglyph-*.tgz petiglyph-*.tar.gz petiglyph-*.pkg.tar.zst petiglyph-debug-*.pkg.tar.zst *.whl; do
  paths+=("$file")
done
for file in npm/petiglyph/*.tgz; do
  paths+=("$file")
done
for file in src/petiglyph-*.tar.gz; do
  paths+=("$file")
done
for dir in src/petiglyph; do
  [[ -e "$dir" ]] && paths+=("$dir")
done
shopt -u nullglob

while IFS= read -r cache_dir; do
  paths+=("$cache_dir")
done < <(find . -type d -name '__pycache__' -print)

declare -a existing=()
for path in "${paths[@]}"; do
  [[ -e "$path" ]] && existing+=("$path")
done

if [[ "${#existing[@]}" -eq 0 ]]; then
  echo "No local residue found."
  exit 0
fi

echo "Found ${#existing[@]} path(s):"
for path in "${existing[@]}"; do
  echo "  $path"
done

if [[ "$apply" -eq 0 ]]; then
  echo
  echo "Dry-run only. Re-run with --apply to delete these paths."
  exit 0
fi

for path in "${existing[@]}"; do
  rm -rf -- "$path"
done

echo "Removed ${#existing[@]} path(s)."
