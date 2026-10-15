import AppKit
import SwiftUI

@main
struct AmgiApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup("Amgi") {
            ContentView()
                .environment(model)
                .frame(minWidth: 720, minHeight: 460)
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                    // Pick up edits made in a text editor while the app was in the background.
                    model.reload()
                }
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Open Folder…") { model.chooseFolder() }
                    .keyboardShortcut("o")
                Button("Reload Cards") { model.reload() }
                    .keyboardShortcut("r")
                    .disabled(model.folder == nil)
            }
        }
    }
}

/// Lets the app take focus and show in the Dock when launched as a bare executable (`swift run`).
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
