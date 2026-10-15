import AmgiCore
import SwiftUI

struct ContentView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if model.folder == nil {
                WelcomeView()
            } else {
                NavigationSplitView {
                    Sidebar()
                        .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
                } detail: {
                    switch model.selection {
                    case .dashboard: DashboardView()
                    case .review: StudyView()
                    case .deck, .tag: BrowseView()
                    }
                }
            }
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.errorMessage ?? "")
        }
        .alert(
            "Anki Import Finished",
            isPresented: Binding(get: { model.importSummary != nil }, set: { if !$0 { model.importSummary = nil } }),
            presenting: model.importSummary
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { summary in
            Text(summary.message)
        }
    }
}

private struct WelcomeView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 20) {
            VStack(spacing: 6) {
                Text("Amgi")
                    .font(.largeTitle.weight(.semibold))
                Text("Open a folder of Markdown flash cards.")
                    .foregroundStyle(.secondary)
            }
            Button("Open Folder…") { model.chooseFolder() }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                .keyboardShortcut("o")
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct Sidebar: View {
    @Environment(AppModel.self) private var model
    @State private var showingWarnings = false
    @State private var expanded: Set<String> = []
    @AppStorage("decksExpanded") private var decksExpanded = true
    @AppStorage("tagsExpanded") private var tagsExpanded = true

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selection) {
            Label("Dashboard", systemImage: "chart.bar.xaxis")
                .tag(SidebarItem.dashboard)
            Section("Review") {
                ReviewRow()
                    .tag(SidebarItem.review)
            }
            // Sections collapse from the chevron shown when hovering the header.
            if !model.deckNames.isEmpty {
                Section("Decks", isExpanded: $decksExpanded) {
                    ForEach(DeckNode.tree(decks: model.deckNames)) { node in
                        DeckTree(node: node, expanded: $expanded)
                    }
                }
            }
            if !model.tags.isEmpty {
                Section("Tags", isExpanded: $tagsExpanded) {
                    ForEach(model.tags, id: \.self) { tag in
                        TagRow(tag: tag)
                    }
                }
            }
        }
        // The scroll bar appearing re-tiles the list and cancels its expand and collapse
        // animations, so the sidebar scrolls without one.
        .scrollIndicators(.never)
        .safeAreaInset(edge: .bottom) {
            ReviewedToday()
        }
        .toolbar {
            if !model.warnings.isEmpty {
                ToolbarItem {
                    Button {
                        showingWarnings.toggle()
                    } label: {
                        Label("Warnings", systemImage: "exclamationmark.triangle")
                    }
                    .help("\(model.warnings.count) problems found while reading cards")
                    .popover(isPresented: $showingWarnings, arrowEdge: .bottom) { WarningList() }
                }
            }
        }
    }
}

// Rows that show counts read them in views of their own, so an answer re-renders those
// rows rather than the whole list.

private struct ReviewRow: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Label(model.folder?.lastPathComponent ?? "All Cards", systemImage: "rectangle.stack")
            .lineLimit(1)
            .badge(Int(model.totalDue))
    }
}

private struct ReviewedToday: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Text("\(model.today.reviewed) reviewed today")
            .font(.caption)
            .foregroundStyle(.tertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
    }
}

/// A deck in the sidebar: a deck file, a folder, or both (`lang.md` and `lang/`).
private struct DeckNode: Identifiable {
    let id: String
    let title: String
    let children: [DeckNode]

    /// The top-level decks.
    static func tree(decks: [DeckName]) -> [DeckNode] {
        // `decks` lists parents before children, so grouping keeps file order.
        let byParent = Dictionary(grouping: decks) { deck in
            deck.name.split(separator: "/").dropLast().joined(separator: "/")
        }
        func children(of parent: String) -> [DeckNode] {
            (byParent[parent] ?? []).map { deck in
                DeckNode(id: deck.name, title: deck.title, children: children(of: deck.name))
            }
        }
        return children(of: "")
    }
}

private struct DeckTree: View {
    let node: DeckNode
    @Binding var expanded: Set<String>

    var body: some View {
        if node.children.isEmpty {
            row
        } else {
            DisclosureGroup(isExpanded: isExpanded) {
                ForEach(node.children) { child in
                    DeckTree(node: child, expanded: $expanded)
                }
            } label: {
                row
            }
        }
    }

    private var row: some View {
        Label(node.title, systemImage: node.children.isEmpty ? "rectangle.portrait.on.rectangle.portrait" : "folder")
            .lineLimit(1)
            .tag(SidebarItem.deck(node.id))
    }

    private var isExpanded: Binding<Bool> {
        Binding(
            get: { expanded.contains(node.id) },
            set: { open in
                if open { expanded.insert(node.id) } else { expanded.remove(node.id) }
            }
        )
    }
}

private struct TagRow: View {
    @Environment(AppModel.self) private var model
    let tag: String
    @State private var choosingIcon = false

    var body: some View {
        let icon = model.icon(forTag: tag)
        Label {
            Text(tag)
        } icon: {
            if let icon { Text(icon) } else { Image(systemName: "number") }
        }
        .lineLimit(1)
        .tag(SidebarItem.tag(tag))
        .contextMenu {
            Button("Choose Icon…") { choosingIcon = true }
            if icon != nil {
                Button("Remove Icon") { model.setIcon(nil, forTag: tag) }
            }
        }
        .popover(isPresented: $choosingIcon, arrowEdge: .trailing) {
            // Popovers are hosted outside the window's view tree.
            TagIconPicker(tag: tag)
                .environment(model)
        }
    }
}

/// Common emoji to pick from, with the system emoji picker for everything else.
private struct TagIconPicker: View {
    private static let suggestions = Array("📚📝🧠💡🔬🧪🧮📐🌍🗺️🏛️🎨🎵💻⚙️🩺💊🌱🐾🍎⭐️🔥❤️✅")
        .map(String.init)

    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let tag: String
    @State private var custom = ""
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            LazyVGrid(columns: Array(repeating: GridItem(.fixed(28), spacing: 4), count: 8), spacing: 4) {
                ForEach(Self.suggestions, id: \.self) { emoji in
                    Button {
                        choose(emoji)
                    } label: {
                        Text(emoji)
                            .font(.title2)
                            .frame(width: 28, height: 28)
                            .background(
                                model.icon(forTag: tag) == emoji ? AnyShapeStyle(.selection) : AnyShapeStyle(.clear),
                                in: .rect(cornerRadius: 6)
                            )
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
            }
            HStack {
                TextField("Other emoji", text: $custom)
                    .focused($fieldFocused)
                    .frame(width: 110)
                    .onChange(of: custom) {
                        // Keep only the last character typed or picked.
                        if let last = custom.last { choose(String(last)) }
                    }
                Button {
                    fieldFocused = true
                    NSApp.orderFrontCharacterPalette(nil)
                } label: {
                    Image(systemName: "face.smiling")
                }
                .help("Show Emoji & Symbols")
                Spacer()
                Button("Remove") {
                    model.setIcon(nil, forTag: tag)
                    dismiss()
                }
                .disabled(model.icon(forTag: tag) == nil)
            }
        }
        .padding(14)
    }

    private func choose(_ icon: String) {
        model.setIcon(icon, forTag: tag)
        dismiss()
    }
}

private struct WarningList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(Array(model.warnings.enumerated()), id: \.offset) { _, warning in
            VStack(alignment: .leading, spacing: 2) {
                Text(warning.message)
                Text("\(warning.path):\(warning.line)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
        }
        .scrollContentBackground(.hidden)
        .frame(width: 360, height: 220)
    }
}
