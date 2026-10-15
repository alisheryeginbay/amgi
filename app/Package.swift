// swift-tools-version: 6.0
import Foundation
import PackageDescription

// Static library produced by `scripts/build-core.sh`.
let coreLibrary = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../target/amgi/libamgi_core.a")
    .standardized.path

let package = Package(
    name: "Amgi",
    platforms: [.macOS(.v14)],
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
        .executableTarget(name: "Amgi", dependencies: ["AmgiCore"]),
    ]
)
