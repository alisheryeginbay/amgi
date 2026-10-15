import AmgiCore
import SwiftUI

/// The cards of a deck or tag, as a sortable, searchable table.
struct BrowseView: View {
    @Environment(AppModel.self) private var model
    @State private var search = ""
    @State private var selected: Set<BrowseCard.ID> = []
    /// Empty keeps file order.
    @State private var sortOrder: [KeyPathComparator<BrowseCard>] = []

    private var cards: [BrowseCard] {
        let query = search.trimmingCharacters(in: .whitespaces)
        let matching = query.isEmpty ? model.browsed : model.browsed.filter { $0.matches(query) }
        return sortOrder.isEmpty ? matching : matching.sorted(using: sortOrder)
    }

    var body: some View {
        let cards = cards
        Table(cards, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("Question", value: \.front) { card in
                Text(inlineMarkdown(card.front))
            }
            TableColumn("Answer", value: \.back) { card in
                Text(inlineMarkdown(card.back))
                    .foregroundStyle(.secondary)
            }
            TableColumn("Deck", value: \.deck) { card in
                Text(deckTitle(card.deck))
                    .help(card.deck)
            }
            .width(ideal: 100)
            TableColumn("Tags") { card in
                Text(card.tags.map { "#\($0)" }.joined(separator: " "))
                    .foregroundStyle(.secondary)
            }
            .width(ideal: 80)
            TableColumn("Due", value: \.dueOrder) { card in
                if let due = card.due {
                    Text(Date(timeIntervalSince1970: TimeInterval(due)), format: .dateTime.day().month().year())
                        .monospacedDigit()
                } else {
                    Text("New")
                        .foregroundStyle(.tertiary)
                }
            }
            .width(ideal: 90)
            TableColumn("Reviews", value: \.reps) { card in
                Text("\(card.reps)")
                    .monospacedDigit()
            }
            .width(ideal: 56)
            TableColumn("Lapses", value: \.lapses) { card in
                Text("\(card.lapses)")
                    .monospacedDigit()
            }
            .width(ideal: 52)
        }
        .contextMenu(forSelectionType: BrowseCard.ID.self) { ids in
            if let card = card(ids) {
                Button("Edit Card") { model.revealInEditor(path: card.path) }
            }
        } primaryAction: { ids in
            if let card = card(ids) { model.revealInEditor(path: card.path) }
        }
        .overlay {
            if cards.isEmpty && !search.isEmpty {
                ContentUnavailableView.search(text: search)
            }
        }
        .searchable(text: $search, placement: .toolbar, prompt: "Search Cards")
        .navigationTitle(title)
        .navigationSubtitle(model.browsed.count == 1 ? "1 card" : "\(model.browsed.count) cards")
        .toolbar {
            ToolbarItem {
                let card = card(selected)
                Button {
                    if let card { model.revealInEditor(path: card.path) }
                } label: {
                    Label("Edit Card", systemImage: "pencil")
                }
                .disabled(card == nil)
                .help(card.map { "Open \($0.path), line \($0.line)" } ?? "Edit Card")
            }
        }
        .onChange(of: model.selection) {
            search = ""
            selected = []
        }
    }

    private var title: String {
        switch model.selection {
        case .tag(let tag): "#\(tag)"
        case .deck(let name): model.decks.first { $0.name == name }?.title ?? deckTitle(name)
        default: ""
        }
    }

    /// The one card selected, if exactly one is.
    private func card(_ ids: Set<BrowseCard.ID>) -> BrowseCard? {
        guard ids.count == 1, let id = ids.first else { return nil }
        return model.browsed.first { $0.id == id }
    }

    /// Card text on one line, with inline Markdown rendered.
    private func inlineMarkdown(_ source: String) -> AttributedString {
        let line = source.split(whereSeparator: \.isNewline).joined(separator: " ")
        return (try? AttributedString(markdown: line)) ?? AttributedString(line)
    }
}

extension BrowseCard: Identifiable {
    /// New cards sort after every due date.
    var dueOrder: Int64 { due ?? .max }

    func matches(_ query: String) -> Bool {
        front.localizedStandardContains(query)
            || back.localizedStandardContains(query)
            || tags.contains { $0.localizedStandardContains(query.trimmingPrefix("#")) }
    }
}
