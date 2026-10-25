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
                .listRowBackground(Palette.surface)
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
                .listRowBackground(Palette.surface)
                Section {
                    Toggle("Rauschfilter", isOn: $settings.noiseSuppression)
                        .tint(Palette.live)
                    LabeledContent("Ausgabe") {
                        RoutePicker()
                            .frame(width: 44, height: 44)
                    }
                }
                .listRowBackground(Palette.surface)
            }
            .multilineTextAlignment(.trailing)
            .foregroundStyle(Palette.text)
            .scrollContentBackground(.hidden)
            .background(Palette.background)
            .navigationTitle("Einstellungen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
            }
            .onChange(of: settings.noiseSuppression) { call.noiseSuppressionChanged() }
        }
    }
}
