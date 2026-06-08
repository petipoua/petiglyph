#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

output="npm/petiglyph/README.md"
temporary="$(mktemp)"
trap 'rm -f "$temporary"' EXIT

{
  cat npm/petiglyph-README-intro.md
  printf '\n'
  cat README.md
} > "$temporary"

if [[ "${1:-}" == "--check" ]]; then
  if ! cmp -s "$temporary" "$output"; then
    echo "npm/petiglyph/README.md is stale; run ./scripts/release_sync_readmes.sh" >&2
    exit 1
  fi
  echo "Package README sync check passed"
  exit 0
fi

if [[ $# -gt 0 ]]; then
  echo "Usage: ./scripts/release_sync_readmes.sh [--check]" >&2
  exit 1
fi

cp "$temporary" "$output"
echo "Synchronized npm package README from README.md"
