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

required_bins=(
  npm/petiglyph-linux-x64-gnu/bin/petiglyph
  npm/petiglyph-linux-arm64-gnu/bin/petiglyph
  npm/petiglyph-linux-x64-musl/bin/petiglyph
  npm/petiglyph-linux-arm64-musl/bin/petiglyph
  npm/petiglyph-darwin-x64/bin/petiglyph
  npm/petiglyph-darwin-arm64/bin/petiglyph
  npm/petiglyph-win32-x64-msvc/bin/petiglyph.exe
  npm/petiglyph-win32-arm64-msvc/bin/petiglyph.exe
)

for bin_path in "${required_bins[@]}"; do
  if [[ ! -f "$bin_path" ]]; then
    echo "Missing staged binary: $bin_path" >&2
    exit 1
  fi
done

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
platform_tgz="$(ls "$tarball_dir"/petiglyph-petiglyph-linux-x64-gnu-*.tgz 2>/dev/null | head -n1 || true)"

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
