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
                Label(model.folder?.lastPathComponent ?? "All Cards", systemImage: "rectangle.stack")
                    .lineLimit(1)
                    .badge(Int(model.totalDue))
                    .tag(SidebarItem.review)
            }
            // Sections collapse from the chevron shown when hovering the header.
            if !model.decks.isEmpty {
                Section("Decks", isExpanded: $decksExpanded) {
                    ForEach(DeckNode.tree(decks: model.decks)) { node in
                        DeckTree(node: node, expanded: $expanded)
                    }
                }
            }
            if !model.tags.isEmpty {
                Section("Tags", isExpanded: $tagsExpanded) {
                    ForEach(model.tags, id: \.self) { tag in
                        Label(tag, systemImage: "number")
                            .lineLimit(1)
                            .tag(SidebarItem.tag(tag))
                    }
                }
            }
        }
        // The scroll bar appearing re-tiles the list and cancels its expand and collapse
        // animations, so the sidebar scrolls without one.
        .scrollIndicators(.never)
        .safeAreaInset(edge: .bottom) {
            Text("\(model.today.reviewed) reviewed today")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
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

/// A deck in the sidebar: a deck file, a folder, or both (`lang.md` and `lang/`).
private struct DeckNode: Identifiable {
    let id: String
    let title: String
    let children: [DeckNode]

    /// The top-level decks.
    static func tree(decks: [DeckInfo]) -> [DeckNode] {
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
