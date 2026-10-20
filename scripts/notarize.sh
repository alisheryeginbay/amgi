#!/usr/bin/env bash
# Notarize target/bundle/Amgi.app (built with a Developer ID by scripts/bundle.sh) and
# staple the ticket to it, then package it twice: a zip, and a disk image that is
# signed, notarized and stapled in turn.
#   NOTARY_PROFILE=amgi scripts/notarize.sh
#       credentials saved with `xcrun notarytool store-credentials amgi`
#   NOTARY_KEY=key.p8 NOTARY_KEY_ID=… NOTARY_ISSUER_ID=… scripts/notarize.sh
#       an App Store Connect API key (what the release workflow uses)
# AMGI_SIGN_IDENTITY signs the disk image (default: "Developer ID Application").
# Writes target/bundle/Amgi-<version>.zip and Amgi-<version>.dmg.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/bundle"
app="$out/Amgi.app"

if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    auth=(--keychain-profile "$NOTARY_PROFILE")
else
    auth=(--key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
fi
notarize() { xcrun notarytool submit "$1" "${auth[@]}" --wait --timeout 30m; }

version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")"
zip="$out/Amgi-$version.zip"
dmg="$out/Amgi-$version.dmg"

# notarytool takes a zip; the stapled app is zipped again for the release.
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
notarize "$zip"
xcrun stapler staple "$app"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
spctl --assess --type execute --verbose "$app"

# The disk image holds the stapled app, so the app opens offline once it's copied out.
AMGI_SIGN_IDENTITY="${AMGI_SIGN_IDENTITY:-Developer ID Application}" "$root/scripts/dmg.sh" "$app"
notarize "$dmg"
xcrun stapler staple "$dmg"
spctl --assess --type open --context context:primary-signature --verbose "$dmg"

echo "Notarized $app; wrote $zip and $dmg."
