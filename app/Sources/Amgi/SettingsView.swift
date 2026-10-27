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
            let newCards = Binding(get: { model.newCardsPerDay }, set: { model.newCardsPerDay = max(0, $0) })
            LabeledContent {
                HStack(spacing: 4) {
                    TextField("New cards per day", value: newCards, format: .number)
                        .labelsHidden()
                        .multilineTextAlignment(.trailing)
                        .frame(width: 60)
                    Stepper("New cards per day", value: newCards, in: 0...9999, step: 5)
                        .labelsHidden()
                }
            } label: {
                Text("New cards per day")
                Text("How many cards you haven't seen before come up each day. Set 0 to only review. 20 suits most people.")
            }
            Toggle(isOn: Binding(
                get: { model.reviewsPerDay != nil },
                set: { model.reviewsPerDay = $0 ? Self.defaultReviewsPerDay : nil }
            )) {
                Text("Limit reviews per day")
                Text("Due cards past the limit wait for the next day, most overdue first. New cards and forgotten cards coming back aren't counted.")
            }
            if let limit = model.reviewsPerDay {
                let binding = Binding(get: { limit }, set: { model.reviewsPerDay = max(0, $0) })
                LabeledContent("Reviews per day") {
                    HStack(spacing: 4) {
                        TextField("Reviews per day", value: binding, format: .number)
                            .labelsHidden()
                            .multilineTextAlignment(.trailing)
                            .frame(width: 60)
                        Stepper("Reviews per day", value: binding, in: 0...9999, step: 10)
                            .labelsHidden()
                    }
                }
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

    /// Anki's default.
    private static let defaultReviewsPerDay = 200

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
