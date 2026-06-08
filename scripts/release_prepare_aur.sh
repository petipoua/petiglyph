#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
source "$repo_root/scripts/lib/pkg_meta.sh"

version="${1:-}"
pkgrel="1"

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
    --help|-h)
      cat <<'EOF'
Usage:
  ./scripts/release_prepare_aur.sh [X.Y.Z] [--pkgrel N]

Defaults:
  X.Y.Z defaults to Cargo.toml version
  pkgrel defaults to 1
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

source_url="https://github.com/petipoua/petiglyph/archive/refs/tags/v${version}.tar.gz"
sha256="$(curl -fsSL "$source_url" | sha256sum | awk '{print $1}')"

cat > PKGBUILD <<PKGEOF
pkgname=$AUR_PKGNAME
pkgver=${version}
pkgrel=${pkgrel}
pkgdesc='$AUR_PKGDESC'
arch=($AUR_ARCH_LITERAL)
url='$AUR_DEFAULT_REPO_URL'
license=($AUR_LICENSE_LITERAL)
depends=($AUR_DEPENDS_LITERAL)
makedepends=($AUR_MAKEDEPENDS_LITERAL)
source=("\$pkgname-\$pkgver.tar.gz::${source_url}")
sha256sums=('${sha256}')

build() {
  cd "\$srcdir/petiglyph-\$pkgver"
  cargo build --release --locked
}

package() {
  cd "\$srcdir/petiglyph-\$pkgver"
  install -Dm755 target/release/petiglyph "\$pkgdir/usr/bin/petiglyph"
  install -Dm644 README.md "\$pkgdir/usr/share/doc/petiglyph/README.md"
  install -Dm644 LICENSE "\$pkgdir/usr/share/licenses/petiglyph/LICENSE"
}
PKGEOF

makepkg --printsrcinfo > .SRCINFO

echo "Prepared PKGBUILD/.SRCINFO for AUR release version ${version}-${pkgrel}"
echo "Source URL: ${source_url}"
echo "SHA256: ${sha256}"
