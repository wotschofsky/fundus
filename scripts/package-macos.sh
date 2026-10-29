#!/bin/bash
# Local packages use build-macos.sh's signature. --release requires Developer ID and notarizes when
# NOTARY_PROFILE is set; without it the bundle runs on this Mac but Gatekeeper blocks it elsewhere.
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:---local}"
case "$mode" in
  --local) ;;
  --release)
    : "${MACOS_SIGNING_IDENTITY:?Set a Developer ID Application signing identity}"
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
app="$work/Fundus.app"
ditto "build/Fundus.app" "$app"
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
  if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    ditto -c -k --keepParent "$app" "$work/notary.zip"
    notarize "$work/notary.zip" "$work/app-notary.json"
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    spctl --assess --type execute "$app"
  fi
fi
codesign --verify --deep --strict "$app"

staging="$work/dmg-root"
mkdir "$staging"
ditto "$app" "$staging/Fundus.app"
ln -s /Applications "$staging/Applications"
printf '%s\n' 'Drag Fundus to Applications. Open Settings, enter your vault address and sign in with your browser.' > "$staging/Read Me.txt"
ditto -c -k --keepParent "$app" "$work/Fundus-$suffix.zip"
hdiutil create -volname 'Fundus' -srcfolder "$staging" -format UDZO "$work/Fundus-$suffix.dmg"
if [[ "$mode" == --release ]]; then
  codesign --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$work/Fundus-$suffix.dmg"
  if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    notarize "$work/Fundus-$suffix.dmg" "$work/dmg-notary.json"
    xcrun stapler staple "$work/Fundus-$suffix.dmg"
    xcrun stapler validate "$work/Fundus-$suffix.dmg"
  fi
  codesign --verify --strict "$work/Fundus-$suffix.dmg"
fi
hdiutil verify "$work/Fundus-$suffix.dmg"
cp "$work/Fundus-$suffix.zip" "$work/Fundus-$suffix.dmg" build/
(
  cd build
  shasum -a 256 "Fundus-$suffix.zip" "Fundus-$suffix.dmg" > "Fundus-$suffix.sha256"
  shasum -a 256 -c "Fundus-$suffix.sha256"
)
printf 'Packaged build/Fundus-%s.{zip,dmg,sha256}\n' "$suffix"
if [[ "$mode" == --local ]]; then
  echo 'Local test bundle only: not notarized for distribution.'
elif [[ -z "${NOTARY_PROFILE:-}" ]]; then
  echo 'Not notarized (NOTARY_PROFILE unset): runs on this Mac; Gatekeeper blocks it on other Macs.'
fi
