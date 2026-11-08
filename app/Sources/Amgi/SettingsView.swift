import SwiftUI

/// Whether card sounds play when a card or its answer appears.
let autoplaySoundsKey = "autoplaySounds"
/// Whether a bar above the card shows how much of today's studying is done.
let studyProgressKey = "studyProgress"

/// The app's Settings window (⌘,), a tab for studying and one for the rest.
struct SettingsView: View {
    var body: some View {
        TabView {
            Tab("Studying", systemImage: "rectangle.stack") {
                StudySettings()
            }
            Tab("General", systemImage: "gearshape") {
                GeneralSettings()
            }
        }
    }
}

private struct StudySettings: View {
    @AppStorage(autoplaySoundsKey) private var autoplaySounds = true
    @AppStorage(studyProgressKey) private var studyProgress = true

    var body: some View {
        Form {
            Section {
                Toggle(isOn: $autoplaySounds) {
                    Text("Play audio automatically")
                    Text("Sounds play when a card or its answer appears. Press R to replay them.")
                }
                Toggle(isOn: $studyProgress) {
                    Text("Show progress bar")
                    Text("Fills as you work through today's due, learning and new cards.")
                }
            } footer: {
                Text("Desired retention, daily limits, when the day starts and leeches are set in amgi.toml in the collection folder. `amgi check --help` lists them.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }
}

private struct GeneralSettings: View {
    @Environment(AppModel.self) private var model
    @AppStorage(showMenuBarKey) private var showMenuBar = true

    var body: some View {
        @Bindable var model = model
        Form {
            Toggle(isOn: $showMenuBar) {
                Text("Show in menu bar")
                Text("Shows how many cards are due. \(appName) keeps running when its window is closed.")
            }
            Toggle(isOn: $model.quickAddShortcut) {
                Text("Add cards from any app with ⌥⌘N")
                Text("Opens Add Card over the app you're using. Add Card is also in the File menu (⌘N).")
            }
            Section {
                Toggle(isOn: $model.ankiConnect) {
                    Text("Add cards from Yomitan")
                    Text("Answers AnkiConnect, as Anki does, so Yomitan and other tools made for Anki can add cards while \(appName) is open. Only this Mac can reach it.")
                }
                if model.ankiConnect, let error = model.ankiConnectError {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
