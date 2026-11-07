import SwiftUI

// In the desktop app's order, without what the iPhone does itself: its voice processing
// cancels echo, and the system picks the devices.
struct SettingsView: View {
    @Environment(Settings.self) private var settings
    @Environment(\.dismiss) private var dismiss
    // Kept once the sheet closes: an empty server would take the channel away beneath it.
    @State private var server = ""

    var body: some View {
        @Bindable var settings = settings
        NavigationStack {
            Form {
                Section("Mikrofon") {
                    Picker("Rauschfilter", selection: $settings.noiseSuppression) {
                        Text("Aus").tag(false)
                        Text("Standard").tag(true)
                    }
                    .pickerStyle(.menu)
                }
                Section("Lautsprecher") {
                    HStack(spacing: Space.gap) {
                        Text("Lautstärke")
                        Slider(value: $settings.volume, in: 0...Engine.maxVolume, step: 0.05)
                        Text(settings.volume, format: .percent.precision(.fractionLength(0)))
                            .font(.subheadline.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .frame(minWidth: Measure.value, alignment: .trailing)
                    }
                }
                Section("App") {
                    LabeledContent("Server") {
                        TextField("Server", text: $server, prompt: Text("server.example:9988"))
                            .font(.body.monospaced())
                            .multilineTextAlignment(.trailing)
                            .keyboardType(.URL)
                            .textContentType(.URL)
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
        }
        .onAppear { server = settings.server }
        .onDisappear(perform: keepServer)
    }

    private func keepServer() {
        let trimmed = server.trimmingCharacters(in: .whitespaces)
        if !trimmed.isEmpty {
            settings.server = trimmed
        }
    }
}
