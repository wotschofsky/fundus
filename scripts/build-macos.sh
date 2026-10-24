#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' crates/server/Cargo.toml | head -n 1)"
build_number="${VAULT_BUILD_NUMBER:-1}"
case "$build_number" in ''|*[!0-9]*) echo 'VAULT_BUILD_NUMBER must be numeric' >&2; exit 1 ;; esac
swift build --package-path apps/macos -c release --scratch-path .cache/swift-macos
binary="$(swift build --package-path apps/macos -c release --scratch-path .cache/swift-macos --show-bin-path)/VaultUploader"
app=build/VaultUploader.app
mkdir -p "$app/Contents/MacOS"
cp "$binary" "$app/Contents/MacOS/VaultUploader"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>
<key>CFBundleName</key><string>Vault Uploader</string>
<key>CFBundleIdentifier</key><string>dev.wsky.vaultuploader.macos</string>
<key>CFBundleExecutable</key><string>VaultUploader</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
</dict></plist>
PLIST
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $build_number" "$app/Contents/Info.plist"
codesign --force --sign - "$app"
printf '%s\n' "$app"
