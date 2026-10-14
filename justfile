name := 'wmde-settings'
appid := 'fun.wmde.Settings'
# Чтобы shell() работал и на Windows с Git Bash; на Linux игнорируется
set windows-shell := ["C:\\Program Files\\Git\\bin\\sh.exe", "-cu"]
rootdir := ''
prefix := '/usr'

usrdir := absolute_path(clean(rootdir / prefix))
appdir := usrdir / 'share' / 'applications'
default-schema-target := usrdir / 'share' / 'wmde'
iconsdir := usrdir / 'share' / 'icons' / 'hicolor'

# Installation target paths
bin-dst := usrdir / 'bin' / name
metainfo-dst := usrdir / 'share' / 'metainfo' / appid + '.metainfo.xml'
polkit-actions-dst := usrdir / 'share' / 'polkit-1' / 'actions'
policy-users-dst := polkit-actions-dst / appid + '.Users.policy'
polkit-rules-dst := usrdir / 'share' / 'polkit-1' / 'rules.d' / 'wmde-settings.rules'

import 'cargo.just'

# Build recipes
[private]
default: build-release

# Install everything
install:
    install -Dm0644 {{'resources' / appid + '.metainfo.xml'}} {{metainfo-dst}}
    install -Dm0644 {{'resources' / 'polkit-1' / 'rules.d' / 'wmde-settings.rules'}} {{polkit-rules-dst}}
    install -Dm0644 {{'resources' / 'polkit-1' / 'actions' / appid + '.Users.policy'}} {{policy-users-dst}}
    install -Dm0755 {{cargo-target-dir / 'release' / name}} {{bin-dst}}
    cd target/xdgen && find * -type f -exec install -Dm0644 '{}' '{{appdir}}/{}' \;
    cd resources/default_schema && find * -type f -exec install -Dm0644 '{}' '{{default-schema-target}}/{}' \;
    cd resources/icons && find * -type f -exec install -Dm0644 '{}' '{{iconsdir}}/{}' \;

# Uninstalls everything (requires same arguments as given to install)
uninstall:
    rm {{bin-dst}} {{metainfo-dst}} {{polkit-rules-dst}} {{policy-users-dst}}
    cd target/xdgen && find * -type f -exec rm '{{appdir}}/{}' \;
    cd resources/default_schema && find * -type f -exec rm '{{default-schema-target}}/{}' \;
    cd resources/icons && find * -type f -exec rm '{{iconsdir}}/{}' \;

heaptrack *args:
    #!/usr/bin/env bash
    set -ex
    rm -fv heaptrack.wmde-settings.*
    cargo heaptrack --profile release-with-debug --bin wmde-settings -- {{args}}
    zstd -dc < heaptrack.wmde-settings.*.raw.zst + /usr/lib/heaptrack/libexec/heaptrack_env | zstd -c > heaptrack_env.wmde-settings.zst
    heaptrack_gui heaptrack.wmde-settings.zst

check-features:
    #!/usr/bin/env bash
    set -ex
    cargo check
    for service_manager in \
        "systemd" \
        "openrc"
    do
        cargo check --no-default-features --features "${service_manager}"
        for feature in \
            "page-accessibility" \
            "page-about" \
            "page-bluetooth" \
            "page-date" \
            "page-default-apps" \
            "page-display" \
            "page-input" \
            "page-legacy-applications" \
            "page-networking" \
            "page-power" \
            "page-region" \
            "page-sound" \
            "page-users" \
            "page-window-management" \
            "page-workspaces"
        do
            cargo check --no-default-features --features "${feature},${service_manager}"
        done
    done

# Bump cargo version, create git commit, and create tag.
# WMDE note: package versions come from git describe in the PKGBUILD, so this is only
# for the crate version; the debian changelog step was dropped with the debian/ tree.
tag version:
    find -type f -name Cargo.toml -exec sed -i '0,/^version/s/^version.*/version = "{{version}}"/' '{}' \; -exec git add '{}' \;
    cargo check
    cargo clean
    git add Cargo.lock
    git commit -m 'release: {{version}}'
    git tag -a {{version}} -m ''

# Версия пакета как pkgver() в PKGBUILD: 1.10.0.<commits>.g<sha>.
# Теги epoch-* дают только счётчик через git describe, fallback — rev-list.
wmde-version := shell('desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true); if [ -n "$desc" ]; then printf "1.10.0.%s.g%s" "$(printf "%s" "$desc" | sed -E "s/.*-([0-9]+)-g[0-9a-f]+$/\\1/")" "$(git rev-parse --short=7 HEAD)"; else printf "1.10.0.%s.g%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"; fi')
# CPU-baseline: пусто = чистый x86-64 (для deb/rpm по умолчанию), для v3: WMDE_CPU=x86-64-v3
wmde-cpu := env('WMDE_CPU', '')

# Раскладывает собранное в target/staging (тот же набор, что PKGBUILD кладёт в $pkgdir).
# ZSTD_SYS_USE_PKG_CONFIG здесь не нужен (нет zstd-зависимости, в отличие от files/term/portal).
package-staging staging="target/staging":
    #!/usr/bin/env bash
    set -e
    rm -rf {{staging}}
    if [ -n "{{wmde-cpu}}" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu={{wmde-cpu}}"; fi
    just build-release
    just rootdir="{{staging}}" prefix=/usr install
    # Categories=WMDE в .desktop — незарегистрированная категория.
    # OnlyShowIn=WMDE НЕ трогаем: это рабочая механика скрытия страниц от чужих DE.
    # Правим только Categories и только в стейджинге, сырцы не трогаем.
    sed -i 's/^Categories=WMDE$/Categories=X-WMDE;/' "{{staging}}/usr/share/applications/"*.desktop
    # makepkg на Arch стрипает бинарники сам, в deb/rpm это делаем руками
    strip --strip-unneeded "{{staging}}/usr/bin/"* 2>/dev/null || true
    just package-doc "{{staging}}"

# doc для deb/rpm: копирайт + changelog.yaml (nFPM сам разложит changelog
# в changelog.Debian.gz для deb и в %changelog для rpm)
package-doc staging="target/staging":
    #!/usr/bin/env bash
    set -e
    mkdir -p "{{staging}}/usr/share/doc/{{name}}"
    cp packaging/copyright "{{staging}}/usr/share/doc/{{name}}/copyright"
    chmod 644 "{{staging}}/usr/share/doc/{{name}}/copyright"
    printf -- '- semver: %s\n  date: %s\n  packager: WMDE <wmde@wmde.fun>\n  urgency: low\n  distribution: unstable\n  deb:\n    urgency: low\n    distributions:\n      - unstable\n  changes:\n    - note: Automated build via nFPM (see INSTALL.deb-rpm.md).\n' "{{wmde-version}}-1" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > target/changelog.yaml

# Стейджинг из вендора: сборка без сети к git-зависимостям.
# vendor.tar генерим один раз при наличии сети (`just vendor`), сам тарболл
# в git не коммитится (см. .gitignore), на офлайн-машину его привозим файлом.
# Соседние чек-ауты нужны и тут: path-зависимости вендором не покрываются.
package-staging-vendored staging="target/staging":
    #!/usr/bin/env bash
    set -e
    rm -rf {{staging}}
    if [ ! -f vendor.tar ]; then just vendor; fi
    if [ -n "{{wmde-cpu}}" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu={{wmde-cpu}}"; fi
    just build-vendored # сам распакует vendor.tar и соберёт --frozen --offline
    just rootdir="{{staging}}" prefix=/usr install
    sed -i 's/^Categories=WMDE$/Categories=X-WMDE;/' "{{staging}}/usr/share/applications/"*.desktop
    strip --strip-unneeded "{{staging}}/usr/bin/"* 2>/dev/null || true
    just package-doc "{{staging}}"
    rm -rf vendor .cargo

# Собирает .deb через nFPM (нужен nfpm в PATH)
deb:
    #!/usr/bin/env bash
    set -e
    just package-staging
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager deb --target target/

# Собирает .rpm через nFPM (нужен nfpm в PATH)
rpm:
    #!/usr/bin/env bash
    set -e
    just package-staging
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager rpm --target target/

# .deb из вендора (офлайн-сборка)
deb-vendored:
    #!/usr/bin/env bash
    set -e
    just package-staging-vendored
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager deb --target target/

# .rpm из вендора (офлайн-сборка)
rpm-vendored:
    #!/usr/bin/env bash
    set -e
    just package-staging-vendored
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager rpm --target target/
