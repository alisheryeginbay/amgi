import SwiftUI

/// A shortcut as listed in the Keyboard Shortcuts window.
struct Shortcut: Identifiable {
    let action: String
    /// The keys pressed together, each drawn as its own key cap.
    let keys: [String]
    var id: String { action }

    init(_ action: String, _ keys: String...) {
        self.action = action
        self.keys = keys
    }
}

struct ShortcutGroup: Identifiable {
    let title: String
    let shortcuts: [Shortcut]
    var id: String { title }

    /// Every shortcut in the app. Keep in step with the menus, `AppModel`'s key monitor
    /// and the page's key handlers in `Study.svelte` and `Browse.svelte`.
    static let all = [
        ShortcutGroup(title: "General", shortcuts: [
            Shortcut("Open Folder", "⌘", "O"),
            Shortcut("Reload Cards", "⌘", "R"),
            Shortcut("Import Anki Deck", "⇧", "⌘", "I"),
            Shortcut("Settings", "⌘", ","),
            Shortcut("Keyboard Shortcuts", "⌘", "/"),
        ]),
        ShortcutGroup(title: "Study", shortcuts: [
            Shortcut("Show Answer", "Space"),
            Shortcut("Again", "1"),
            Shortcut("Hard", "2"),
            Shortcut("Good", "3"),
            Shortcut("Easy", "4"),
            Shortcut("Replay Audio", "R"),
            Shortcut("Suspend Card", "⌘", "S"),
            Shortcut("Card Info", "⌘", "I"),
            Shortcut("More", "⌘", "K"),
        ]),
        ShortcutGroup(title: "Browse", shortcuts: [
            Shortcut("Select Next or Previous Card", "↓", "↑"),
            Shortcut("Extend Selection", "⇧", "↓"),
            Shortcut("Select All", "⌘", "A"),
            Shortcut("Edit Card", "Return"),
            Shortcut("Card Info", "⌘", "I"),
        ]),
    ]
}

/// The Keyboard Shortcuts window, opened from the Help menu with ⌘/.
struct KeyboardShortcutsView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            ForEach(ShortcutGroup.all) { group in
                Section(group.title) {
                    ForEach(group.shortcuts) { shortcut in
                        LabeledContent(shortcut.action) {
                            HStack(spacing: 4) {
                                ForEach(shortcut.keys, id: \.self) { KeyCap(key: $0) }
                            }
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 340, minHeight: 320)
        // Esc closes the window, as it would a panel.
        .background {
            Button("Close") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .opacity(0)
                .accessibilityHidden(true)
        }
    }
}

private struct KeyCap: View {
    let key: String

    var body: some View {
        Text(key)
            .font(.callout.monospaced())
            .foregroundStyle(.secondary)
            .frame(minWidth: 22, minHeight: 22)
            .padding(.horizontal, key.count > 1 ? 6 : 0)
            .background(.quaternary.opacity(0.6), in: .rect(cornerRadius: 5))
            .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(.quaternary))
    }
}
