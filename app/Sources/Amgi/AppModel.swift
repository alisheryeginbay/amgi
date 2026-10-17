import AmgiCore
import AppKit
import Observation
import SwiftUI
import UniformTypeIdentifiers

/// What the sidebar shows in the detail pane.
enum SidebarItem: Hashable {
    case dashboard
    /// Study the whole collection.
    case review
    /// Browse a deck and its subdecks.
    case deck(String)
    /// Browse cards with this tag, across all decks.
    case tag(String)
    /// Browse a saved view's search, by its id.
    case view(Int64)
}

/// A saved view being added (no `id`) or edited.
struct SavedViewDraft: Identifiable {
    var id: Int64?
    var name: String
    var query: String
    var icon: String?
}

struct DeckName: Equatable {
    let name: String
    let title: String
}

@Observable
@MainActor
final class AppModel {
    private static let folderKey = "collectionPath"
    private static let retentionKey = "desiredRetention"
    /// The range the core accepts.
    static let retentionRange = 0.7...0.99

    private(set) var folder: URL?
    private(set) var decks: [DeckInfo] = []
    /// Deck names and titles without counts, for views that only show the decks:
    /// counts change with every answer, these only when files do.
    private(set) var deckNames: [DeckName] = []
    private(set) var tags: [String] = []
    /// Lowercased tag → icon, such as an emoji.
    private(set) var tagIcons: [String: String] = [:]
    private(set) var savedViews: [SavedView] = []
    /// The saved view shown in the add or edit sheet.
    var editingView: SavedViewDraft?
    private(set) var warnings: [CollectionWarning] = []
    private(set) var today = TodayStats(reviewed: 0, newStudied: 0, newAvailable: 0)
    private(set) var current: StudyCard?
    private(set) var importing = false
    private(set) var dashboard: Dashboard?
    /// Cards of the deck being browsed, or every card for a tag or saved view: the
    /// browse pane runs their searches.
    private(set) var browsed: [BrowseCard] = []
    /// How many browsed cards match the tag or saved view, as counted by the browse pane.
    private(set) var browsedCount: Int?
    var revealed = false
    /// Cards selected in the browse table.
    var browseSelection: Set<String> = []
    /// The browse search, as typed.
    var search = ""
    var errorMessage: String?
    var importSummary: ImportSummary?

    /// Target probability of recalling a card when it comes due. Applies to every collection.
    var desiredRetention: Double = AppModel.savedRetention() {
        didSet {
            guard oldValue != desiredRetention else { return }
            UserDefaults.standard.set(desiredRetention, forKey: Self.retentionKey)
            collection?.setDesiredRetention(retention: Float(desiredRetention))
            refreshIntervals()
        }
    }

    var selection: SidebarItem = .review {
        didSet {
            guard oldValue != selection else { return }
            search = ""
            browseSelection = []
            browsedCount = nil
            switch selection {
            case .dashboard: loadDashboard()
            case .review: loadNext()
            case .deck, .tag, .view: loadBrowsed()
            }
        }
    }

    /// The cards being browsed, if any.
    var browseScope: StudyScope? {
        switch selection {
        case .dashboard, .review: nil
        case .deck(let name): .deck(name: name)
        case .tag, .view: .all
        }
    }

    /// The search a tag or saved view stands for, applied before the one typed.
    var browseQuery: String {
        switch selection {
        case .dashboard, .review, .deck: ""
        case .tag(let tag): "#\(tag)"
        case .view(let id): savedView(id)?.query ?? ""
        }
    }

    func savedView(_ id: Int64) -> SavedView? {
        savedViews.first { $0.id == id }
    }

    private var collection: CardCollection?
    /// The web view behind every detail pane.
    let web = WebHost()

    init() {
        web.onMessage = { [weak self] in self?.handle($0) }
        web.contextMenu = { [weak self] ids in self?.menuItems(for: ids) ?? [] }
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
            collection.setDesiredRetention(retention: Float(desiredRetention))
            self.collection = collection
            self.folder = url
            web.collectionRoot = url
            self.selection = .review
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
        // Only changed values are assigned, so views that read the rest are left alone:
        // a reload when the app comes to the front usually changes nothing.
        perform {
            let decks = try collection.decks()
            update(\.decks, decks)
            update(\.deckNames, decks.map { DeckName(name: $0.name, title: $0.title) })
            update(\.tags, collection.tags())
            update(\.tagIcons, try collection.tagIcons())
            update(\.savedViews, try collection.savedViews())
            update(\.today, try collection.today())
            update(\.warnings, collection.warnings())
        }
        if selection == .dashboard { loadDashboard() }
        if browseScope != nil { loadBrowsed() }
        if current == nil || !revealed { loadNext() }
    }

    /// Statistics are only gathered while the dashboard is showing.
    func loadDashboard() {
        guard let collection else { return }
        perform { update(\.dashboard, try collection.dashboard(activityDays: Self.activityDays())) }
    }

    /// Weeks of activity on the dashboard, one column each.
    private static let activityWeeks = 53

    /// Days to request so the heatmap ends today and fills its week columns.
    private static func activityDays(now: Date = .now) -> UInt32 {
        // Study days roll over at 4am, like the scheduler.
        let today = now.addingTimeInterval(-4 * 3600)
        let calendar = Calendar.current
        let weekday = (calendar.component(.weekday, from: today) - calendar.firstWeekday + 7) % 7
        return UInt32((activityWeeks - 1) * 7 + weekday + 1)
    }

    // MARK: Web panes

    /// What the web panes show. Read while rendering the detail column, so any change
    /// here updates the page.
    func webState(autoplay: Bool, preview: Bool) -> WebState {
        let pane: WebPane = switch selection {
        case .dashboard: .dashboard
        case .review: .study
        case .deck, .tag, .view: .browse
        }
        let browse: BrowseSource? = switch selection {
        case .dashboard, .review: nil
        case .deck(let name): BrowseSource(scope: "deck:\(name)", query: "", cards: browsed)
        case .tag(let tag): BrowseSource(scope: "tag:\(tag)", query: browseQuery, cards: browsed)
        case .view(let id): BrowseSource(scope: "view:\(id)", query: browseQuery, cards: browsed)
        }
        return WebState(
            pane: pane,
            environment: .current,
            icons: tagIcons,
            study: StudyPayload(card: current, revealed: revealed, autoplay: autoplay),
            dashboard: pane == .dashboard ? dashboard.map { DashboardPayload($0, due: totalDue, today: today) } : nil,
            browse: browse,
            search: search,
            preview: preview
        )
    }

    /// The one browsed card selected, if exactly one is.
    var selectedBrowseCard: BrowseCard? {
        guard browseSelection.count == 1, let id = browseSelection.first else { return nil }
        return browsed.first { $0.id == id }
    }

    /// Suspend or unsuspend cards; the next card comes up if the one studied is suspended.
    func setSuspended(_ ids: [String], _ suspended: Bool) {
        guard let collection else { return }
        perform {
            let changed = try collection.setSuspended(cardIds: ids, suspended: suspended)
            if !suspended && changed == 0 {
                errorMessage = "These cards are suspended by their deck: remove `suspended` from the tags in the deck file's front matter."
            }
        }
        if suspended, let current, ids.contains(current.id) {
            self.current = nil
        }
        refresh()
    }

    /// Right-click items for browse rows.
    private func menuItems(for ids: [String]) -> [NSMenuItem] {
        let cards = browsed.filter { ids.contains($0.id) }
        guard !cards.isEmpty else { return [] }
        var items: [NSMenuItem] = []
        if cards.count == 1, let card = cards.first {
            items.append(ActionMenuItem("Edit Card") { [weak self] in self?.revealInEditor(path: card.path) })
        }
        let suspend = cards.contains { !$0.suspended }
        let noun = cards.count == 1 ? "Card" : "\(cards.count) Cards"
        let ids = cards.map(\.id)
        items.append(ActionMenuItem(suspend ? "Suspend \(noun)" : "Unsuspend \(noun)") { [weak self] in
            self?.setSuspended(ids, suspend)
        })
        return items
    }

    func revealWithAnimation() {
        guard current != nil else { return }
        withAnimation(.smooth(duration: 0.3)) { revealed = true }
    }

    func answerWithAnimation(_ rating: Rating) {
        guard current != nil, revealed else { return }
        withAnimation(.smooth(duration: 0.25)) { answer(rating) }
    }

    private func handle(_ message: WebMessage) {
        switch message {
        case .reveal: revealWithAnimation()
        case .answer(let rating): answerWithAnimation(rating)
        case .open(let path): revealInEditor(path: path)
        case .select(let ids): update(\.browseSelection, Set(ids))
        case .count(let count): update(\.browsedCount, count)
        case .ready, .contextCards: break
        }
    }

    /// Card lists are only read while a deck, tag or saved view is showing.
    func loadBrowsed() {
        guard let collection, let scope = browseScope else { return }
        update(\.browsed, collection.cards(scope: scope))
    }

    func loadNext() {
        guard let collection else { return }
        perform {
            update(\.current, try collection.nextCard(scope: .all))
            update(\.revealed, false)
        }
    }

    /// Recompute the answer button intervals of the card on screen, keeping it revealed.
    private func refreshIntervals() {
        guard let collection, let card = current else { return }
        perform {
            if let next = try collection.nextCard(scope: .all), next.id == card.id {
                update(\.current, next)
            }
        }
    }

    private static func savedRetention() -> Double {
        let saved = UserDefaults.standard.object(forKey: retentionKey) as? Double ?? 0.9
        return min(max(saved, retentionRange.lowerBound), retentionRange.upperBound)
    }

    func answer(_ rating: Rating) {
        guard let collection, let card = current else { return }
        perform { try collection.answer(cardId: card.id, rating: rating) }
        current = nil
        refresh()
    }

    func icon(forTag tag: String) -> String? {
        tagIcons[tag.lowercased()]
    }

    /// `nil` removes the icon.
    func setIcon(_ icon: String?, forTag tag: String) {
        guard let collection else { return }
        perform {
            try collection.setTagIcon(tag: tag, icon: icon)
            update(\.tagIcons, try collection.tagIcons())
        }
    }

    // MARK: Saved views

    /// What's browsed and searched now, as one search to save.
    var currentQuery: String {
        let base = switch selection {
        case .deck(let name): "deck:\"\(name)\""
        default: browseQuery
        }
        return [base, search.trimmingCharacters(in: .whitespaces)].filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// Opens the sheet to save the current search as a view.
    func saveCurrentSearch() {
        editingView = SavedViewDraft(name: search.trimmingCharacters(in: .whitespaces), query: currentQuery)
    }

    func editSavedView(_ view: SavedView) {
        editingView = SavedViewDraft(id: view.id, name: view.name, query: view.query, icon: view.icon)
    }

    /// Adds or updates a view; a new one is then shown, with the search it came from cleared.
    func commit(_ draft: SavedViewDraft) {
        guard let collection else { return }
        perform {
            if let id = draft.id {
                try collection.updateSavedView(view: SavedView(id: id, name: draft.name, query: draft.query, icon: draft.icon))
                update(\.savedViews, try collection.savedViews())
            } else {
                let id = try collection.addSavedView(name: draft.name, query: draft.query)
                update(\.savedViews, try collection.savedViews())
                selection = .view(id)
            }
        }
    }

    /// `nil` removes the icon.
    func setIcon(_ icon: String?, for view: SavedView) {
        guard let collection else { return }
        perform {
            try collection.updateSavedView(view: SavedView(id: view.id, name: view.name, query: view.query, icon: icon))
            update(\.savedViews, try collection.savedViews())
        }
    }

    func deleteSavedView(_ view: SavedView) {
        guard let collection else { return }
        perform {
            try collection.deleteSavedView(id: view.id)
            update(\.savedViews, try collection.savedViews())
        }
        if selection == .view(view.id) { selection = .review }
    }

    /// Open a deck file, given relative to the collection root.
    func revealInEditor(path: String) {
        guard let folder else { return }
        NSWorkspace.shared.open(folder.appendingPathComponent(path))
    }

    /// Assign only if the value changed; every assignment re-renders the views that read it.
    private func update<Value: Equatable>(_ property: ReferenceWritableKeyPath<AppModel, Value>, _ value: Value) {
        if self[keyPath: property] != value {
            self[keyPath: property] = value
        }
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
        var first = "\(cards) cards from \(notes) notes were written to \(decks)."
        if reviewedCards > 0 { first += " \(reviewedCards) kept their review history." }
        if existingHistory > 0 { first += " \(existingHistory) already had review history here, which was kept." }
        if newCards > 0 { first += newCards == 1 ? " 1 is new." : " \(newCards) are new." }
        var lines = [first]
        if suspendedCards > 0 {
            lines.append(suspendedCards == 1
                ? "1 suspended card was imported with the #suspended tag."
                : "\(suspendedCards) suspended cards were imported with the #suspended tag.")
        }
        if skippedNotes > 0 { lines.append("\(skippedNotes) notes could not be converted, such as image occlusions.") }
        if images > 0 { lines.append("\(images) images were copied to the media folder.") }
        if missingImages > 0 { lines.append("\(missingImages) images were missing from the package.") }
        if sounds > 0 { lines.append("\(sounds) sounds were copied to the media folder.") }
        if missingSounds > 0 { lines.append("\(missingSounds) sounds were missing from the package.") }
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
