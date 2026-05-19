pkgname=petiglyph
pkgver=0.1.0
pkgrel=1
pkgdesc='Build icon fonts from project-local assets'
arch=('x86_64')
url='https://github.com/petipoua/petiglyph'
license=('MIT')
depends=('ffmpeg')
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$srcdir/petiglyph"
  cargo build --release --locked
}

package() {
  cd "$srcdir/petiglyph"
  install -Dm755 target/release/petiglyph "$pkgdir/usr/bin/petiglyph"
  install -Dm644 README.md "$pkgdir/usr/share/doc/petiglyph/README.md"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/petiglyph/LICENSE"
}
