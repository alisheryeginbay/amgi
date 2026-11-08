import AmgiCore
import AppKit
import Carbon.HIToolbox
import Observation
import SwiftUI
import UniformTypeIdentifiers

struct DeckName: Equatable {
    let name: String
    let title: String
    /// The note type most of its notes have, offered first for adding to it.
    let noteType: String?
}

@Observable
@MainActor
final class AppModel {
    private static let folderKey = "collectionPath"
    private static let quickAddShortcutKey = "quickAddShortcut"
    private static let ankiConnectKey = "ankiConnect"
    /// The port AnkiConnect listens on, which Yomitan and other tools try by default.
    static let ankiConnectPort: UInt16 = 8765

    private(set) var folder: URL?
    /// Deck names and titles, for adding cards to them.
    private(set) var deckNames: [DeckName] = []
    /// The note types in the collection's `note-types` folder, by name.
    private(set) var noteTypes: [NoteTypeInfo] = []
    private(set) var warnings: [CollectionWarning] = []
    private(set) var today = TodayStats(reviewed: 0, newStudied: 0, newAvailable: 0, dueAvailable: 0, learning: 0, studySecs: 0, secsLeft: nil)
    /// The card being studied, and whether its answer shows.
    private(set) var study: StudyState = .none {
        didSet { timer.show(study.card?.id) }
    }
    /// What last showed an answer or rated a card, for whether the change animates.
    private(set) var studyInput: StudyInput = .pointer
    var current: StudyCard? { study.card }
    var revealed: Bool { study.revealed }
    /// Whether an answer given since the folder opened can be taken back with ⌘Z.
    private(set) var canUndoAnswer = false
    /// The last answer made its card a leech; shown beside the next card.
    private(set) var leechNotice: LeechNotice?
    private(set) var importing = false
    /// The card the Card Info window shows, kept current as cards are studied or edited.
    private(set) var infoCard: CardInfo?
    @ObservationIgnored private var infoCardID: String?
    /// Opens the Card Info window. Set by the main window's view, which can open windows.
    @ObservationIgnored var openCardInfoWindow: () -> Void = {}
    /// Set by the main window, which can open SwiftUI's windows; the command palette uses them.
    @ObservationIgnored var openSettingsWindow: () -> Void = {}
    @ObservationIgnored var openShortcutsWindow: () -> Void = {}
    var errorMessage: String?
    var importSummary: ImportSummary?
    /// The keys that rate a shown answer, from `~/.config/amgi/config.toml`.
    private(set) var answerKeys = AnswerKeys(again: "1", hard: "2", good: "3", easy: "4")
    /// The last error in that file, so it's shown once rather than every time the app
    /// comes to the front.
    @ObservationIgnored private var configError: String?

    /// Target probability of recalling a card when it comes due, from the open
    /// collection's `amgi.toml`.
    private(set) var desiredRetention: Double = 0.9

    /// Whether ⌥⌘N opens Add Card from any app.
    var quickAddShortcut: Bool = UserDefaults.standard.object(forKey: AppModel.quickAddShortcutKey) as? Bool ?? true {
        didSet {
            guard oldValue != quickAddShortcut else { return }
            UserDefaults.standard.set(quickAddShortcut, forKey: Self.quickAddShortcutKey)
            registerQuickAddShortcut()
        }
    }

    /// Whether tools made for Anki, such as Yomitan, can add cards through AnkiConnect.
    var ankiConnect: Bool = UserDefaults.standard.bool(forKey: AppModel.ankiConnectKey) {
        didSet {
            guard oldValue != ankiConnect else { return }
            UserDefaults.standard.set(ankiConnect, forKey: Self.ankiConnectKey)
            restartAnkiConnect()
        }
    }
    /// Why AnkiConnect isn't answering although it's on, such as its port being taken.
    private(set) var ankiConnectError: String?

    private var collection: CardCollection?
    /// The web view the cards show in.
    let web = WebHost()
    /// CJK fonts for card text, downloaded when the collection first uses them.
    let fonts = FontPacks()
    /// The header's More button, which its menu opens under.
    @ObservationIgnored weak var moreMenuAnchor: NSView?
    /// Times the card being studied.
    @ObservationIgnored private let timer = StudyTimer()
    @ObservationIgnored private var keyMonitor: Any?
    @ObservationIgnored private var countsTimer: Timer?
    @ObservationIgnored private let quickAdd = QuickAddPanel()
    @ObservationIgnored private let commandPalette = CommandPalette()
    @ObservationIgnored private var quickAddHotKey: GlobalHotKey?
    @ObservationIgnored private var ankiConnectServer: AnkiConnectServer?

    init() {
        timer.view = web.webView
        web.onMessage = { [weak self] in self?.handle($0) }
        loadConfig()
        monitorReviewShortcuts()
        refreshCountsPeriodically()
        registerQuickAddShortcut()
        if let path = UserDefaults.standard.string(forKey: Self.folderKey) {
            open(URL(fileURLWithPath: path))
        }
    }

    /// Due cards, with reviews capped by the daily limit, which decks share.
    var totalDue: UInt32 { today.dueAvailable }
    /// Cards answered Again and not yet passed, waiting to come back or back already.
    var totalLearning: UInt32 { today.learning }
    /// Due and learning cards, which the menu bar counts.
    var totalDueAndLearning: UInt32 { totalDue + totalLearning }
    /// Cards still to study today: due, learning and new.
    var totalLeft: UInt32 { totalDue + totalLearning + today.newAvailable }

    /// "3 due · 2 learning · 5 new · about 8 min", leaving out learning when there is
    /// none, and the time left until there are timed reviews to estimate it from.
    var studyCounts: String {
        [
            "\(totalDue) due",
            totalLearning > 0 ? "\(totalLearning) learning" : nil,
            "\(today.newAvailable) new",
            timeLeft.map { "about \($0)" },
        ].compactMap { $0 }.joined(separator: " · ")
    }

    /// About how long the cards left today will take, such as "8 min"; `nil` when none
    /// are left or there's nothing to estimate it from.
    var timeLeft: String? {
        guard totalLeft > 0, let secs = today.secsLeft else { return nil }
        return formatStudyTime(seconds: max(secs, 60))
    }

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
            self.canUndoAnswer = false
            self.leechNotice = nil
            self.folder = url
            web.collectionRoot = url
            UserDefaults.standard.set(url.path, forKey: Self.folderKey)
        }
        restartAnkiConnect()
        refresh()
    }

    /// Close the collection and go back to the welcome screen, which stays on next launch.
    func closeFolder() {
        guard folder != nil, !importing else { return }
        collection = nil
        restartAnkiConnect()
        folder = nil
        web.collectionRoot = nil
        UserDefaults.standard.removeObject(forKey: Self.folderKey)
        deckNames = []
        noteTypes = []
        warnings = []
        today = TodayStats(reviewed: 0, newStudied: 0, newAvailable: 0, dueAvailable: 0, learning: 0, studySecs: 0, secsLeft: nil)
        study = .none
        timer.reset()
        canUndoAnswer = false
        infoCard = nil
        infoCardID = nil
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
        loadConfig()
        perform { _ = try collection?.reload() }
        refresh()
    }

    /// Read the config file again. A broken file keeps the keys already in use.
    private func loadConfig() {
        do {
            let config = try AmgiCore.loadConfig()
            update(\.answerKeys, config.keys)
            configError = nil
        } catch {
            let message = error.localizedDescription
            if message != configError { errorMessage = message }
            configError = message
        }
    }

    func refresh() {
        guard let collection else { return }
        // Only changed values are assigned, so views that read the rest are left alone:
        // a reload when the app comes to the front usually changes nothing.
        perform {
            update(\.deckNames, try collection.decks().map { DeckName(name: $0.name, title: $0.title, noteType: $0.noteType) })
            update(\.noteTypes, collection.noteTypes())
            update(\.today, try collection.today(scope: .all))
            update(\.warnings, collection.warnings())
            update(\.desiredRetention, Double(collection.desiredRetention()))
        }
        fonts.fetch(for: collection.scripts(), han: EnvironmentPayload.preferredHan)
        loadCardInfo()
        // `amgi.toml` may have changed the schedule, so a revealed card's intervals too.
        if current == nil || !revealed { loadNext() } else { refreshIntervals() }
    }

    /// Cards come due, and the study day rolls over, while the app is in the background:
    /// keep the menu bar's counts current without touching the card being studied.
    private func refreshCountsPeriodically() {
        countsTimer = Timer.scheduledTimer(withTimeInterval: 5 * 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshCounts() }
        }
        countsTimer?.tolerance = 60
    }

    private func refreshCounts() {
        guard let collection else { return }
        perform { update(\.today, try collection.today(scope: .all)) }
    }

    func showCardInfo(id: String) {
        infoCardID = id
        loadCardInfo()
        openCardInfoWindow()
    }

    /// Keeps showing a card that leaves the collection, as it was last seen.
    private func loadCardInfo() {
        guard let collection, let id = infoCardID else { return }
        perform {
            if let info = try collection.cardInfo(cardId: id) { update(\.infoCard, info) }
        }
    }

    // MARK: Web view

    /// What the page shows. Read while rendering the window, so any change here updates
    /// the page.
    func webState(autoplay: Bool, progress: Bool) -> WebState {
        WebState(
            environment: .current(answerKeys: answerKeys),
            fonts: fonts.stylesheets,
            study: StudyPayload(
                card: current,
                revealed: revealed,
                autoplay: autoplay,
                instant: studyInput == .key,
                progress: progress,
                reviewed: today.reviewed,
                remaining: totalLeft,
                leech: leechNotice
            )
        )
    }

    /// ⌘S suspends the card being studied, ⌘Z takes back the last answer and ⌘I shows
    /// Card Info for the card. Caught before any view, so they work whether the web view,
    /// the header or nothing has focus.
    private func monitorReviewShortcuts() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            let flags = event.modifierFlags.intersection([.command, .control, .option, .shift])
            let key = event.charactersIgnoringModifiers?.lowercased()
            let window = event.window
            let handled = MainActor.assumeIsolated { () -> Bool in
                guard let self, flags == .command, let window, window.attachedSheet == nil,
                      window === self.web.webView.window
                else { return false }
                if key == "i", let id = self.current?.id {
                    self.showCardInfo(id: id)
                    return true
                }
                // A text field keeps ⌘Z for its own undo.
                if key == "z", !(window.firstResponder is NSText) {
                    self.undoAnswer()
                    return true
                }
                guard key == "s", self.current != nil else { return false }
                if !event.isARepeat { self.suspendCurrent() }
                return true
            }
            return handled ? nil : event
        }
    }

    /// Tags the card being studied `#suspended` in its file, and brings up the next one.
    func suspendCurrent() {
        guard let collection, let card = current else { return }
        perform { _ = try collection.setSuspended(cardIds: [card.id], suspended: true) }
        study = .none
        refresh()
    }

    /// The review header's More menu, opened under its button.
    func showMoreMenu() {
        guard let card = current else { return }
        let menu = NSMenu()
        let suspend = ActionMenuItem("Suspend Card", keyEquivalent: "s") { [weak self] in
            self?.suspendCurrent()
        }
        suspend.image = NSImage(systemSymbolName: "pause.circle", accessibilityDescription: nil)
        suspend.toolTip = card.kind == .cloze
            ? "Suspend Card (⌘S): tags it #suspended, so none of this cloze's gaps come up"
            : "Suspend Card (⌘S): tags it #suspended, so it doesn't come up"
        menu.addItem(suspend)
        let info = ActionMenuItem("Card Info", keyEquivalent: "i") { [weak self] in
            self?.showCardInfo(id: card.id)
        }
        info.image = NSImage(systemSymbolName: "info.circle", accessibilityDescription: nil)
        menu.addItem(info)
        if canUndoAnswer {
            let undo = ActionMenuItem("Undo Answer", keyEquivalent: "z") { [weak self] in
                self?.undoAnswer()
            }
            undo.image = NSImage(systemSymbolName: "arrow.uturn.backward", accessibilityDescription: nil)
            menu.addItem(undo)
        }
        if let view = moreMenuAnchor, view.window != nil {
            let below = NSPoint(x: 0, y: view.isFlipped ? view.bounds.maxY + 6 : -6)
            menu.popUp(positioning: nil, at: below, in: view)
        } else {
            // The button isn't showing, so open at the pointer instead.
            menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
        }
    }

    func reveal(via input: StudyInput) {
        guard case .question(let card) = study else { return }
        studyInput = input
        withAnimation(input.animation) { study = .answer(card) }
    }

    func answer(_ rating: Rating, via input: StudyInput) {
        guard let collection, case .answer(let card) = study else { return }
        studyInput = input
        let taken = timer.answer(card.id)
        withAnimation(input.animation) {
            var leech: Leech?
            perform { leech = try collection.answer(cardId: card.id, rating: rating, takenMs: taken) }
            leechNotice = leech.map {
                LeechNotice(id: (leechNotice?.id ?? 0) + 1, lapses: $0.lapses, suspended: $0.suspended)
            }
            canUndoAnswer = collection.canUndoAnswer()
            study = .none
            refresh()
        }
    }

    /// Take back the last answer and show that card again with its answer, ready to
    /// be rated anew. Answers can be undone one after another, latest first.
    func undoAnswer() {
        guard let collection, canUndoAnswer else {
            NSSound.beep()
            return
        }
        var card: StudyCard?
        perform { card = try collection.undoAnswer() }
        canUndoAnswer = collection.canUndoAnswer()
        leechNotice = nil
        timer.undo(card?.id)
        // A card no longer in its file can't be shown, so the next one comes up instead.
        study = card.map(StudyState.answer) ?? .none
        refresh()
    }

    private func handle(_ message: WebMessage) {
        switch message {
        case .reveal(let input): reveal(via: input)
        case .answer(let rating, let input): answer(rating, via: input)
        case .ready: break
        }
    }

    func loadNext() {
        guard let collection else { return }
        perform {
            update(\.study, try collection.nextCard(scope: .all).map(StudyState.question) ?? .none)
        }
    }

    /// Recompute the answer button intervals of the card on screen, keeping it revealed.
    private func refreshIntervals() {
        guard let collection, let card = current else { return }
        perform {
            if let next = try collection.nextCard(scope: .all), next.id == card.id {
                update(\.study, revealed ? .answer(next) : .question(next))
            }
        }
    }

    // MARK: Adding cards

    /// Opens Add Card, or puts it away if it's showing.
    func toggleQuickAdd() {
        if quickAdd.isShown { quickAdd.close() } else { quickAdd.show(model: self) }
    }

    /// Opens the command palette over the front window, or puts it away if it's showing.
    func toggleCommandPalette() {
        if commandPalette.isShown { commandPalette.close() } else { commandPalette.show(paletteCommands()) }
    }

    /// Appends the card to its deck's file. Returns why it couldn't be added, if it couldn't.
    func addCard(_ card: NewCard) -> String? {
        guard let collection else { return "Open a folder first." }
        do {
            _ = try collection.addCard(card: card)
        } catch AmgiError.InvalidCard(let message) {
            return message
        } catch {
            return error.localizedDescription
        }
        refresh()
        return nil
    }

    /// Appends the note to its deck's file. Returns why it couldn't be added, if it couldn't.
    func addNote(_ note: NewNote) -> String? {
        guard let collection else { return "Open a folder first." }
        do {
            _ = try collection.addNote(note: note)
        } catch AmgiError.InvalidCard(let message) {
            return message
        } catch {
            return error.localizedDescription
        }
        refresh()
        return nil
    }

    /// How many cards a note of `noteType` would make; 0 if it can't be added as it is.
    func noteCardCount(noteType: String, values: [String]) -> Int {
        Int(collection?.noteCardCount(noteType: noteType, values: values) ?? 0)
    }

    /// Copies a dropped image or sound into the collection's media folder, returning
    /// the Markdown that shows it in a card of `deck`, or why it couldn't.
    func addMedia(deck: String, file: URL) -> Result<String, Error> {
        guard let collection else { return .failure(CocoaError(.fileNoSuchFile)) }
        return Result { try collection.addMedia(deck: deck, path: file.path) }
    }

    // MARK: AnkiConnect

    /// Answer AnkiConnect requests for the open folder while the setting is on.
    private func restartAnkiConnect() {
        ankiConnectServer?.stop()
        ankiConnectServer = nil
        ankiConnectError = nil
        guard ankiConnect, let collection else { return }
        do {
            ankiConnectServer = try AnkiConnectServer.start(
                collection: collection, port: Self.ankiConnectPort, delegate: AnkiConnectEvents(model: self))
        } catch AmgiError.AnkiConnect(let message) {
            ankiConnectError = message
        } catch {
            ankiConnectError = error.localizedDescription
        }
    }

    private func registerQuickAddShortcut() {
        quickAddHotKey?.unregister()
        quickAddHotKey = nil
        guard quickAddShortcut else { return }
        quickAddHotKey = GlobalHotKey(keyCode: kVK_ANSI_N, modifiers: optionKey | cmdKey) { [weak self] in
            self?.toggleQuickAdd()
        }
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

extension AnswerKeys {
    /// Again, Hard, Good and Easy's keys, in that order.
    var ordered: [String] { [again, hard, good, easy] }
}

/// The card being studied: its question, or its answer too.
enum StudyState: Equatable {
    case none
    case question(StudyCard)
    case answer(StudyCard)

    var card: StudyCard? {
        switch self {
        case .none: nil
        case .question(let card), .answer(let card): card
        }
    }

    var revealed: Bool {
        if case .answer = self { true } else { false }
    }
}

/// What showed an answer or rated a card. Study keys are pressed hundreds of times a
/// session, so what they change shows at once; a click animates.
enum StudyInput: String {
    case key
    case pointer

    var animation: Animation? {
        self == .pointer ? .smooth(duration: 0.25) : nil
    }
}

/// Passes on what the AnkiConnect server did, from its threads to the app.
private final class AnkiConnectEvents: AnkiConnectDelegate, @unchecked Sendable {
    // Only set in `init`, and only read on the main actor.
    private weak var model: AppModel?

    init(model: AppModel) {
        self.model = model
    }

    func cardsChanged() {
        Task { @MainActor in self.model?.refresh() }
    }

    func show(path: String) {
        Task { @MainActor in self.model?.revealInEditor(path: path) }
    }
}
