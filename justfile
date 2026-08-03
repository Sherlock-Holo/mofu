# Application binary name.
name := 'mofu'
# Reverse-DNS application ID (matches .desktop / metainfo / GTK).
appid := 'io.holo.mofu'

# Install prefix; override with `just rootdir=/` when needed.
rootdir := ''
prefix := '/usr'

desktop := 'resources/' + appid + '.desktop'
appdata := 'resources/' + appid + '.metainfo.xml'
linux-bundle := 'build/linux/x64/release/bundle/' + name
macos-app := 'build/macos/Build/Products/Release/' + name + '.app'

base-dir := absolute_path(clean(rootdir / prefix))
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / (appid + '.desktop')
appdata-dst := base-dir / 'share' / 'metainfo' / (appid + '.metainfo.xml')
icons-dst := base-dir / 'share' / 'icons' / 'hicolor'
applications-dst := '/Applications/' + name + '.app'

default: build-linux

# Generate Linux hicolor PNGs and macOS/Windows launcher icons from resources/logo.png.
build-icon:
    dart run tool/generate_icons.dart
    dart run flutter_launcher_icons

# Release Linux desktop bundle.
build-linux: build-icon
    flutter build linux --verbose

# Release macOS .app bundle.
build-macos: build-icon
    flutter build macos --verbose

# Install Linux binary, desktop entry, AppStream metadata, and icon theme files.
install-linux: build-linux
    #!/usr/bin/env bash
    set -euo pipefail
    install -Dm0755 "{{linux-bundle}}" "{{bin-dst}}"
    install -Dm0644 "{{desktop}}" "{{desktop-dst}}"
    install -Dm0644 "{{appdata}}" "{{appdata-dst}}"
    for size in 64 256 512; do
      install -Dm0644 \
        "resources/icons/hicolor/${size}x${size}/apps/{{appid}}.png" \
        "{{icons-dst}}/${size}x${size}/apps/{{appid}}.png"
    done

# Install macOS .app bundle into /Applications.
install-macos: build-macos
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf "{{applications-dst}}"
    cp -R "{{macos-app}}" "{{applications-dst}}"
