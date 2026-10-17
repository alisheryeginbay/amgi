import AmgiCore
import Foundation

// What the web panes are sent, as JSON. Field names match `web/src/lib/types.ts`.

/// Everything the panes show, read from the model. Parts are compared with what was
/// sent last, and only changed ones are encoded and sent again.
struct WebState {
    var pane: WebPane
    var environment: EnvironmentPayload
    var icons: [String: String]
    var study: StudyPayload
    var dashboard: DashboardPayload?
    /// Only while browsing, so other panes never copy the card list.
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

    static var current: EnvironmentPayload {
        EnvironmentPayload(
            locale: Locale.current.identifier(.bcp47),
            firstWeekday: Calendar.current.firstWeekday - 1
        )
    }
}

struct StudyPayload: Encodable, Equatable {
    let card: StudyCard?
    let revealed: Bool
    let autoplay: Bool

    enum CodingKeys: CodingKey { case card, revealed, autoplay }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(card.map(CardPayload.init), forKey: .card)
        try c.encode(revealed, forKey: .revealed)
        try c.encode(autoplay, forKey: .autoplay)
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

/// The cards browsed, kept as the core's records so comparing them is cheap; they
/// are only converted when they change.
struct BrowseSource: Equatable {
    /// Changes when another deck or tag is browsed.
    let scope: String
    let cards: [BrowseCard]
}

struct BrowsePayload: Encodable {
    let scope: String
    let cards: [CardPayload]

    init(_ source: BrowseSource) {
        scope = source.scope
        cards = source.cards.map(CardPayload.init)
    }
}

/// A card to show, from study or browsing. Fields a card lacks are left out.
struct CardPayload: Encodable {
    let id: String
    let deck: String
    let tags: [String]
    let path: String
    let line: UInt32
    let kind: String
    let front: String
    let back: String
    var due: Int64?
    var reps: UInt32?
    var lapses: UInt32?
    var suspended: Bool?

    enum CodingKeys: CodingKey { case id, deck, tags, path, line, kind, front, back, due, reps, lapses, suspended }

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

    init(_ card: BrowseCard) {
        id = card.id
        deck = card.deck
        tags = card.tags
        path = card.path
        line = card.line
        kind = card.kind.name
        front = card.front
        back = card.back
        due = card.due
        reps = card.reps
        lapses = card.lapses
        suspended = card.suspended
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(deck, forKey: .deck)
        try c.encode(tags, forKey: .tags)
        try c.encode(path, forKey: .path)
        try c.encode(line, forKey: .line)
        try c.encode(kind, forKey: .kind)
        try c.encode(front, forKey: .front)
        try c.encode(back, forKey: .back)
        // Browse cards send `null` for a new card's due date; study cards leave all three out.
        if reps != nil {
            try c.encode(due, forKey: .due)
            try c.encode(reps, forKey: .reps)
            try c.encode(lapses, forKey: .lapses)
            try c.encode(suspended, forKey: .suspended)
        }
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
    case reveal
    case answer(Rating)
    /// Open a card's deck file.
    case open(path: String)
    case select(ids: [String])
    /// The cards a right-click acts on.
    case contextCards(ids: [String])

    init?(_ body: Any) {
        guard let body = body as? [String: Any], let type = body["type"] as? String else { return nil }
        switch type {
        case "ready": self = .ready
        case "reveal": self = .reveal
        case "answer":
            guard let n = body["rating"] as? Int, Rating.allCases.indices.contains(n - 1) else { return nil }
            self = .answer(Rating.allCases[n - 1])
        case "open":
            guard let path = body["path"] as? String else { return nil }
            self = .open(path: path)
        case "select":
            self = .select(ids: body["ids"] as? [String] ?? [])
        case "contextCards":
            self = .contextCards(ids: body["ids"] as? [String] ?? [])
        default:
            return nil
        }
    }
}
