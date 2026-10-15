import AmgiCore
import SwiftUI

/// The cards of a deck or tag, as a sortable, searchable table.
struct BrowseView: View {
    @Environment(AppModel.self) private var model
    @State private var search = ""
    @State private var selected: Set<BrowseCard.ID> = []

    var body: some View {
        let query = search.trimmingCharacters(in: .whitespaces)
        let cards = query.isEmpty ? model.browsed : model.browsed.filter { $0.matches(query) }
        CardTable(cards: cards, icons: model.tagIcons, selection: $selected) { card in
            model.revealInEditor(path: card.path)
        }
        .overlay {
            if cards.isEmpty && !query.isEmpty {
                ContentUnavailableView.search(text: search)
            }
        }
        .searchable(text: $search, placement: .toolbar, prompt: "Search Cards")
        .navigationTitle(title)
        .navigationSubtitle(model.browsed.count == 1 ? "1 card" : "\(model.browsed.count) cards")
        .toolbar {
            ToolbarItem {
                // Its own view, so changing the selection re-renders only the button.
                EditSelectedButton(selected: $selected)
            }
        }
        .onChange(of: model.selection) {
            search = ""
            selected = []
        }
    }

    private var title: String {
        switch model.selection {
        case .tag(let tag): model.icon(forTag: tag).map { "\($0) \(tag)" } ?? "#\(tag)"
        case .deck(let name): model.deckNames.first { $0.name == name }?.title ?? deckTitle(name)
        case .leeches: "Leeches"
        default: ""
        }
    }
}

private struct EditSelectedButton: View {
    @Environment(AppModel.self) private var model
    @Binding var selected: Set<BrowseCard.ID>

    var body: some View {
        let card = model.card(selectedFrom: selected)
        Button {
            if let card { model.revealInEditor(path: card.path) }
        } label: {
            Label("Edit Card", systemImage: "pencil")
        }
        .disabled(card == nil)
        .help(card.map { "Open \($0.path), line \($0.line)" } ?? "Edit Card")
    }
}

extension AppModel {
    /// The one browsed card selected, if exactly one is.
    func card(selectedFrom ids: Set<BrowseCard.ID>) -> BrowseCard? {
        guard ids.count == 1, let id = ids.first else { return nil }
        return browsed.first { $0.id == id }
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
