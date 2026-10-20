import SwiftUI

/// The app's Settings window (⌘,).
/// Whether card sounds play when a card or its answer appears.
let autoplaySoundsKey = "autoplaySounds"
/// Whether a bar above the card shows how much of today's studying is done.
let studyProgressKey = "studyProgress"

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplaySounds = true
    @AppStorage(studyProgressKey) private var studyProgress = true
    @AppStorage(showMenuBarKey) private var showMenuBar = true

    var body: some View {
        @Bindable var model = model
        Form {
            Section {
                LabeledContent("Desired retention") {
                    HStack {
                        Slider(value: $model.desiredRetention, in: AppModel.retentionRange, step: 0.01)
                        Text(model.desiredRetention.formatted(.percent.precision(.fractionLength(0))))
                            .monospacedDigit()
                            .frame(width: 40, alignment: .trailing)
                    }
                }
            } footer: {
                Text("How likely you should be to remember a card when it comes due. Higher values mean shorter intervals and more reviews. 90% suits most people.")
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Section {
                Toggle("Play audio automatically", isOn: $autoplaySounds)
            } footer: {
                Text("Sounds on the question play when a card appears, and the answer's when it is shown. Press R to replay them.")
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Section {
                Toggle("Show progress bar", isOn: $studyProgress)
            } footer: {
                Text("A bar above the card fills as you work through today's due, learning and new cards.")
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Section {
                Toggle("Add cards from any app with ⌥⌘N", isOn: $model.quickAddShortcut)
            } footer: {
                Text("Opens Add Card over the app you're using, while Amgi is running. Add Card is also in the File menu (⌘N).")
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Section {
                Toggle("Show in menu bar", isOn: $showMenuBar)
            } footer: {
                Text("Shows how many cards are due. Amgi keeps running when its window is closed.")
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
