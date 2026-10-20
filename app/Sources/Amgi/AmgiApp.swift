import AppKit
import SwiftUI

@main
struct AmgiApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var model = AppModel()
    @AppStorage(showMenuBarKey) private var showMenuBar = true

    var body: some Scene {
        // One main window: a collection is studied in a single place, so no tabs.
        Window(appName, id: "main") {
            ContentView()
                .environment(model)
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                    // Pick up edits made in a text editor while the app was in the background.
                    model.reload()
                }
        }
        // The welcome screen is a fixed size; the collection's window resizes freely.
        .windowResizability(.contentSize)
        .commands {
            HelpCommands()
            CommandGroup(replacing: .newItem) {
                Button("Open Folder…") { model.chooseFolder() }
                    .keyboardShortcut("o")
                Button("Close Folder") { model.closeFolder() }
                    .disabled(model.folder == nil || model.importing)
                Button("Reload Cards") { model.reload() }
                    .keyboardShortcut("r")
                    .disabled(model.folder == nil)
                Divider()
                Button("Add Card…") { model.toggleQuickAdd() }
                    .keyboardShortcut("n")
                Divider()
                Button("Import Anki Deck…") { model.chooseAnkiPackage() }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                    .disabled(model.folder == nil || model.importing)
            }
            // The sidebar's first three items, as in Mail and Finder.
            CommandGroup(before: .sidebar) {
                Button("Dashboard") { model.selection = .dashboard }
                    .keyboardShortcut("1")
                    .disabled(model.folder == nil)
                Button("Review") { model.selection = .review }
                    .keyboardShortcut("2")
                    .disabled(model.folder == nil)
                Button("Decks") { model.selection = .cards }
                    .keyboardShortcut("3")
                    .disabled(model.folder == nil)
                Divider()
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

        // Cards due at a glance, while the window is closed or behind others.
        MenuBarExtra(isInserted: $showMenuBar) {
            MenuBarMenu()
                .environment(model)
        } label: {
            MenuBarLabel()
                .environment(model)
        }
    }
}

/// Whether this is a development build (`make`, `swift run`) rather than Amgi.app, which
/// has a bundle identifier. It's called Amgi Dev and its flower is violet, so it isn't
/// mistaken for the installed app.
let isDevBuild = Bundle.main.bundleIdentifier == nil
let appName = isDevBuild ? "Amgi Dev" : "Amgi"
/// The dev build's petals, in place of forget-me-not blue.
let devViolet = NSColor(srgbRed: 0x91 / 255, green: 0x5b / 255, blue: 0xef / 255, alpha: 1)

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
        // A bare executable has no icon of its own.
        if isDevBuild, let url = Bundle.module.url(forResource: "AmgiDev", withExtension: "icns") {
            NSApp.applicationIconImage = NSImage(contentsOf: url)
        }
        NSApp.activate()
    }

    /// The menu bar item keeps the app running, so it can show what comes due.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        UserDefaults.standard.object(forKey: showMenuBarKey) as? Bool == false
    }
}
