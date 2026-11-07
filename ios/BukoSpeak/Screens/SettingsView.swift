import SwiftUI

/// How the call sounds, and the server: in the desktop app's order, each kept as it is changed.
struct SettingsView: View {
    @Environment(Call.self) private var call
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        @Bindable var settings = call.settings
        NavigationStack {
            Form {
                Section("Audio") {
                    LabeledContent("Lautstärke") {
                        HStack(spacing: 12) {
                            Slider(value: $settings.volume, in: 0...Engine.maxVolume, step: 0.05)
                            Text(settings.volume, format: .percent.precision(.fractionLength(0)))
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                                .frame(minWidth: 48, alignment: .trailing)
                        }
                    }
                    Toggle("Rauschunterdrückung", isOn: $settings.noiseSuppression)
                }
                Section("Server") {
                    LabeledContent("Adresse") {
                        TextField("Adresse", text: $settings.server, prompt: Text("host:port"))
                            .font(.body.monospaced())
                            .multilineTextAlignment(.trailing)
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    }
                }
            }
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
