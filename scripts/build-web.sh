#!/usr/bin/env bash
# Build the Svelte panes and copy them into the app's resources.
#   scripts/build-web.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/web"

[[ -d node_modules ]] || bun install --frozen-lockfile
bun run build

out="$root/app/Sources/Amgi/Web"
rm -rf "$out"
cp -R dist "$out"
echo "Web panes built; copied to app/Sources/Amgi/Web."
