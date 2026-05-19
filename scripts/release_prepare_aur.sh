#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="${1:-}"
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

source_url="https://github.com/petipoua/petiglyph/archive/refs/tags/v${version}.tar.gz"
sha256="$(curl -fsSL "$source_url" | sha256sum | awk '{print $1}')"

cat > PKGBUILD <<PKGEOF
pkgname=petiglyph
pkgver=${version}
pkgrel=1
pkgdesc='Build icon fonts from project-local assets'
arch=('x86_64')
url='https://github.com/petipoua/petiglyph'
license=('MIT')
depends=('ffmpeg')
makedepends=('cargo')
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

echo "Prepared PKGBUILD/.SRCINFO for AUR release version ${version}"
echo "Source URL: ${source_url}"
echo "SHA256: ${sha256}"
