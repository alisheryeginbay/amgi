// swift-tools-version: 6.2
import Foundation
import PackageDescription

// Static library produced by `scripts/build-core.sh`.
let coreLibrary = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../target/amgi/libamgi_core.a")
    .standardized.path

let package = Package(
    name: "Amgi",
    platforms: [.macOS(.v26)],
    targets: [
        // C module exposing the UniFFI header; the generated Swift imports it as `amgi_coreFFI`.
        .target(name: "amgi_coreFFI"),
        // Generated UniFFI bindings for the Rust core.
        .target(
            name: "AmgiCore",
            dependencies: ["amgi_coreFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [.unsafeFlags([coreLibrary])]
        ),
        // `Web` holds the Svelte panes, built by `scripts/build-web.sh`; `Emoji.tsv` the emoji
        // picker's data, written by `scripts/build-emoji.py`; `Amgi.svg` the welcome screen's
        // handwriting, written by `scripts/readme-svg.mjs`.
        .executableTarget(name: "Amgi", dependencies: ["AmgiCore"], resources: [.copy("Web"), .copy("Emoji.tsv"), .copy("Amgi.svg")]),
    ]
)
