#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pkgname="petiglyph"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/aur.sh [step]

Steps:
  uninstall     Remove installed package, rebuild latest package, then install it (default)
  build         Regenerate PKGBUILD + source tarball, then rebuild package artifacts
  install       Install latest built package with pacman
  pkgbuild      Generate PKGBUILD from Cargo.toml metadata
  tarball       Create source tarball using git archive
  all           Alias for build
  all-install   Alias for uninstall

Defaults:
  step defaults to "uninstall"
EOF
}

read_pkgver() {
  sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -n1
}

read_repo_url() {
  sed -n 's/^repository = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -n1
}

write_pkgbuild() {
  local pkgver repo_url
  pkgver="$(read_pkgver)"
  repo_url="$(read_repo_url)"

  if [[ -z "${pkgver}" ]]; then
    echo "Could not read package version from Cargo.toml" >&2
    exit 1
  fi
  if [[ -z "${repo_url}" ]]; then
    repo_url="https://github.com/petipoua/petiglyph"
  fi

  cat >"$repo_root/PKGBUILD" <<EOF
pkgname=$pkgname
pkgver=$pkgver
pkgrel=1
pkgdesc='Build icon fonts from project-local assets'
arch=('x86_64')
url='$repo_url'
license=('MIT')
depends=()
makedepends=('cargo')
source=("\$pkgname-\$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "\$srcdir/petiglyph"
  cargo build --release --locked
}

package() {
  cd "\$srcdir/petiglyph"
  install -Dm755 target/release/petiglyph "\$pkgdir/usr/bin/petiglyph"
  install -Dm644 README.md "\$pkgdir/usr/share/doc/petiglyph/README.md"
}
EOF

  echo "Wrote $repo_root/PKGBUILD (pkgver=$pkgver)"
}

create_tarball() {
  local pkgver out
  pkgver="$(read_pkgver)"
  if [[ -z "${pkgver}" ]]; then
    echo "Could not read package version from Cargo.toml" >&2
    exit 1
  fi
  out="$repo_root/petiglyph-$pkgver.tar.gz"
  git -C "$repo_root" archive --format=tar.gz --prefix=petiglyph/ -o "$out" HEAD
  echo "Created $out"
}

build_package() {
  local build_root src_cache pkg_dest log_dest
  build_root="$repo_root/.makepkg/build"
  src_cache="$repo_root/.makepkg/src-cache"
  pkg_dest="$repo_root"
  log_dest="$repo_root/.makepkg/logs"

  cd "$repo_root"
  if [[ ! -f PKGBUILD ]]; then
    echo "PKGBUILD missing. Generating it first."
    write_pkgbuild
  fi

  mkdir -p "$build_root" "$src_cache" "$log_dest"
  BUILDDIR="$build_root" \
  SRCDEST="$src_cache" \
  PKGDEST="$pkg_dest" \
  LOGDEST="$log_dest" \
  makepkg --syncdeps --cleanbuild --force --noconfirm
}

build_latest_package() {
  write_pkgbuild
  create_tarball
  build_package
}

install_package() {
  local pkgver pkg_file
  pkgver="$(read_pkgver)"
  if [[ -z "${pkgver}" ]]; then
    echo "Could not read package version from Cargo.toml" >&2
    exit 1
  fi

  pkg_file="$(ls -1t "$repo_root"/petiglyph-"$pkgver"-*.pkg.tar.zst 2>/dev/null | grep -v 'petiglyph-debug' | head -n1 || true)"
  if [[ -z "${pkg_file}" ]]; then
    echo "No built package found for version $pkgver. Run ./scripts/aur.sh build first." >&2
    exit 1
  fi

  echo "Installing $pkg_file"
  sudo pacman -U --needed --noconfirm "$pkg_file"
}

uninstall_and_reinstall_package() {
  if pacman -Q "$pkgname" >/dev/null 2>&1; then
    echo "Removing installed package: $pkgname"
    sudo pacman -Rns --noconfirm "$pkgname"
  else
    echo "Package not currently installed: $pkgname"
  fi
  build_latest_package
  install_package
}

step="${1:-uninstall}"
if [[ $# -gt 0 ]]; then
  shift
fi

if [[ $# -gt 0 ]]; then
  echo "Unexpected arguments: $*" >&2
  usage
  exit 1
fi

case "$step" in
  pkgbuild)
    write_pkgbuild
    ;;
  tarball)
    create_tarball
    ;;
  build)
    build_latest_package
    ;;
  install)
    install_package
    ;;
  uninstall)
    uninstall_and_reinstall_package
    ;;
  all)
    build_latest_package
    ;;
  all-install)
    uninstall_and_reinstall_package
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    echo "Unknown step: $step" >&2
    usage
    exit 1
    ;;
esac
