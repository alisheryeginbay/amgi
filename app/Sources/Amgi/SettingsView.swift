import SwiftUI

/// The app's Settings window (⌘,).
/// Whether card sounds play when a card or its answer appears.
let autoplaySoundsKey = "autoplaySounds"

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplaySounds = true

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
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
