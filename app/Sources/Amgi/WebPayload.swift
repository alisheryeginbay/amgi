import AmgiCore
import Foundation

// What the page is sent, as JSON. Field names match `web/src/lib/types.ts`.

/// Everything the page shows, read from the model. Parts are compared with what was
/// sent last, and only changed ones are encoded and sent again.
struct WebState {
    var environment: EnvironmentPayload
    /// Stylesheets of the CJK font packs installed, as `amgi://fonts/` URLs.
    var fonts: [String]
    var study: StudyPayload
}

struct EnvironmentPayload: Encodable, Equatable {
    /// The Mac's first CJK language: `ko`, `ja`, `zh-hans` or `zh-hant`. Chinese, Japanese
    /// and Korean share Han characters but draw some differently, and cards don't say
    /// their language, so this picks the forms. Left out of the JSON when there is none.
    let han: String?
    /// The keys that rate Again, Hard, Good and Easy, lowercase.
    let answerKeys: [String]

    static func current(answerKeys: AnswerKeys) -> EnvironmentPayload {
        EnvironmentPayload(
            han: preferredHan,
            answerKeys: answerKeys.ordered
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
    /// The last answer made its card a leech.
    let leech: LeechNotice?

    enum CodingKeys: CodingKey { case card, revealed, autoplay, instant, progress, reviewed, remaining, leech }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(card.map(CardPayload.init), forKey: .card)
        try c.encode(revealed, forKey: .revealed)
        try c.encode(autoplay, forKey: .autoplay)
        try c.encode(instant, forKey: .instant)
        try c.encode(progress, forKey: .progress)
        try c.encode(reviewed, forKey: .reviewed)
        try c.encode(remaining, forKey: .remaining)
        try c.encode(leech, forKey: .leech)
    }
}

/// A card that the last answer made a leech; `id` is new for each, so the same
/// count twice still shows.
struct LeechNotice: Encodable, Equatable {
    let id: Int
    let lapses: UInt32
    let suspended: Bool
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
    /// `new`, `learning` or `due`.
    let stage: String
    let typed: String?
    let typedField: String?

    init(_ card: StudyCard) {
        id = card.id
        deck = card.deck
        tags = card.tags
        path = card.path
        line = card.line
        kind = card.kind.name
        front = card.front
        back = card.back
        stage = card.stage.name
        typed = card.typed
        typedField = card.typedField
    }
}

private extension StudyStage {
    var name: String {
        switch self {
        case .new: "new"
        case .learning: "learning"
        case .due: "due"
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
    case reveal(StudyInput)
    case answer(Rating, StudyInput)

    init?(_ body: Any) {
        guard let body = body as? [String: Any], let type = body["type"] as? String else { return nil }
        let input = (body["via"] as? String).flatMap(StudyInput.init) ?? .pointer
        switch type {
        case "ready": self = .ready
        case "reveal": self = .reveal(input)
        case "answer":
            guard let n = body["rating"] as? Int, Rating.allCases.indices.contains(n - 1) else { return nil }
            self = .answer(Rating.allCases[n - 1], input)
        default:
            return nil
        }
    }
}
