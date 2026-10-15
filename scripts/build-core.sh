#!/usr/bin/env bash
# Build the Rust core and generate Swift bindings for the SwiftUI app.
#   scripts/build-core.sh [debug|release]   (default: release)
set -euo pipefail

profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# Match the app's minimum macOS version (app/Package.swift).
export MACOSX_DEPLOYMENT_TARGET=14.0

flag=()
[[ "$profile" == "release" ]] && flag=(--release)
cargo build -p amgi-core "${flag[@]}"

lib="target/$profile/libamgi_core.dylib"
out="target/bindings"
cargo run -q -p uniffi-bindgen -- generate "$lib" --language swift --out-dir "$out"

mkdir -p app/Sources/AmgiCore app/Sources/amgi_coreFFI/include
cp "$out/amgi_core.swift" app/Sources/AmgiCore/
cp "$out/amgi_coreFFI.h" app/Sources/amgi_coreFFI/include/

# The app links the static library; point the package at the chosen profile.
ln -sfn "$profile" target/amgi
echo "Core built ($profile); bindings written to app/Sources."
