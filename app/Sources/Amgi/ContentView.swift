import AmgiCore
import SwiftUI

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @State private var showingWarnings = false

    var body: some View {
        @Bindable var model = model
        Group {
            if model.folder == nil {
                ContentUnavailableView {
                    Label("No Collection", systemImage: "rectangle.stack")
                } description: {
                    Text("Open a folder of Markdown files with `Q:` / `A:` and `C:` cards.")
                } actions: {
                    Button("Open Folder…") { model.chooseFolder() }
                        .buttonStyle(.borderedProminent)
                }
            } else {
                NavigationSplitView {
                    DeckList()
                        .navigationSplitViewColumnWidth(min: 200, ideal: 240)
                } detail: {
                    StudyView()
                }
            }
        }
        .toolbar {
            if !model.warnings.isEmpty {
                ToolbarItem {
                    Button {
                        showingWarnings.toggle()
                    } label: {
                        Label("\(model.warnings.count) Warnings", systemImage: "exclamationmark.triangle")
                    }
                    .help("Problems found while parsing cards")
                    .popover(isPresented: $showingWarnings) { WarningList() }
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
    }
}

private struct DeckList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selectedDeck) {
            Section {
                DeckRow(
                    title: "All Cards",
                    systemImage: "tray.full",
                    due: model.totalDue,
                    new: model.decks.map(\.new).max() ?? 0
                )
                .tag(String?.none)
            }
            Section(model.folder?.lastPathComponent ?? "Decks") {
                ForEach(model.decks, id: \.name) { deck in
                    DeckRow(title: deck.name, systemImage: "doc.text", due: deck.due, new: deck.new)
                        .tag(String?.some(deck.name))
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            Text("Today: \(model.today.reviewed) reviews, \(model.today.newStudied) new")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
                .padding(8)
        }
    }
}

private struct DeckRow: View {
    let title: String
    let systemImage: String
    let due: UInt32
    let new: UInt32

    var body: some View {
        HStack {
            Label(title, systemImage: systemImage)
            Spacer()
            if new > 0 {
                Text("\(new)").foregroundStyle(.blue)
            }
            if due > 0 {
                Text("\(due)").foregroundStyle(.green)
            }
        }
        .monospacedDigit()
    }
}

private struct WarningList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(Array(model.warnings.enumerated()), id: \.offset) { _, warning in
            VStack(alignment: .leading) {
                Text(warning.message)
                Text("\(warning.path):\(warning.line)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: 360, height: 240)
    }
}
