#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
source "$repo_root/scripts/lib/pkg_meta.sh"

version="${1:-}"
aur_repo_dir="${2:-$repo_root/../${AUR_PKGNAME}-aur}"
aur_repo_url="ssh://aur@aur.archlinux.org/${AUR_PKGNAME}.git"

if [[ -z "$version" ]]; then
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
fi

if [[ -z "$version" ]]; then
  echo "Could not determine version" >&2
  exit 1
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid version: $version" >&2
  exit 1
fi

"$repo_root/scripts/release_prepare_aur.sh" "$version"

if [[ -e "$aur_repo_dir" && ! -d "$aur_repo_dir/.git" ]]; then
  echo "AUR repo path exists but is not a git repository: $aur_repo_dir" >&2
  exit 1
fi

if [[ ! -d "$aur_repo_dir/.git" ]]; then
  git clone "$aur_repo_url" "$aur_repo_dir"
fi

cp "$repo_root/PKGBUILD" "$aur_repo_dir/PKGBUILD"
cp "$repo_root/.SRCINFO" "$aur_repo_dir/.SRCINFO"

cd "$aur_repo_dir"

git add PKGBUILD .SRCINFO

if git diff --cached --quiet; then
  echo "No AUR packaging changes to publish for version $version"
  exit 0
fi

if git rev-parse --verify HEAD >/dev/null 2>&1; then
  commit_message="Update to ${version}"
else
  commit_message="Initial import"
fi

git commit -m "$commit_message"
git push

echo "Published AUR packaging to $aur_repo_url from $aur_repo_dir"
