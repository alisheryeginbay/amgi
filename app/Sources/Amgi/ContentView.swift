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
                    if model.selection == .dashboard {
                        DashboardView()
                    } else {
                        StudyView()
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
    /// Expanded decks; the collection itself starts open.
    @State private var expanded: Set<String> = [DeckNode.rootID]
    @AppStorage("tagsExpanded") private var tagsExpanded = true

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selection) {
            Label("Dashboard", systemImage: "chart.bar.xaxis")
                .tag(SidebarItem.dashboard)
            DeckTree(node: DeckNode.tree(model: model), expanded: $expanded)
            if !model.tags.isEmpty {
                // Collapses from the chevron shown when hovering the header.
                Section("Tags", isExpanded: $tagsExpanded) {
                    ForEach(model.tags, id: \.self) { tag in
                        Label(tag, systemImage: "number")
                            .lineLimit(1)
                            .tag(SidebarItem.tag(tag))
                    }
                }
            }
        }
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

/// A deck in the sidebar. The root is the whole collection; every other node is a
/// deck file, a folder, or both (`lang.md` and `lang/`).
private struct DeckNode: Identifiable {
    static let rootID = ""

    let id: String
    let title: String
    let children: [DeckNode]

    /// Selection value: `nil` studies the whole collection.
    var selection: String? { id == Self.rootID ? nil : id }

    @MainActor
    static func tree(model: AppModel) -> DeckNode {
        // `decks` lists parents before children, so grouping keeps file order.
        let byParent = Dictionary(grouping: model.decks) { deck in
            deck.name.split(separator: "/").dropLast().joined(separator: "/")
        }
        func children(of parent: String) -> [DeckNode] {
            (byParent[parent] ?? []).map { deck in
                DeckNode(id: deck.name, title: deck.title, children: children(of: deck.name))
            }
        }
        return DeckNode(
            id: rootID,
            title: model.folder?.lastPathComponent ?? "All Cards",
            children: children(of: "")
        )
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

    @ViewBuilder
    private var row: some View {
        if node.id == DeckNode.rootID {
            Label(node.title, systemImage: "rectangle.stack")
                .tag(SidebarItem.deck(node.selection))
        } else if !node.children.isEmpty {
            Label(node.title, systemImage: "folder")
                .lineLimit(1)
                .tag(SidebarItem.deck(node.selection))
        } else {
            Label(node.title, systemImage: "rectangle.portrait.on.rectangle.portrait")
                .lineLimit(1)
                .tag(SidebarItem.deck(node.selection))
        }
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
