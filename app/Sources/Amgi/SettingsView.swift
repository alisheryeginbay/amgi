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
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplaySounds = true
    @AppStorage(studyProgressKey) private var studyProgress = true

    var body: some View {
        Form {
            Section {
                schedule
            } footer: {
                Text(model.folder == nil
                    ? "Open a collection to change these. Each collection keeps its own."
                    : "Saved with this collection, so the app and the amgi command agree.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Section {
                options
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }

    @ViewBuilder private var schedule: some View {
        @Bindable var model = model
        Group {
            LabeledContent {
                HStack {
                    Slider(value: $model.desiredRetention, in: AppModel.retentionRange, step: 0.01)
                    Text(model.desiredRetention.formatted(.percent.precision(.fractionLength(0))))
                        .monospacedDigit()
                        .frame(width: 40, alignment: .trailing)
                }
                .frame(width: 170)
            } label: {
                Text("Desired retention")
                Text("How likely you should be to remember a card when it comes due. Higher means shorter intervals and more reviews. 90% suits most people.")
            }
            Picker(selection: $model.rolloverHour) {
                ForEach(0..<24, id: \.self) { hour in
                    Text(Self.hourLabel(hour)).tag(hour)
                }
            } label: {
                Text("Next day starts at")
                Text("Cards are due for the whole day. Studying after midnight but before this hour counts as the day before. 4 AM suits most people.")
            }
        }
        .disabled(model.folder == nil)
    }

    @ViewBuilder private var options: some View {
        Toggle(isOn: $autoplaySounds) {
            Text("Play audio automatically")
            Text("Sounds play when a card or its answer appears. Press R to replay them.")
        }
        Toggle(isOn: $studyProgress) {
            Text("Show progress bar")
            Text("Fills as you work through today's due, learning and new cards.")
        }
    }

    /// The hour as the Mac shows times, such as `4 AM` or `04:00`.
    private static func hourLabel(_ hour: Int) -> String {
        let date = Calendar.current.date(bySettingHour: hour, minute: 0, second: 0, of: .now) ?? .now
        return date.formatted(date: .omitted, time: .shortened)
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
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
