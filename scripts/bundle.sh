#!/usr/bin/env bash
# Build Amgi.app: the Rust core, the web panes and the app in release mode, put
# together as a macOS app bundle and signed.
#   scripts/bundle.sh            ad-hoc signature, for this Mac
#   AMGI_SIGN_IDENTITY="Developer ID Application" scripts/bundle.sh
#                                signed for distribution (notarize separately)
# Writes target/bundle/Amgi.app.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

bundle_id="io.github.alisheryeginbay.amgi"
identity="${AMGI_SIGN_IDENTITY:--}"
out="$root/target/bundle"
app="$out/Amgi.app"

# Rust keeps source paths for panic messages, including the builder's home folder
# (`~/.cargo`, `~/.rustup`). Map them to neutral names. Swift's paths are only in
# debug info, which `strip` removes below.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
remap=(
    "$root=/amgi"
    "$cargo_home=/cargo"
    "$rustup_home=/rustup"
    "$HOME=/home"
)
rustflags=()
for map in "${remap[@]}"; do
    rustflags+=("--remap-path-prefix=$map")
done

# Its own target directory: different flags would otherwise rebuild everything in
# `target/release` on every switch.
CARGO_TARGET_DIR="$out/cargo" RUSTFLAGS="${rustflags[*]}" scripts/build-core.sh release
scripts/build-web.sh

swift build -c release --package-path app --scratch-path "$out/swift"
bin="$(swift build -c release --package-path app --scratch-path "$out/swift" --show-bin-path)"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
build="$(git rev-list --count HEAD)"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/Amgi" "$app/Contents/MacOS/Amgi"
# `Bundle.resources` (AmgiApp.swift) finds them here; SwiftPM's `Bundle.module` wouldn't.
cp -R "$bin/Amgi_Amgi.bundle" "$app/Contents/Resources/"
# Drop debug symbols, which also name the object files they came from.
strip -S -x "$app/Contents/MacOS/Amgi"

# Sparkle, the updater. Its XPC services are only for sandboxed apps, which Amgi isn't.
sparkle="$app/Contents/Frameworks/Sparkle.framework"
mkdir -p "$app/Contents/Frameworks"
ditto "$bin/Sparkle.framework" "$sparkle"
rm -rf "$sparkle/XPCServices" "$sparkle/Versions/B/XPCServices"

# The app icon, made in Icon Composer: layers macOS renders as Liquid Glass, in light,
# dark, clear and tinted. actool (Xcode's, not the Command Line Tools') compiles it into
# Assets.car, plus Amgi.icns for older systems. It needs absolute paths.
if ! icon_log="$(xcrun actool "$root/app/Amgi.icon" \
    --compile "$app/Contents/Resources" \
    --platform macosx --minimum-deployment-target 26.0 \
    --app-icon Amgi \
    --output-partial-info-plist "$out/icon-info.plist" \
    --errors --warnings --output-format human-readable-text 2>&1)"; then
    echo "$icon_log" >&2
    exit 1
fi

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>Amgi</string>
    <key>CFBundleIdentifier</key>
    <string>$bundle_id</string>
    <key>CFBundleName</key>
    <string>Amgi</string>
    <key>CFBundleDisplayName</key>
    <string>Amgi</string>
    <key>CFBundleIconName</key>
    <string>Amgi</string>
    <key>CFBundleIconFile</key>
    <string>Amgi</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$build</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>26.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.education</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>SUFeedURL</key>
    <string>https://github.com/alisheryeginbay/amgi/releases/latest/download/appcast.xml</string>
    <key>SUPublicEDKey</key>
    <string>TShzf8QQ88x/3EE1dfKeQ2QFi59r2r7/zXfZK808aXY=</string>
</dict>
</plist>
EOF

# Ad-hoc (`-`) runs on this Mac. A Developer ID signature also needs the hardened
# runtime and a secure timestamp to pass notarization.
sign=(--force --sign "$identity")
[[ "$identity" != "-" ]] && sign+=(--options runtime --timestamp)
# Inside out: Sparkle's helpers, the framework, then the app.
codesign "${sign[@]}" "$sparkle/Versions/B/Autoupdate"
codesign "${sign[@]}" "$sparkle/Versions/B/Updater.app"
codesign "${sign[@]}" "$sparkle"
codesign "${sign[@]}" "$app"
codesign --verify --strict --deep "$app"

# Check that no home folder path made it into the bundle.
if grep -rqa "$HOME" "$app"; then
    echo "warning: $app still contains $HOME:" >&2
    grep -rla "$HOME" "$app" >&2
fi
echo "Built $app ($version, build $build)."
