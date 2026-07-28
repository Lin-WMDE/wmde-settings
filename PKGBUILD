# Maintainer: WMDE <https://wmde.fun>
# Contributor: System76 <info@system76.com> (original cosmic-settings)
#
# Builds our fork Lin-WMDE/wmde-settings (branch wmde). WMDE settings app:
# installs /usr/bin/wmde-settings, the fun.wmde.Settings desktop/metainfo/polkit
# surfaces, per-page .desktop entries, appid icons, wmde-illustration-* status
# icons, and the fun.wmde.* default-schema tree under /usr/share/wmde/. It ships
# alongside cosmic-settings with no shared paths, so nothing here relates to cosmic-*.
#
# It absorbed wmde-sysinfo (the standalone System Information app): its collection layer
# is now the wmde-hardware crate and its content is the Hardware sub-page under System &
# accounts. Hence replaces/conflicts below.
pkgname=wmde-settings
pkgver=1.0.12
pkgrel=5
pkgdesc="WMDE settings application (fork of cosmic-settings) - fun.wmde.Settings"
arch=('x86_64')
url="https://wmde.fun"
license=('GPL-3.0-only')
# replaces is what makes `pacman -Syu` actually REMOVE an installed wmde-sysinfo; dropping
# the package from the repo alone would leave it on the machine forever as a foreign one.
# conflicts covers the paths that bypass -Syu (pacman -U, a fresh pacman -S).
# provides is deliberately absent: this package installs no /usr/bin/wmde-sysinfo, so
# claiming the name would be a lie and would keep `pacman -S wmde-sysinfo` alive for good.
replaces=('wmde-sysinfo')
conflicts=('wmde-sysinfo')
# Runtime: wayland client, xkbcommon, udev (display page), pipewire + libpulse
# (sound page audio client), fontconfig/expat (font handling). Verify with namcap.
# hwdata came in with the Hardware page: without /usr/share/hwdata/{pci,usb,pnp}.ids the
# PCI, USB and display rows can only show raw hex ids. pciutils is deliberately NOT here -
# the point of absorbing wmde-sysinfo was to drop the two `lspci -nn` subprocesses.
# hicolor-icon-theme: the package installs into /usr/share/icons/hicolor and namcap
# errors without it. It arrives transitively through wmde-icons, but a standalone
# `pacman -S wmde-settings` should be correct on its own.
depends=('glibc' 'gcc-libs' 'wayland' 'libxkbcommon' 'libinput' 'udev'
         'pipewire' 'libpulse' 'fontconfig' 'expat' 'dav1d' 'hwdata'
         'hicolor-icon-theme')
# Sibling WMDE forks consumed as local path/patch crates at build time; runtime
# integration (daemon, panel, comp) is provided by their own packages.
makedepends=('rust' 'cargo' 'just' 'git' 'clang' 'lld' 'pkgconf' 'dav1d'
             'wmde-comp' 'wmde-panel' 'wmde-settings-daemon' 'wmde-bg')
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-settings.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  # WMDE unified version: 1.4 (libcosmic base) . <commits since nearest tag> . g<short>.
  local desc
  desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true)
  if [ -n "$desc" ]; then
    printf '1.4.%s.g%s' "$(printf '%s' "$desc" | sed -E 's/.*-([0-9]+)-g[0-9a-f]+$/\1/')" "$(git rev-parse --short=7 HEAD)"
  else
    printf '1.4.%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
  fi
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
