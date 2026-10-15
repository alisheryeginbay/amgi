import AmgiCore
import AppKit
import Observation

@Observable
@MainActor
final class AppModel {
    private static let folderKey = "collectionPath"

    private(set) var folder: URL?
    private(set) var decks: [DeckInfo] = []
    private(set) var warnings: [CollectionWarning] = []
    private(set) var today = TodayStats(reviewed: 0, newStudied: 0, newAvailable: 0)
    private(set) var current: StudyCard?
    var revealed = false
    var errorMessage: String?

    /// `nil` studies the whole collection.
    var selectedDeck: String? {
        didSet {
            if oldValue != selectedDeck { loadNext() }
        }
    }

    private var collection: CardCollection?

    init() {
        if let path = UserDefaults.standard.string(forKey: Self.folderKey) {
            open(URL(fileURLWithPath: path))
        }
    }

    var totalDue: UInt32 { decks.reduce(0) { $0 + $1.due } }

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
            self.selectedDeck = nil
            UserDefaults.standard.set(url.path, forKey: Self.folderKey)
        }
        refresh()
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
            today = try collection.today()
            warnings = collection.warnings()
        }
        if current == nil || !revealed { loadNext() }
    }

    func loadNext() {
        guard let collection else { return }
        perform {
            current = try collection.nextCard(deck: selectedDeck)
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
