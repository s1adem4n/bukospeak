import SwiftUI

/// Who you are, where the channel is, and how the call sounds.
struct SettingsView: View {
    @Environment(Call.self) private var call
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        @Bindable var settings = call.settings
        NavigationStack {
            Form {
                Section("You") {
                    TextField("Name", text: $settings.name)
                        .textContentType(.nickname)
                }
                Section {
                    TextField("host:port", text: $settings.server)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("Room key", text: $settings.roomKey)
                } header: {
                    Text("Server")
                } footer: {
                    Text("Changes apply from the next join.")
                }
                Section {
                    Toggle("Noise suppression", isOn: $settings.noiseSuppression)
                    HStack {
                        Text("Output")
                        Spacer()
                        RoutePicker()
                            .frame(width: 44, height: 44)
                    }
                } header: {
                    Text("Audio")
                } footer: {
                    Text(
                        "Noise suppression keeps game sound and the room out of what you send. The microphone follows the output."
                    )
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .onChange(of: settings.noiseSuppression) { call.noiseSuppressionChanged() }
        }
    }
}
