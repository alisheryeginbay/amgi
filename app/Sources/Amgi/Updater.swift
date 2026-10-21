import Combine
import Sparkle
import SwiftUI

/// Checks for new versions with Sparkle, from the feed and key in Amgi.app's Info.plist
/// (`SUFeedURL`, `SUPublicEDKey`, written by scripts/bundle.sh). Sparkle asks on the
/// second launch whether to check automatically.
@MainActor @Observable
final class Updater {
    /// Nil in the dev build, which has no Info.plist to read the feed from.
    @ObservationIgnored private let controller: SPUStandardUpdaterController?
    @ObservationIgnored private var observation: AnyCancellable?
    /// False while a check is already running.
    private(set) var canCheckForUpdates = false

    init() {
        guard !isDevBuild else {
            controller = nil
            return
        }
        let controller = SPUStandardUpdaterController(
            startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)
        self.controller = controller
        observation = controller.updater.publisher(for: \.canCheckForUpdates)
            .receive(on: RunLoop.main)
            .sink { [weak self] in self?.canCheckForUpdates = $0 }
    }

    var isAvailable: Bool { controller != nil }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }
}

/// "Check for Updates…" in the app menu, below "About Amgi".
struct UpdateCommands: Commands {
    let updater: Updater

    var body: some Commands {
        CommandGroup(after: .appInfo) {
            if updater.isAvailable {
                Button("Check for Updates…") { updater.checkForUpdates() }
                    .disabled(!updater.canCheckForUpdates)
            }
        }
    }
}
