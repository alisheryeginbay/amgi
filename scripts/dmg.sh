#!/usr/bin/env bash
# Put Amgi.app in a disk image beside an Applications shortcut, over the drawn
# background (media/dmg-background.tiff), and sign it if AMGI_SIGN_IDENTITY is set.
#   scripts/dmg.sh [path/to/Amgi.app]     (default: target/bundle/Amgi.app)
# Writes Amgi-<version>.dmg next to the app. Needs uv, which runs dmgbuild.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="$(cd "${1:-$root/target/bundle/Amgi.app}" && pwd)"
identity="${AMGI_SIGN_IDENTITY:-}"

version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")"
dmg="$(dirname "$app")/Amgi-$version.dmg"

rm -f "$dmg"
uvx --from dmgbuild==1.6.7 dmgbuild \
    -s "$root/scripts/dmg-settings.py" \
    -D app="$app" \
    -D background="$root/media/dmg-background.tiff" \
    "Amgi" "$dmg"

if [[ -n "$identity" ]]; then
    codesign --sign "$identity" --timestamp "$dmg"
    codesign --verify --strict "$dmg"
fi
echo "Built $dmg."
