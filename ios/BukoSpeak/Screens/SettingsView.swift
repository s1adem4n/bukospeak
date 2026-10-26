import SwiftUI

/// Who you are, where the channel is, and how the call sounds.
struct SettingsView: View {
    @Environment(Call.self) private var call
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        @Bindable var settings = call.settings
        NavigationStack {
            Form {
                Section {
                    LabeledContent("Name") {
                        TextField("Name", text: $settings.name, prompt: Text(""))
                            .textContentType(.nickname)
                    }
                }
                Section {
                    LabeledContent("Server") {
                        TextField("Server", text: $settings.server, prompt: Text("host:port"))
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    }
                    LabeledContent("Raumschlüssel") {
                        SecureField("Raumschlüssel", text: $settings.roomKey)
                    }
                }
                Section {
                    LabeledContent("Lautstärke") {
                        HStack(spacing: 12) {
                            Slider(value: $settings.volume, in: 0...Engine.maxVolume, step: 0.05)
                            Text(settings.volume, format: .percent.precision(.fractionLength(0)))
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                                .frame(minWidth: 48, alignment: .trailing)
                        }
                    }
                    Toggle("Rauschfilter", isOn: $settings.noiseSuppression)
                }
            }
            .multilineTextAlignment(.trailing)
            .navigationTitle("Einstellungen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(role: .confirm) { dismiss() }
                }
            }
            .onChange(of: settings.noiseSuppression) { call.noiseSuppressionChanged() }
            .onChange(of: settings.volume) { call.volumeChanged() }
        }
    }
}
