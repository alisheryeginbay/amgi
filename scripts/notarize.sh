#!/usr/bin/env bash
# Notarize target/bundle/Amgi.app (built with a Developer ID by scripts/bundle.sh),
# staple the ticket to it and zip it for release.
#   NOTARY_PROFILE=amgi scripts/notarize.sh
#       credentials saved with `xcrun notarytool store-credentials amgi`
#   NOTARY_KEY=key.p8 NOTARY_KEY_ID=… NOTARY_ISSUER_ID=… scripts/notarize.sh
#       an App Store Connect API key (what the release workflow uses)
# Writes target/bundle/Amgi-<version>.zip.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/bundle"
app="$out/Amgi.app"

if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    auth=(--keychain-profile "$NOTARY_PROFILE")
else
    auth=(--key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
fi

version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")"
zip="$out/Amgi-$version.zip"

# notarytool takes a zip; the stapled app is zipped again for the release.
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
xcrun notarytool submit "$zip" "${auth[@]}" --wait --timeout 30m
xcrun stapler staple "$app"

rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
spctl --assess --type execute --verbose "$app"
echo "Notarized $app; wrote $zip."
