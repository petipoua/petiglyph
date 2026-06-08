#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
source "$repo_root/scripts/lib/pkg_meta.sh"

version="${1:-}"
pkgrel="1"
aur_repo_dir="$repo_root/../${AUR_PKGNAME}-aur"
aur_repo_url="ssh://aur@aur.archlinux.org/${AUR_PKGNAME}.git"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pkgrel)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --pkgrel" >&2
        exit 1
      fi
      pkgrel="$2"
      shift 2
      ;;
    --repo-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --repo-dir" >&2
        exit 1
      fi
      aur_repo_dir="$2"
      shift 2
      ;;
    --help|-h)
      cat <<'EOF'
Usage:
  ./scripts/release_publish_aur.sh [X.Y.Z] [--pkgrel N] [--repo-dir PATH]

Defaults:
  X.Y.Z defaults to Cargo.toml version
  pkgrel defaults to 1
  repo dir defaults to ../petiglyph-aur
EOF
      exit 0
      ;;
    -*)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
    *)
      if [[ -n "${version}" && "$1" != "$version" ]]; then
        echo "Unexpected extra argument: $1" >&2
        exit 1
      fi
      version="$1"
      shift
      ;;
  esac
done

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

if [[ ! "$pkgrel" =~ ^[1-9][0-9]*$ ]]; then
  echo "Invalid pkgrel: $pkgrel" >&2
  exit 1
fi

"$repo_root/scripts/release_prepare_aur.sh" "$version" --pkgrel "$pkgrel"

if [[ -e "$aur_repo_dir" && ! -d "$aur_repo_dir/.git" ]]; then
  echo "AUR repo path exists but is not a git repository: $aur_repo_dir" >&2
  exit 1
fi

if [[ ! -d "$aur_repo_dir/.git" ]]; then
  git clone "$aur_repo_url" "$aur_repo_dir"
fi

cd "$aur_repo_dir"
if git rev-parse --verify HEAD >/dev/null 2>&1; then
  git pull --ff-only
fi

cp "$repo_root/PKGBUILD" PKGBUILD
cp "$repo_root/.SRCINFO" .SRCINFO
git add PKGBUILD .SRCINFO

if git diff --cached --quiet; then
  echo "No AUR packaging changes to publish for version $version-$pkgrel"
  exit 0
fi

if git rev-parse --verify HEAD >/dev/null 2>&1; then
  commit_message="Update to ${version}-${pkgrel}"
else
  commit_message="Initial import"
fi

git commit -m "$commit_message"
git push

echo "Published AUR packaging ${version}-${pkgrel} to $aur_repo_url from $aur_repo_dir"
