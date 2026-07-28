# WMDE Settings

The settings application for the [WMDE desktop](https://wmde.fun).

WMDE Settings is a fork of [pop-os/cosmic-settings][cosmic-settings] by System76, rebranded for
WMDE. Original authorship and copyright are retained. The crate directory keeps its upstream
`cosmic-settings` name so that syncing with upstream stays cheap; the installed binary is
`wmde-settings`.

## Translators

Translations are inherited from upstream, which takes them through Weblate at
https://hosted.weblate.org/projects/pop-os/cosmic-settings. Fork-specific strings are edited in
`i18n/` directly.

## Distributors

We will accept pull requests for distro-specific features and pages.
Make them compile conditionally with a [cargo feature][cargo-feature].

The accent palettes on the Appearance settings page are configurable through the cosmic-config directory at `/usr/share/wmde/fun.wmde.Settings/v1/`. One at `accent_palette_dark`, and another at `accent_palette_light`. Examples can be found at [resources/accent_palette_dark.ron](./resources/accent_palette_dark.ron) and [resources/accent_palette_light.ron](./resources/accent_palette_light.ron). This can be copied locally to `~/.config/wmde/fun.wmde.Settings/v1/` for testing, and then move to `/usr/share/wmde` for packaging.

## Build

### Dependencies

See `makedepends` in the [PKGBUILD](./PKGBUILD), and `forks/Dockerfile.build` in the
WMDE project for the container the packages are actually built in.

### Install

WMDE uses [just][just] as its preferred build tool.

```sh
just
sudo just install
```

### Packaging

If packaging for a Linux distribution, vendor dependencies locally with the `vendor` rule, and build with the vendored sources using the `build-vendored` rule. When installing files, use the `rootdir` and `prefix` variables to change installation paths.

```sh
just vendor
just build-vendored
just rootdir="$pkgdir" prefix=/usr install
```

WMDE packages for Arch: see [PKGBUILD](./PKGBUILD). The whole stack is built through
`tools/build-packages.sh` in the WMDE project, inside the `wmde-build-full` container.

## Developers

Developers should install [rustup][rustup] and configure their editor to use [rust-analyzer][rust-analyzer]. Run `just check` to ensure that the changes you make are free of linter warnings. You may configure your editor to run `just check-json` as the rust-analyzer check command.

Run the wmde-settings binary with `just run` so that logs will be emitted to stderr, and crashes will generate detailed backtraces. Applications shouldn't crash, so when writing code, avoid use of `unwrap()` and `expect()`. Instead, log errors with `tracing::error!()` or `tracing::warn!()`.

To improve compilation times, use Rust >= 1.90.0 and configure [sccache][sccache] for use with Rust.

## License

Licensed under the [GNU Public License 3.0](https://choosealicense.com/licenses/gpl-3.0).

### Contribution

Any contribution intentionally submitted for inclusion in the work by you shall be licensed under the GNU Public License 3.0 (GPL-3.0). Each source file should have a SPDX copyright notice at the top of the file:

```
// SPDX-License-Identifier: GPL-3.0-only
```

[cargo-feature]: https://doc.rust-lang.org/cargo/reference/features.html
[cosmic-settings]: https://github.com/pop-os/cosmic-settings
[just]: https://github.com/casey/just
[rustup]: https://rustup.rs/
[rust-analyzer]: https://rust-analyzer.github.io/
[sccache]: https://github.com/mozilla/sccache
