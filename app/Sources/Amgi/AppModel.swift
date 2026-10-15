import AmgiCore
import AppKit
import Observation
import UniformTypeIdentifiers

/// What the sidebar shows in the detail pane.
enum SidebarItem: Hashable {
    case dashboard
    /// A deck and its subdecks; `nil` is the whole collection.
    case deck(String?)
    /// Cards with this tag, across all decks.
    case tag(String)
}

@Observable
@MainActor
final class AppModel {
    private static let folderKey = "collectionPath"

    private(set) var folder: URL?
    private(set) var decks: [DeckInfo] = []
    private(set) var tags: [String] = []
    private(set) var warnings: [CollectionWarning] = []
    private(set) var today = TodayStats(reviewed: 0, newStudied: 0, newAvailable: 0)
    private(set) var current: StudyCard?
    private(set) var importing = false
    private(set) var dashboard: Dashboard?
    var revealed = false
    var errorMessage: String?
    var importSummary: ImportSummary?

    var selection: SidebarItem = .deck(nil) {
        didSet {
            guard oldValue != selection else { return }
            if selection == .dashboard { loadDashboard() } else { loadNext() }
        }
    }

    /// The deck to study; `nil` for the whole collection or a tag.
    var selectedDeck: String? {
        if case .deck(let name) = selection { name } else { nil }
    }

    var scope: StudyScope {
        switch selection {
        case .dashboard, .deck(nil): .all
        case .deck(let name?): .deck(name: name)
        case .tag(let name): .tag(name: name)
        }
    }

    private var collection: CardCollection?

    init() {
        if let path = UserDefaults.standard.string(forKey: Self.folderKey) {
            open(URL(fileURLWithPath: path))
        }
    }

    /// Top-level decks already include their subdecks.
    var totalDue: UInt32 { decks.filter { !$0.name.contains("/") }.reduce(0) { $0 + $1.due } }

    func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.prompt = "Open"
        panel.message = "Choose a folder of Markdown flash cards"
        if panel.runModal() == .OK, let url = panel.url {
            open(url)
        }
    }

    func open(_ url: URL) {
        perform {
            let collection = try CardCollection.open(root: url.path)
            collection.setUtcOffset(seconds: Int32(TimeZone.current.secondsFromGMT()))
            self.collection = collection
            self.folder = url
            self.selection = .deck(nil)
            UserDefaults.standard.set(url.path, forKey: Self.folderKey)
        }
        refresh()
    }

    /// Convert an Anki package into Markdown decks inside the open folder.
    func chooseAnkiPackage() {
        guard let collection else { return }
        let panel = NSOpenPanel()
        panel.allowedContentTypes = ["apkg", "colpkg"].compactMap { UTType(filenameExtension: $0) }
        panel.prompt = "Import"
        panel.message = "Choose an Anki deck (.apkg) or collection (.colpkg) to import"
        guard panel.runModal() == .OK, let url = panel.url else { return }

        importing = true
        Task {
            let result = await Task.detached {
                Result { try collection.importAnki(path: url.path) }
            }.value
            importing = false
            switch result {
            case .success(let summary): importSummary = summary
            case .failure(let error): errorMessage = error.localizedDescription
            }
            refresh()
        }
    }

    /// Re-read the Markdown files and update counts. Keeps the current card if it still exists.
    func reload() {
        perform { try collection?.reload() }
        refresh()
    }

    func refresh() {
        guard let collection else { return }
        perform {
            decks = try collection.decks()
            tags = collection.tags()
            today = try collection.today()
            warnings = collection.warnings()
        }
        if selection == .dashboard { loadDashboard() }
        if current == nil || !revealed { loadNext() }
    }

    /// Statistics are only gathered while the dashboard is showing.
    func loadDashboard() {
        guard let collection else { return }
        perform { dashboard = try collection.dashboard(activityDays: ActivityHeatmap.dayCount()) }
    }

    func loadNext() {
        guard let collection else { return }
        perform {
            current = try collection.nextCard(scope: scope)
            revealed = false
        }
    }

    func answer(_ rating: Rating) {
        guard let collection, let card = current else { return }
        perform { try collection.answer(cardId: card.id, rating: rating) }
        current = nil
        refresh()
    }

    func revealInEditor(_ card: StudyCard) {
        guard let folder else { return }
        NSWorkspace.shared.open(folder.appendingPathComponent(card.path))
    }

    private func perform(_ body: () throws -> Void) {
        do {
            try body()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

extension ImportSummary {
    var message: String {
        let decks = files.count == 1 ? "1 deck" : "\(files.count) decks"
        var lines = ["\(cards) cards from \(notes) notes were written to \(decks). \(reviewedCards) kept their review history."]
        if suspendedCards > 0 { lines.append("\(suspendedCards) suspended cards were left out.") }
        if skippedNotes > 0 { lines.append("\(skippedNotes) notes could not be converted, such as image occlusions.") }
        if images > 0 { lines.append("\(images) images were copied to the media folder.") }
        if missingImages > 0 { lines.append("\(missingImages) images were missing from the package.") }
        if soundsRemoved > 0 { lines.append("\(soundsRemoved) sounds were removed.") }
        return lines.joined(separator: "\n\n")
    }
}

extension Rating: CaseIterable, Identifiable {
    public static var allCases: [Rating] { [.again, .hard, .good, .easy] }
    public var id: Self { self }

    var label: String {
        switch self {
        case .again: "Again"
        case .hard: "Hard"
        case .good: "Good"
        case .easy: "Easy"
        }
    }

    var index: Int { Self.allCases.firstIndex(of: self)! }
}
