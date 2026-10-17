import SwiftUI

/// The app's Settings window (⌘,).
struct SettingsView: View {
    @Environment(AppModel.self) private var model

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
        }
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
