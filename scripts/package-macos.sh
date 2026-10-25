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
app="$work/Transcript Vault.app"
ditto "build/Transcript Vault.app" "$app"
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
ditto "$app" "$staging/Transcript Vault.app"
ln -s /Applications "$staging/Applications"
printf '%s\n' 'Drag Transcript Vault to Applications. Open Settings, enter your vault address and sign in with your browser.' > "$staging/Read Me.txt"
ditto -c -k --keepParent "$app" "$work/TranscriptVault-$suffix.zip"
hdiutil create -volname 'Transcript Vault' -srcfolder "$staging" -format UDZO "$work/TranscriptVault-$suffix.dmg"
if [[ "$mode" == --release ]]; then
  codesign --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$work/TranscriptVault-$suffix.dmg"
  notarize "$work/TranscriptVault-$suffix.dmg" "$work/dmg-notary.json"
  xcrun stapler staple "$work/TranscriptVault-$suffix.dmg"
  xcrun stapler validate "$work/TranscriptVault-$suffix.dmg"
  codesign --verify --strict "$work/TranscriptVault-$suffix.dmg"
fi
hdiutil verify "$work/TranscriptVault-$suffix.dmg"
cp "$work/TranscriptVault-$suffix.zip" "$work/TranscriptVault-$suffix.dmg" build/
(
  cd build
  shasum -a 256 "TranscriptVault-$suffix.zip" "TranscriptVault-$suffix.dmg" > "TranscriptVault-$suffix.sha256"
  shasum -a 256 -c "TranscriptVault-$suffix.sha256"
)
printf 'Packaged build/TranscriptVault-%s.{zip,dmg,sha256}\n' "$suffix"
if [[ "$mode" == --local ]]; then
  echo 'Local test bundle only: ad hoc signed, not notarized for distribution.'
fi
