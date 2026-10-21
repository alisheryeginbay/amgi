#!/usr/bin/env bash
# Write target/bundle/appcast.xml, the feed Amgi.app checks for updates. Its SUFeedURL,
# https://amgi.study/appcast.xml, redirects to the copy attached to the latest release;
# copies from 0.4.0 and earlier read that copy directly. Run after scripts/notarize.sh: it
# signs Amgi-<version>.zip with the Sparkle EdDSA key, taken from
# SPARKLE_ED_PRIVATE_KEY if set (what the release workflow uses), otherwise from the
# login keychain, where Sparkle's generate_keys saved it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/bundle"
app="$out/Amgi.app"
repo="https://github.com/alisheryeginbay/amgi"

version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")"
build="$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$app/Contents/Info.plist")"
minimum="$(/usr/libexec/PlistBuddy -c "Print :LSMinimumSystemVersion" "$app/Contents/Info.plist")"
zip="$out/Amgi-$version.zip"

# Sparkle's command-line tools aren't in its Swift package; fetch the release the app
# was built with.
sparkle_version="$(sed -n '/"sparkle"/,/"version"/s/.*"version" : "\(.*\)".*/\1/p' "$root/app/Package.resolved")"
tools="$root/target/sparkle-$sparkle_version"
if [[ ! -x "$tools/bin/sign_update" ]]; then
    mkdir -p "$tools"
    curl -fsSL "https://github.com/sparkle-project/Sparkle/releases/download/$sparkle_version/Sparkle-$sparkle_version.tar.xz" |
        tar -xJf - -C "$tools"
fi

if [[ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ]]; then
    signature="$(printf '%s\n' "$SPARKLE_ED_PRIVATE_KEY" | "$tools/bin/sign_update" --ed-key-file - "$zip")"
else
    signature="$("$tools/bin/sign_update" "$zip")"
fi

# One item, the newest version: Sparkle only needs that to offer the update. The full
# notes open from the dialog's Version History button.
cat > "$out/appcast.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
    <channel>
        <title>Amgi</title>
        <link>$repo</link>
        <item>
            <title>Amgi $version</title>
            <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
            <sparkle:version>$build</sparkle:version>
            <sparkle:shortVersionString>$version</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>$minimum</sparkle:minimumSystemVersion>
            <sparkle:fullReleaseNotesLink>$repo/releases/tag/v$version</sparkle:fullReleaseNotesLink>
            <enclosure url="$repo/releases/download/v$version/Amgi-$version.zip" type="application/octet-stream" $signature/>
        </item>
    </channel>
</rss>
EOF
echo "Wrote $out/appcast.xml for Amgi $version (build $build)."
