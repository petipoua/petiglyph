#!/usr/bin/env bash
# Release version sync helper.
# Keeps Cargo/PKGBUILD/.SRCINFO/npm versions aligned so multi-channel releases
# do not drift due to partial manual edits.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/release_sync_versions.sh [VERSION]

Behavior:
  - If VERSION is provided, update Cargo.toml package.version first.
  - Sync PKGBUILD pkgver and npm package versions to Cargo.toml version.
  - Regenerate .SRCINFO from PKGBUILD.
  - Sync optionalDependencies pins in npm/petiglyph/package.json.
  - Sync README JSON envelope sample version.
  - Sync the npm meta-package README from the repository README.
USAGE
}

if [[ "${1:-}" =~ ^(-h|--help|help)$ ]]; then
  usage
  exit 0
fi

if [[ $# -gt 1 ]]; then
  usage >&2
  exit 1
fi

new_version="${1:-}"
if [[ -n "$new_version" && ! "$new_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid version: $new_version" >&2
  exit 1
fi

if [[ -n "$new_version" ]]; then
  NEW_VERSION="$new_version" perl -i -pe 's/^(version\s*=\s*")[^"]+(")/$1$ENV{NEW_VERSION}$2/' Cargo.toml
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
if [[ -z "$version" ]]; then
  echo "Could not read version from Cargo.toml" >&2
  exit 1
fi

VERSION="$version" perl -i -pe 's/^pkgver=.*/pkgver=$ENV{VERSION}/m' PKGBUILD
VERSION="$version" perl -i -pe \
  's#(archive/refs/tags/v)[^/"]+(\.tar\.gz)#$1$ENV{VERSION}$2#g' \
  PKGBUILD
perl -i -pe "s/^sha256sums=.*/sha256sums=('SKIP')/" PKGBUILD

if command -v makepkg >/dev/null 2>&1; then
  makepkg --printsrcinfo > .SRCINFO
else
  echo "Skipping .SRCINFO regeneration because makepkg is not available." >&2
fi

for pkg in npm/*/package.json; do
  VERSION="$version" perl -i -pe 's/("version"\s*:\s*")[^"]+(",)/$1$ENV{VERSION}$2/' "$pkg"
done

VERSION="$version" perl -0i -pe 's/("petiglyph-[^"]+"\s*:\s*")[^"]+(")/$1$ENV{VERSION}$2/g' npm/petiglyph/package.json

VERSION="$version" perl -0i -pe 's/("version":\s*")[^"]+(",\s*\n\s*"data")/$1$ENV{VERSION}$2/' README.md

./scripts/release_sync_readmes.sh

echo "Synchronized release versions to $version"
