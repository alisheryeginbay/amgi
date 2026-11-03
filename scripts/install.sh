#!/usr/bin/env bash
# Build Amgi.app and put it in /Applications, in place of the one there. If Amgi is
# open, it's quit first and opened again afterwards. Takes AMGI_SIGN_IDENTITY, as
# scripts/bundle.sh does.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bundle_id="io.github.alisheryeginbay.amgi"
dest="/Applications/Amgi.app"

"$root/scripts/bundle.sh"

# By bundle id, so Amgi Dev (which has none) is left running.
running="$(osascript -e "application id \"$bundle_id\" is running" 2>/dev/null || echo false)"
if [[ "$running" == "true" ]]; then
    osascript -e "quit app id \"$bundle_id\""
    while [[ "$(osascript -e "application id \"$bundle_id\" is running")" == "true" ]]; do
        sleep 0.2
    done
fi

rm -rf "$dest"
ditto "$root/target/bundle/Amgi.app" "$dest"
echo "Installed $dest."

if [[ "$running" == "true" ]]; then
    open "$dest"
fi
