#!/usr/bin/env bash
# Build the Rust core and generate Swift bindings for the SwiftUI app.
#   scripts/build-core.sh [debug|release]   (default: release)
# Honors CARGO_TARGET_DIR, so a build with other flags can keep its own directory.
set -euo pipefail

profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# Match the app's minimum macOS version (app/Package.swift).
export MACOSX_DEPLOYMENT_TARGET=26.0

flag=()
[[ "$profile" == "release" ]] && flag=(--release)
cargo build -p amgi-core "${flag[@]}"

target="${CARGO_TARGET_DIR:-$root/target}"
lib="$target/$profile/libamgi_core.dylib"
out="target/bindings"
cargo run -q -p uniffi-bindgen -- generate "$lib" --language swift --out-dir "$out"

mkdir -p app/Sources/AmgiCore app/Sources/amgi_coreFFI/include
# Copy only when the bindings changed, so Swift doesn't recompile them on every build.
update() { cmp -s "$1" "$2/$(basename "$1")" || cp "$1" "$2/"; }
update "$out/amgi_core.swift" app/Sources/AmgiCore
update "$out/amgi_coreFFI.h" app/Sources/amgi_coreFFI/include

# The app links the static library; point the package at the chosen profile.
mkdir -p target
ln -sfn "$target/$profile" target/amgi
echo "Core built ($profile); bindings written to app/Sources."
