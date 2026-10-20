import AmgiCore
import SwiftUI

/// Every tag, opened from the Tags header in the sidebar. Clicking one browses its cards.
struct TagList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.tags.isEmpty {
            ContentUnavailableView("No Tags", systemImage: "number", description: Text("Add #tags to cards to group them across decks."))
        } else {
            List(model.tags, id: \.name) { tag in
                LinkRow {
                    model.selection = .tag(tag.name)
                } icon: {
                    if let icon = model.icon(forTag: tag.name) { Text(icon) } else { Image(systemName: "number") }
                } title: {
                    Text(tag.name)
                } detail: {
                    Text(tag.cards == 1 ? "1 card" : "\(tag.cards) cards")
                        .monospacedDigit()
                }
            }
        }
    }
}

/// Every saved view, opened from the Saved Views header in the sidebar. Clicking one
/// browses its cards.
struct SavedViewList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.savedViews.isEmpty {
            ContentUnavailableView(
                "No Saved Views",
                systemImage: "line.3.horizontal.decrease.circle",
                description: Text("Search while browsing cards, then save the search.")
            )
        } else {
            List(model.savedViews, id: \.id) { view in
                LinkRow {
                    model.selection = .view(view.id)
                } icon: {
                    if let icon = view.icon { Text(icon) } else { Image(systemName: "line.3.horizontal.decrease.circle") }
                } title: {
                    Text(view.name)
                } detail: {
                    Text(view.query)
                        .font(.callout.monospaced())
                }
            }
        }
    }
}

/// A full-width row that opens something when clicked, highlighted under the pointer.
private struct LinkRow<Icon: View, Title: View, Detail: View>: View {
    let action: () -> Void
    @ViewBuilder let icon: Icon
    @ViewBuilder let title: Title
    @ViewBuilder let detail: Detail
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                icon
                    .frame(width: 20)
                    .foregroundStyle(.secondary)
                title
                    .lineLimit(1)
                Spacer(minLength: 16)
                detail
                    .lineLimit(1)
                    .foregroundStyle(.secondary)
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.vertical, 6)
            .padding(.horizontal, 8)
            .contentShape(.rect)
            .background(.quaternary.opacity(hovering ? 1 : 0), in: .rect(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}
