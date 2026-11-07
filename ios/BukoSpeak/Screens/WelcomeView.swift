import SwiftUI

struct WelcomeView: View {
    @Environment(Settings.self) private var settings
    @Environment(\.call) private var call
    @State private var server = ""
    @FocusState private var isFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            BrandMark(height: Measure.mark)
            Text("Willkommen bei BukoSpeak")
                .font(.title2.weight(.bold))
                .padding(.top, Space.section)
            Text("Wo trefft ihr euch?")
                .foregroundStyle(.secondary)
                .padding(.top, Space.line)
            TextField("Server", text: $server, prompt: Text("server.example:9988"))
                .font(.body.monospaced())
                .keyboardType(.URL)
                .textContentType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.join)
                .focused($isFocused)
                .onSubmit(join)
                .padding(.horizontal, Space.gutter)
                .frame(height: Measure.field)
                .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: Radius.field))
                .overlay {
                    RoundedRectangle(cornerRadius: Radius.field)
                        .strokeBorder(.primary.opacity(isFocused ? 0.2 : 0.08), lineWidth: Measure.hairline)
                }
                .padding(.top, Space.section)
            Button("Beitreten", action: join)
                .buttonStyle(ControlStyle(kind: .primary, fills: true))
                .disabled(trimmed.isEmpty)
                .padding(.top, Space.gap)
        }
        .frame(maxWidth: Measure.welcome)
        .padding(.horizontal, Space.gutter)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(.systemGroupedBackground))
        .task { isFocused = true }
    }

    private var trimmed: String {
        server.trimmingCharacters(in: .whitespaces)
    }

    private func join() {
        guard !trimmed.isEmpty else { return }
        settings.server = trimmed
        call.join()
    }
}
