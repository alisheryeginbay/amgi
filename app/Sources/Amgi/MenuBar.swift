import AmgiCore
import AppKit
import SwiftUI

/// Whether Amgi shows its due count in the menu bar.
let showMenuBarKey = "showMenuBar"

/// The menu bar item: a stack of cards, with the number due beside it.
struct MenuBarLabel: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let due = model.totalDue
        HStack(spacing: 3) {
            Image(systemName: due > 0 ? "rectangle.stack.fill" : "rectangle.stack")
            if due > 0 { Text("\(due)").monospacedDigit() }
        }
        .accessibilityLabel(due == 0 ? "Amgi: nothing due" : "Amgi: \(due) due")
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
                Button(model.totalDue + model.today.newAvailable > 0 ? "Study Now" : "Nothing Left Today") {
                    model.selection = .review
                    showMainWindow()
                }
                .disabled(model.totalDue + model.today.newAvailable == 0)
            }
            let decks = model.decks.filter { !$0.name.contains("/") && $0.due + $0.new > 0 }
            if !decks.isEmpty {
                Section("Decks") {
                    ForEach(decks, id: \.name) { deck in
                        Button(deck.title) {
                            model.selection = .deck(deck.name)
                            showMainWindow()
                        }
                        .badge(Text(counts(due: deck.due, new: deck.new)))
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
        Button("Open Amgi") { showMainWindow() }
        Button("Quit Amgi") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }

    private var summary: String {
        let due = model.totalDue
        let new = model.today.newAvailable
        if due == 0 && new == 0 { return "All caught up" }
        return [due > 0 ? "\(due) due" : nil, new > 0 ? "\(new) new" : nil]
            .compactMap { $0 }
            .joined(separator: " · ")
    }

    private var reviewedToday: String {
        let reviewed = model.today.reviewed
        return reviewed == 1 ? "1 review today" : "\(reviewed) reviews today"
    }

    private func counts(due: UInt32, new: UInt32) -> String {
        [due > 0 ? "\(due) due" : nil, new > 0 ? "\(new) new" : nil]
            .compactMap { $0 }
            .joined(separator: ", ")
    }

    private func showMainWindow() {
        openWindow(id: "main")
        NSApp.activate()
    }
}
