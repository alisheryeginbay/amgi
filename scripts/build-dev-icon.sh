#!/usr/bin/env bash
# Write app/Sources/Amgi/AmgiDev.icns, the Dock icon of debug builds (Amgi Dev): the
# Liquid Glass app icon, app/Amgi.icon, with violet in place of the blue and navy.
# Needs Xcode's actool. Run it again after changing app/Amgi.icon.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Forget-me-not blue #5b8def and ink navy #1b2f5e, turned to the same lightness in violet.
cp -R "$root/app/Amgi.icon" "$work/Amgi.icon"
for file in "$work"/Amgi.icon/Assets/*.svg "$work/Amgi.icon/icon.json"; do
    sed -i '' \
        -e 's/#5b8def/#915bef/gI' \
        -e 's/#1b2f5e/#341b5e/gI' \
        -e 's/0\.10588,0\.18431,0\.36863/0.20222,0.10588,0.36863/' \
        "$file"
done

mkdir "$work/out"
if ! log="$(xcrun actool "$work/Amgi.icon" \
    --compile "$work/out" \
    --platform macosx --minimum-deployment-target 26.0 \
    --app-icon Amgi \
    --output-partial-info-plist "$work/info.plist" \
    --errors --warnings --output-format human-readable-text 2>&1)"; then
    echo "$log" >&2
    exit 1
fi
cp "$work/out/Amgi.icns" "$root/app/Sources/Amgi/AmgiDev.icns"
echo "Wrote app/Sources/Amgi/AmgiDev.icns"
