import AmgiCore
import CryptoKit
import Foundation
import Observation

/// Source Han Serif (Noto Serif CJK) for card text, about 6 MB per language: fetched
/// the first time a collection uses Hangul, kana or Han characters instead of shipping
/// in the app. The packs are zips made by `scripts/build-fonts.sh`, attached to a GitHub
/// release and checked against the SHA-256 sums here before they're unpacked. Until one
/// is installed, the page falls back to the Mac's own serif for that language.
@Observable
@MainActor
final class FontPacks {
    struct Pack: Sendable {
        /// The CJK language whose forms the pack draws, as the page's `data-han`.
        let han: String
        /// Its zip and folder name, such as `noto-serif-kr`.
        let name: String
        let sha256: String

        /// Installed copies are named by their sum, so a new pack never mixes with an old one.
        var folder: String { "\(name)-\(sha256.prefix(16))" }
    }

    nonisolated static let release = URL(string: "https://github.com/alisheryeginbay/amgi/releases/download/fonts-1/")!
    nonisolated static let packs = [
        Pack(han: "ko", name: "noto-serif-kr", sha256: "1f94ab241dbc28d70206daaca4bf24a0e659df445c5b33a3e389207796d8d97a"),
        Pack(han: "ja", name: "noto-serif-jp", sha256: "75667617cce11295ac78a8d71dfe65e80aa150879135958874cced942af24926"),
        Pack(han: "zh-hans", name: "noto-serif-sc", sha256: "afc533e2199bb2110ccc8f9051351a081a2d4fd0737010f5c382b28c8171e37b"),
        Pack(han: "zh-hant", name: "noto-serif-tc", sha256: "338bacd057e76a34e56d8d23556990069d281fdda78404215713b528e575bb3b"),
    ]

    /// Where packs are unpacked, served to the page at `amgi://fonts/`.
    nonisolated static let root = URL.applicationSupportDirectory.appending(path: "io.github.alisheryeginbay.amgi/Fonts", directoryHint: .isDirectory)

    /// The installed packs' stylesheets, as page URLs.
    private(set) var stylesheets: [String] = []
    @ObservationIgnored private var downloading: Set<String> = []
    /// When a pack last failed to download, so a reload soon after doesn't try again.
    @ObservationIgnored private var failed: [String: Date] = [:]
    private static let retryAfter: TimeInterval = 10 * 60

    init() {
        findInstalled()
    }

    /// Download the packs `scripts` needs that aren't installed yet. `han` is the CJK
    /// language the Mac prefers, if any, which picks the forms of Han characters.
    func fetch(for scripts: Scripts, han: String?) {
        for pack in Self.needed(scripts, han: han) where !isInstalled(pack) && !downloading.contains(pack.name) {
            if let last = failed[pack.name], Date.now.timeIntervalSince(last) < Self.retryAfter { continue }
            downloading.insert(pack.name)
            Task {
                do {
                    try await Self.install(pack)
                    failed[pack.name] = nil
                } catch {
                    failed[pack.name] = .now
                    print("Could not download \(pack.name):", error)
                }
                downloading.remove(pack.name)
                findInstalled()
            }
        }
    }

    /// Hangul needs the Korean pack and kana the Japanese one; those draw Han characters
    /// too. Han characters alone take the Mac's CJK language, or Simplified Chinese.
    static func needed(_ scripts: Scripts, han: String?) -> [Pack] {
        var wanted: Set<String> = []
        if scripts.hangul { wanted.insert("ko") }
        if scripts.kana { wanted.insert("ja") }
        if scripts.han && wanted.isEmpty { wanted.insert(han ?? "zh-hans") }
        return packs.filter { wanted.contains($0.han) }
    }

    private func isInstalled(_ pack: Pack) -> Bool {
        FileManager.default.fileExists(atPath: Self.root.appending(path: "\(pack.folder)/wght.css").path)
    }

    private func findInstalled() {
        let found = Self.packs.filter(isInstalled).map { "amgi://fonts/\($0.folder)/wght.css" }
        if found != stylesheets { stylesheets = found }
    }

    private enum PackError: Error {
        case status(Int)
        case checksum(String)
        case unzip(Int32)
    }

    /// Download, check and unpack `pack`, replacing any older copy.
    private nonisolated static func install(_ pack: Pack) async throws {
        let (download, response) = try await URLSession.shared.download(from: release.appending(path: "\(pack.name).zip"))
        defer { try? FileManager.default.removeItem(at: download) }
        if let status = (response as? HTTPURLResponse)?.statusCode, status != 200 { throw PackError.status(status) }
        let sum = SHA256.hash(data: try Data(contentsOf: download)).map { String(format: "%02x", $0) }.joined()
        guard sum == pack.sha256 else { throw PackError.checksum(sum) }

        let files = FileManager.default
        try files.createDirectory(at: root, withIntermediateDirectories: true)
        // Unpacked beside its final place, then moved there whole, so a half-written
        // pack is never served.
        let partial = root.appending(path: ".\(pack.folder)-\(UUID().uuidString)")
        defer { try? files.removeItem(at: partial) }
        try await unzip(download, to: partial)
        for old in try files.contentsOfDirectory(atPath: root.path) where old.hasPrefix("\(pack.name)-") {
            try? files.removeItem(at: root.appending(path: old))
        }
        try files.moveItem(at: partial, to: root.appending(path: pack.folder))
    }

    private nonisolated static func unzip(_ zip: URL, to folder: URL) async throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        process.arguments = ["-x", "-k", zip.path, folder.path]
        let status: Int32 = try await withCheckedThrowingContinuation { done in
            process.terminationHandler = { done.resume(returning: $0.terminationStatus) }
            do { try process.run() } catch { done.resume(throwing: error) }
        }
        if status != 0 { throw PackError.unzip(status) }
    }
}
