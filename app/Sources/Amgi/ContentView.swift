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
                    StudyView()
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

    /// Decks grouped by folder; top-level decks first.
    private var groups: [(folder: String, decks: [DeckInfo])] {
        let grouped = Dictionary(grouping: model.decks) { deck in
            deck.name.split(separator: "/").dropLast().joined(separator: "/")
        }
        return grouped.keys.sorted().map { ($0, grouped[$0]!) }
    }

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selectedDeck) {
            HStack {
                Label("All Cards", systemImage: "rectangle.stack")
                Spacer()
                DeckCounts(due: model.totalDue, new: model.today.newAvailable)
            }
            .tag(String?.none)

            ForEach(groups, id: \.folder) { group in
                Section(group.folder.isEmpty ? (model.folder?.lastPathComponent ?? "Decks") : group.folder) {
                    ForEach(group.decks, id: \.name) { deck in
                        HStack {
                            Text(deckTitle(deck.name))
                                .lineLimit(1)
                            Spacer()
                            DeckCounts(due: deck.due, new: deck.new)
                        }
                        .tag(String?.some(deck.name))
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
