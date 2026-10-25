#!/bin/sh
# Builds build/Transcript Vault.app. Signs with MACOS_SIGNING_IDENTITY, else the first Developer ID
# Application or Apple Development identity in the keychain, else ad hoc. A stable identity matters:
# the Keychain ties saved vault connections to the signature, so every ad hoc build asks again.
set -eu
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' crates/server/Cargo.toml | head -n 1)"
build_number="${VAULT_BUILD_NUMBER:-1}"
case "$build_number" in ''|*[!0-9]*) echo 'VAULT_BUILD_NUMBER must be numeric' >&2; exit 1 ;; esac

# SwiftPM records the deployment target as the SDK version; macOS then runs the app in the
# pre-Liquid Glass compatibility mode. Link against the installed SDK explicitly.
sdk="$(xcrun --show-sdk-version)"
set -- -c release --scratch-path .cache/swift-macos -Xlinker -platform_version -Xlinker macos -Xlinker 15.0 -Xlinker "$sdk"
swift build --package-path apps/macos "$@"
binary="$(swift build --package-path apps/macos "$@" --show-bin-path)/TranscriptVault"

app="build/Transcript Vault.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/TranscriptVault"
xcrun actool apps/macos/Resources/AppIcon.icon --compile "$app/Contents/Resources" --platform macosx \
  --minimum-deployment-target 15.0 --app-icon AppIcon --output-partial-info-plist .cache/AppIcon.plist >/dev/null
# The bundle identifier predates the rename; changing it would orphan saved connections and settings.
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>
<key>CFBundleName</key><string>Transcript Vault</string>
<key>CFBundleDisplayName</key><string>Transcript Vault</string>
<key>CFBundleIdentifier</key><string>dev.wsky.vaultuploader.macos</string>
<key>CFBundleExecutable</key><string>TranscriptVault</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleIconName</key><string>AppIcon</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
</dict></plist>
PLIST
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $build_number" "$app/Contents/Info.plist"

identity="${MACOS_SIGNING_IDENTITY:-}"
if [ -z "$identity" ]; then
  identities="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  identity="$(printf '%s\n' "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$identity" ] || identity="$(printf '%s\n' "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -n 1)"
fi
if [ -n "$identity" ]; then
  codesign --force --options runtime --sign "$identity" "$app"
else
  echo 'No signing identity found; signing ad hoc. Each build will ask for Keychain access again.' >&2
  codesign --force --sign - "$app"
fi
printf '%s\n' "$app"
