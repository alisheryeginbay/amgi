import AmgiCore
import AppKit
import SwiftUI

/// Whether Amgi shows its due count in the menu bar.
let showMenuBarKey = "showMenuBar"

/// The menu bar item: Amgi's flower, with the number due beside it.
struct MenuBarLabel: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let due = model.totalDue
        HStack(spacing: 3) {
            Image(nsImage: MenuBarIcon.image)
            if due > 0 { Text("\(due)").monospacedDigit() }
        }
        .accessibilityLabel(due == 0 ? "\(appName): nothing due" : "\(appName): \(due) due")
    }
}

/// What's left to study today, and a way back to the main window.
struct MenuBarMenu: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        if model.folder == nil {
            Text("No folder open")
            Button("Open Folder…") {
                showMainWindow()
                model.chooseFolder()
            }
        } else {
            Section {
                Text(summary)
                Text(reviewedToday)
            }
            Section {
                Button(model.totalLeft > 0 ? "Study Now" : "Nothing Left Today") {
                    model.selection = .review
                    showMainWindow()
                }
                .disabled(model.totalLeft == 0)
            }
            let decks = model.decks.filter { !$0.name.contains("/") && $0.due + $0.learning + $0.new > 0 }
            if !decks.isEmpty {
                Section("Decks") {
                    ForEach(decks, id: \.name) { deck in
                        Button(deck.title) {
                            model.selection = .deck(deck.name)
                            showMainWindow()
                        }
                        .badge(Text(counts(due: deck.due, learning: deck.learning, new: deck.new)))
                    }
                }
            }
            if !model.warnings.isEmpty {
                Section {
                    Text(model.warnings.count == 1 ? "1 problem in the deck files" : "\(model.warnings.count) problems in the deck files")
                }
            }
        }
        Divider()
        Button("Add Card…") { model.toggleQuickAdd() }
            .keyboardShortcut("n", modifiers: model.quickAddShortcut ? [.option, .command] : .command)
        Button("Open \(appName)") { showMainWindow() }
        Button("Quit \(appName)") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }

    private var summary: String {
        if model.totalLeft == 0 { return "All caught up" }
        return parts(due: model.totalDue, learning: model.totalLearning, new: model.today.newAvailable)
            .joined(separator: " · ")
    }

    private var reviewedToday: String {
        let reviewed = model.today.reviewed
        return reviewed == 1 ? "1 review today" : "\(reviewed) reviews today"
    }

    private func counts(due: UInt32, learning: UInt32, new: UInt32) -> String {
        parts(due: due, learning: learning, new: new).joined(separator: ", ")
    }

    private func parts(due: UInt32, learning: UInt32, new: UInt32) -> [String] {
        [due > 0 ? "\(due) due" : nil, learning > 0 ? "\(learning) learning" : nil, new > 0 ? "\(new) new" : nil]
            .compactMap { $0 }
    }

    private func showMainWindow() {
        openWindow(id: "main")
        NSApp.activate()
    }
}
