name := 'wmde-settings'
appid := 'fun.wmde.Settings'
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
