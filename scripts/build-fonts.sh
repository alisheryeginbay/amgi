#!/usr/bin/env bash
# Build the CJK font packs the app downloads when a collection first needs them:
# Noto Serif KR, JP, SC and TC (Source Han Serif) from web/node_modules, one zip each,
# holding fontsource's `wght.css`, the woff2 files it names and the font license.
#   scripts/build-fonts.sh
# Writes target/fonts/noto-serif-{kr,jp,sc,tc}.zip and prints their SHA-256 sums.
#
# To ship new packs: attach the zips to a new GitHub release (`fonts-2`, …), then put
# its tag and the sums in `FontPacks.swift`. Never replace the files of a release the
# app already points to; installed copies are named by their sum.
#   gh release create fonts-2 target/fonts/*.zip --latest=false --title "CJK fonts" --notes "…"
# `--latest=false` keeps the README's "latest release" link on the app.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/fonts"
modules="$root/web/node_modules/@fontsource-variable"
[[ -d "$root/web/node_modules" ]] || (cd "$root/web" && bun install --frozen-lockfile)

rm -rf "$out"
mkdir -p "$out"
for name in noto-serif-kr noto-serif-jp noto-serif-sc noto-serif-tc; do
    stage="$out/stage/$name"
    mkdir -p "$stage/files"
    cp "$modules/$name/wght.css" "$modules/$name/LICENSE" "$stage/"
    grep -o "files/[^)]*\.woff2" "$modules/$name/wght.css" | while read -r file; do
        cp "$modules/$name/$file" "$stage/files/"
    done
    # The same input gives the same zip: fixed dates and modes, files in sorted order.
    chmod -R u=rwX,go=rX "$stage"
    find "$stage" -exec env TZ=UTC touch -t 198001010000 {} +
    (cd "$stage" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | TZ=UTC zip -X -q -@ "$out/$name.zip")
done
rm -rf "$out/stage"

cd "$out"
shasum -a 256 ./*.zip | sed 's|\./||'
