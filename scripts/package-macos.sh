#!/bin/bash
# Local packages are ad hoc signed. --release requires Developer ID and notarization.
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:---local}"
case "$mode" in
  --local) ;;
  --release)
    : "${MACOS_SIGNING_IDENTITY:?Set a Developer ID Application signing identity}"
    : "${NOTARY_PROFILE:?Set a notarytool Keychain profile}"
    case "$MACOS_SIGNING_IDENTITY" in
      'Developer ID Application: '*) ;;
      *) echo 'A Developer ID Application identity is required.' >&2; exit 1 ;;
    esac
    ;;
  *) echo 'Usage: scripts/package-macos.sh [--local|--release]' >&2; exit 1 ;;
esac
scripts/build-macos.sh
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="$work/VaultUploader.app"
ditto build/VaultUploader.app "$app"
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
arch="$(uname -m)"
suffix="$version-$arch-local"

notarize() {
  local artifact="$1" report="$2" outcome
  xcrun notarytool submit "$artifact" --keychain-profile "$NOTARY_PROFILE" --wait --output-format json > "$report"
  outcome="$(/usr/bin/plutil -extract status raw -o - "$report")"
  if [[ "$outcome" != Accepted ]]; then
    cat "$report" >&2
    echo "Notarization failed: $outcome" >&2
    exit 1
  fi
}

if [[ "$mode" == --release ]]; then
  suffix="$version-$arch"
  codesign --force --options runtime --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$app"
  codesign --verify --deep --strict "$app"
  ditto -c -k --keepParent "$app" "$work/notary.zip"
  notarize "$work/notary.zip" "$work/app-notary.json"
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  spctl --assess --type execute "$app"
fi
codesign --verify --deep --strict "$app"

staging="$work/dmg-root"
mkdir "$staging"
ditto "$app" "$staging/VaultUploader.app"
ln -s /Applications "$staging/Applications"
printf '%s\n' 'Drag VaultUploader to Applications. Configure your server HTTPS URL and an upload token.' > "$staging/Read Me.txt"
ditto -c -k --keepParent "$app" "$work/VaultUploader-$suffix.zip"
hdiutil create -volname 'Vault Uploader' -srcfolder "$staging" -format UDZO "$work/VaultUploader-$suffix.dmg"
if [[ "$mode" == --release ]]; then
  codesign --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$work/VaultUploader-$suffix.dmg"
  notarize "$work/VaultUploader-$suffix.dmg" "$work/dmg-notary.json"
  xcrun stapler staple "$work/VaultUploader-$suffix.dmg"
  xcrun stapler validate "$work/VaultUploader-$suffix.dmg"
  codesign --verify --strict "$work/VaultUploader-$suffix.dmg"
fi
hdiutil verify "$work/VaultUploader-$suffix.dmg"
cp "$work/VaultUploader-$suffix.zip" "$work/VaultUploader-$suffix.dmg" build/
(
  cd build
  shasum -a 256 "VaultUploader-$suffix.zip" "VaultUploader-$suffix.dmg" > "VaultUploader-$suffix.sha256"
  shasum -a 256 -c "VaultUploader-$suffix.sha256"
)
printf 'Packaged build/VaultUploader-%s.{zip,dmg,sha256}\n' "$suffix"
if [[ "$mode" == --local ]]; then
  echo 'Local test bundle only: ad hoc signed, not notarized for distribution.'
fi
