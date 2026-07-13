# Maintainer: WMDE <https://wmde.fun>
# Contributor: System76 <info@system76.com> (original cosmic-settings)
#
# Builds our fork Lin-WMDE/wmde-settings (branch wmde). WMDE settings app:
# installs /usr/bin/wmde-settings, the fun.wmde.Settings desktop/metainfo/polkit
# surfaces, per-page .desktop entries, appid icons, wmde-illustration-* status
# icons, and the fun.wmde.* default-schema tree under /usr/share/wmde/. It ships
# alongside cosmic-settings with no shared paths, so NO conflicts/replaces cosmic-*.
pkgname=wmde-settings
pkgver=1.0.12
pkgrel=4
pkgdesc="WMDE settings application (fork of cosmic-settings) - fun.wmde.Settings"
arch=('x86_64')
url="https://wmde.fun"
license=('GPL-3.0-only')
# Runtime: wayland client, xkbcommon, udev (display page), pipewire + libpulse
# (sound page audio client), fontconfig/expat (font handling). Verify with namcap.
depends=('glibc' 'gcc-libs' 'wayland' 'libxkbcommon' 'libinput' 'udev'
         'pipewire' 'libpulse' 'fontconfig' 'expat')
# Sibling WMDE forks consumed as local path/patch crates at build time; runtime
# integration (daemon, panel, comp) is provided by their own packages.
makedepends=('rust' 'cargo' 'just' 'git' 'clang' 'lld' 'pkgconf'
             'wmde-comp' 'wmde-panel' 'wmde-settings-daemon' 'wmde-bg')
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-settings.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  git describe --long --tags --abbrev=7 2>/dev/null | sed 's/^epoch-//;s/^v//;s/\([^-]*-g\)/r\1/;s/-/./g' ||
    printf '1.0.12.r%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
}

build() {
  cd "$srcdir/$pkgname"
  # x86-64-v3 (AVX2/BMI2) baseline for the WMDE repo; runs on Haswell+ (and the VM).
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu=x86-64-v3"
  just build-release
}

package() {
  cd "$srcdir/$pkgname"
  # installs /usr/bin/wmde-settings, fun.wmde.Settings[.<Page>].desktop entries,
  # the metainfo/polkit surfaces, appid + wmde-illustration icons, and the
  # fun.wmde.* default schemas under /usr/share/wmde/ (root matches libcosmic).
  just rootdir="$pkgdir" prefix=/usr install
  install -Dm644 LICENSE.md "$pkgdir/usr/share/licenses/$pkgname/LICENSE.md"
}
