// swift-tools-version: 6.2
import Foundation
import PackageDescription

// Static library produced by `scripts/build-core.sh` (run by `make`).
let coreLibrary = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../target/amgi/libamgi_core.a")
    .standardized.path

// Debug builds run as a bare executable; this names them Amgi Dev in the menu bar.
let devInfoPlist = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("DevInfo.plist")
    .standardized.path

let package = Package(
    name: "Amgi",
    platforms: [.macOS(.v26)],
    dependencies: [
        // Checks for and installs updates; see Updater.swift.
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.10.0"),
    ],
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
        // handwriting, written by `scripts/readme-svg.mjs`; `AmgiDev.icns` the dev build's
        // violet icon, written by `scripts/build-dev-icon.sh`.
        .executableTarget(
            name: "Amgi",
            dependencies: ["AmgiCore", .product(name: "Sparkle", package: "Sparkle")],
            resources: [.copy("Web"), .copy("Emoji.tsv"), .copy("Amgi.svg"), .copy("AmgiDev.icns")],
            linkerSettings: [
                // Amgi.app keeps Sparkle.framework in Contents/Frameworks (scripts/bundle.sh).
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
                .unsafeFlags(
                    ["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist", "-Xlinker", devInfoPlist],
                    .when(configuration: .debug)
                ),
            ]
        ),
    ]
)
