import AmgiCore
import SwiftUI

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if model.folder == nil {
                WelcomeView()
            } else {
                NavigationSplitView {
                    Sidebar()
                        .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
                } detail: {
                    DetailView()
                }
            }
        }
        .onAppear {
            model.openCardInfoWindow = { [openWindow] in openWindow(id: "card-info") }
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
        .sheet(item: Binding(get: { model.editingView }, set: { model.editingView = $0 })) { draft in
            SavedViewEditor(draft: draft)
                .environment(model)
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
    @AppStorage("viewsExpanded") private var viewsExpanded = true

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
            if !model.savedViews.isEmpty {
                Section("Saved Views", isExpanded: $viewsExpanded) {
                    ForEach(model.savedViews, id: \.id) { view in
                        SavedViewRow(view: view)
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
            EmojiPicker(current: icon) { model.setIcon($0, forTag: tag) }
        }
    }
}

private struct SavedViewRow: View {
    @Environment(AppModel.self) private var model
    let view: SavedView
    @State private var choosingIcon = false

    var body: some View {
        Label {
            Text(view.name)
        } icon: {
            if let icon = view.icon { Text(icon) } else { Image(systemName: "line.3.horizontal.decrease.circle") }
        }
        .lineLimit(1)
        .help(view.query)
        .tag(SidebarItem.view(view.id))
        .contextMenu {
            Button("Edit View…") { model.editSavedView(view) }
            Button("Choose Icon…") { choosingIcon = true }
            if view.icon != nil {
                Button("Remove Icon") { model.setIcon(nil, for: view) }
            }
            Divider()
            Button("Delete View") { model.deleteSavedView(view) }
        }
        .popover(isPresented: $choosingIcon, arrowEdge: .trailing) {
            EmojiPicker(current: view.icon) { model.setIcon($0, for: view) }
        }
    }
}

/// Names a saved view and its search.
private struct SavedViewEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State var draft: SavedViewDraft

    private var complete: Bool {
        !draft.name.trimmingCharacters(in: .whitespaces).isEmpty
            && !draft.query.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(draft.id == nil ? "New Saved View" : "Edit Saved View")
                .font(.headline)
            Form {
                TextField("Name:", text: $draft.name, prompt: Text("Due this week"))
                TextField("Search:", text: $draft.query, prompt: Text("due<7 -is:suspended"))
                    .font(.body.monospaced())
            }
            Text("Searches work as in the browse search field, such as `#verbs`, `deck:french`, `is:due` or `lapses>3`.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(draft.id == nil ? "Add" : "Save") {
                    model.commit(draft)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!complete)
            }
        }
        .padding(20)
        .frame(width: 400)
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
