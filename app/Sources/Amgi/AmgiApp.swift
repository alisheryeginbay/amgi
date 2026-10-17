import AppKit
import SwiftUI

@main
struct AmgiApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var model = AppModel()

    var body: some Scene {
        // One main window: a collection is studied in a single place, so no tabs.
        Window("Amgi", id: "main") {
            ContentView()
                .environment(model)
                .frame(minWidth: 720, minHeight: 460)
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                    // Pick up edits made in a text editor while the app was in the background.
                    model.reload()
                }
        }
        .commands {
            HelpCommands()
            CommandGroup(replacing: .newItem) {
                Button("Open Folder…") { model.chooseFolder() }
                    .keyboardShortcut("o")
                Button("Reload Cards") { model.reload() }
                    .keyboardShortcut("r")
                    .disabled(model.folder == nil)
                Divider()
                Button("Import Anki Deck…") { model.chooseAnkiPackage() }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                    .disabled(model.folder == nil || model.importing)
            }
        }

        // A card's history beside the main window, opened with ⌘I.
        Window("Card Info", id: "card-info") {
            CardInfoView()
                .environment(model)
        }
        .defaultSize(width: 420, height: 600)

        // Every shortcut in one list, opened from the Help menu with ⌘/.
        Window("Keyboard Shortcuts", id: "keyboard-shortcuts") {
            KeyboardShortcutsView()
        }
        .defaultSize(width: 380, height: 560)

        Settings {
            SettingsView()
                .environment(model)
        }
    }
}

/// Replaces the Help menu's search-only "Amgi Help" item, as there is no help book.
private struct HelpCommands: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .help) {
            Button("Keyboard Shortcuts") { openWindow(id: "keyboard-shortcuts") }
                .keyboardShortcut("/")
        }
    }
}

/// Lets the app take focus and show in the Dock when launched as a bare executable (`swift run`).
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        // Removes the tab bar and its menu items (Show Tab Bar, Merge All Windows).
        NSWindow.allowsAutomaticWindowTabbing = false
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
