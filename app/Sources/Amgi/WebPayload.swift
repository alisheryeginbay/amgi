import AmgiCore
import Foundation

// What the web panes are sent, as JSON. Field names match `web/src/lib/types.ts`.

/// Everything the panes show, read from the model. Parts are compared with what was
/// sent last, and only changed ones are encoded and sent again.
struct WebState {
    var pane: WebPane
    var environment: EnvironmentPayload
    /// Stylesheets of the CJK font packs installed, as `amgi://fonts/` URLs.
    var fonts: [String]
    var icons: [String: String]
    var study: StudyPayload
    var dashboard: DashboardPayload?
    /// Every card as JSON, once read.
    var cards: String?
    /// The note types as JSON, once read.
    var noteTypes: String?
    /// Which cards to show, while browsing.
    var browse: BrowseSource?
    var search: String
    /// Whether browsing shows the selected card beside the table.
    var preview: Bool
}

enum WebPane: String, Encodable {
    case study, dashboard, browse
}

struct EnvironmentPayload: Encodable, Equatable {
    let locale: String
    /// 0 for Sunday through 6 for Saturday.
    let firstWeekday: Int
    /// The Mac's first CJK language: `ko`, `ja`, `zh-hans` or `zh-hant`. Chinese, Japanese
    /// and Korean share Han characters but draw some differently, and cards don't say
    /// their language, so this picks the forms. Left out of the JSON when there is none.
    let han: String?

    static var current: EnvironmentPayload {
        EnvironmentPayload(
            locale: Locale.current.identifier(.bcp47),
            firstWeekday: Calendar.current.firstWeekday - 1,
            han: preferredHan
        )
    }

    static var preferredHan: String? {
        Locale.preferredLanguages.lazy.compactMap { tag -> String? in
            let tag = tag.lowercased()
            if tag.hasPrefix("ja") { return "ja" }
            if ["zh-hant", "zh-tw", "zh-hk", "zh-mo"].contains(where: tag.hasPrefix) { return "zh-hant" }
            if tag.hasPrefix("zh") { return "zh-hans" }
            if tag.hasPrefix("ko") { return "ko" }
            return nil
        }.first
    }
}

struct StudyPayload: Encodable, Equatable {
    let card: StudyCard?
    let revealed: Bool
    let autoplay: Bool
    /// Whether a key made the last change, which the page then shows without animating.
    let instant: Bool
    /// Whether the progress bar shows.
    let progress: Bool
    /// Answers given today, and cards still to study today: due, learning and new.
    let reviewed: UInt32
    let remaining: UInt32

    enum CodingKeys: CodingKey { case card, revealed, autoplay, instant, progress, reviewed, remaining }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(card.map(CardPayload.init), forKey: .card)
        try c.encode(revealed, forKey: .revealed)
        try c.encode(autoplay, forKey: .autoplay)
        try c.encode(instant, forKey: .instant)
        try c.encode(progress, forKey: .progress)
        try c.encode(reviewed, forKey: .reviewed)
        try c.encode(remaining, forKey: .remaining)
    }
}

struct DashboardPayload: Encodable, Equatable {
    let due: UInt32
    let newToday: UInt32
    let reviewedToday: UInt32
    let streak: UInt32
    let activity: [UInt32]
    let activityStart: Int64
    let forecast: [UInt32]
    let newCards: UInt32
    let youngCards: UInt32
    let matureCards: UInt32
    let suspendedCards: UInt32
    let retention: Double?

    init(_ d: Dashboard, due: UInt32, today: TodayStats) {
        self.due = due
        newToday = today.newAvailable
        reviewedToday = today.reviewed
        streak = d.streak
        activity = d.activity
        activityStart = d.activityStart
        forecast = d.forecast
        newCards = d.newCards
        youngCards = d.youngCards
        matureCards = d.matureCards
        suspendedCards = d.suspendedCards
        retention = d.retention
    }
}

/// Which of the cards the browse pane shows.
struct BrowseSource: Encodable, Equatable {
    /// Changes when another deck, tag or saved view is browsed.
    let scope: String
    /// Only this deck and its subdecks, if set. Left out of the JSON when `nil`.
    let deck: String?
    /// The tag's or saved view's search, run by the page before the one typed.
    let query: String
}

/// A card to study.
struct CardPayload: Encodable {
    let id: String
    let deck: String
    let tags: [String]
    let path: String
    let line: UInt32
    let kind: String
    let front: String
    let back: String

    init(_ card: StudyCard) {
        id = card.id
        deck = card.deck
        tags = card.tags
        path = card.path
        line = card.line
        kind = card.kind.name
        front = card.front
        back = card.back
    }
}

private extension CardKind {
    var name: String {
        switch self {
        case .basic: "basic"
        case .cloze: "cloze"
        }
    }
}

/// What a pane sends back.
enum WebMessage {
    case ready
    case reveal(StudyInput)
    case answer(Rating, StudyInput)
    /// Open a card's deck file.
    case open(path: String)
    case select(ids: [String])
    /// The cards a right-click acts on.
    case contextCards(ids: [String])
    /// How many cards match the browsed tag or saved view.
    case count(Int)
    /// Show a card in the Card Info window.
    case info(id: String)

    init?(_ body: Any) {
        guard let body = body as? [String: Any], let type = body["type"] as? String else { return nil }
        let input = (body["via"] as? String).flatMap(StudyInput.init) ?? .pointer
        switch type {
        case "ready": self = .ready
        case "reveal": self = .reveal(input)
        case "answer":
            guard let n = body["rating"] as? Int, Rating.allCases.indices.contains(n - 1) else { return nil }
            self = .answer(Rating.allCases[n - 1], input)
        case "open":
            guard let path = body["path"] as? String else { return nil }
            self = .open(path: path)
        case "select":
            self = .select(ids: body["ids"] as? [String] ?? [])
        case "contextCards":
            self = .contextCards(ids: body["ids"] as? [String] ?? [])
        case "info":
            guard let id = body["id"] as? String else { return nil }
            self = .info(id: id)
        case "count":
            guard let count = body["count"] as? Int else { return nil }
            self = .count(count)
        default:
            return nil
        }
    }
}
