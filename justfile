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
macos-release-dir := 'build/macos/Build/Products/Release'
macos-app := macos-release-dir + '/' + name + '.app'

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

# Release macOS .app bundle (arm64 only).
build-macos: build-icon
    FLUTTER_XCODE_ARCHS=arm64 flutter build macos --verbose

# Release universal macOS .app bundle (x86_64 + arm64).
build-macos-uni: build-icon
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

# Package the arm64 macOS .app into a drag-to-Applications DMG.
package-macos: build-macos
    just _package-macos "{{macos-app}}" "arm64"

# Package the universal macOS .app into a drag-to-Applications DMG.
package-macos-uni: build-macos-uni
    just _package-macos "{{macos-app}}" "universal"

# Shared DMG packaging step. `app` is the .app bundle to package; `tag` is the
# architecture label embedded in the DMG filename.
_package-macos app tag:
    #!/usr/bin/env bash
    set -euo pipefail

    app="{{app}}"
    tag="{{tag}}"
    dmg_dir="{{macos-release-dir}}"
    icns="$app/Contents/Resources/AppIcon.icns"
    version=$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist" 2>/dev/null || echo "0.0.0")
    dmg="$dmg_dir/{{name}}-${version}-macos-${tag}.dmg"
    mkdir -p "$dmg_dir"

    if ! command -v hdiutil >/dev/null 2>&1; then
        echo "error: hdiutil not found; DMG packaging must run on macOS" >&2
        exit 1
    fi

    tmpdir=$(mktemp -d)
    mount_dir="$tmpdir/mount"
    mkdir -p "$mount_dir"
    mounted=false
    cleanup() {
        if [ "$mounted" = true ]; then
            hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
        fi
        rm -rf "$tmpdir"
    }
    trap cleanup EXIT

    size_mb=$(du -sk "$app" | cut -f1)
    size_mb=$(( size_mb / 1024 + 64 ))
    if [ "$size_mb" -lt 64 ]; then
        size_mb=64
    fi

    rw_dmg="$tmpdir/installer-rw.dmg"
    hdiutil create -size "${size_mb}m" -fs HFS+ -volname "{{name}}" "$rw_dmg" >/dev/null
    hdiutil attach "$rw_dmg" -readwrite -noverify -noautoopen -nobrowse -mountpoint "$mount_dir" >/dev/null
    mounted=true

    cp -R "$app" "$mount_dir/{{name}}.app"
    ln -s /Applications "$mount_dir/Applications"
    sync

    hdiutil detach "$mount_dir" >/dev/null
    mounted=false

    rm -f "$dmg"
    hdiutil convert "$rw_dmg" -format UDZO -imagekey zlib-level=9 -o "$dmg" >/dev/null

    if [ -f "$icns" ]; then
        icon_copy="$tmpdir/icon.icns"
        cp "$icns" "$icon_copy"
        sips -i "$icon_copy" >/dev/null
        if command -v derez >/dev/null 2>&1 && command -v rez >/dev/null 2>&1 && command -v setfile >/dev/null 2>&1; then
            derez -only icns "$icon_copy" > "$tmpdir/icon.rsrc"
            rez -append "$tmpdir/icon.rsrc" -o "$dmg"
            setfile -a C "$dmg"
        elif command -v xcrun >/dev/null 2>&1; then
            xcrun derez -only icns "$icon_copy" > "$tmpdir/icon.rsrc"
            xcrun rez -append "$tmpdir/icon.rsrc" -o "$dmg"
            xcrun setfile -a C "$dmg"
        else
            echo "warning: DeRez, Rez, or SetFile not found; DMG file icon was not set." >&2
        fi
    else
        echo "warning: $icns not found; DMG file icon was not set." >&2
    fi

    echo "Created $dmg"
