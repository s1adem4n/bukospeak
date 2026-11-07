import SwiftUI

struct OnboardingView: View {
    @Environment(Call.self) private var call
    @Environment(\.dismiss) private var dismiss
    @FocusState private var isFocused: Bool

    var body: some View {
        @Bindable var settings = call.settings
        VStack(spacing: 24) {
            BrandMark(height: 96)
            Text("Wo trefft ihr euch?")
                .font(.largeTitle.bold())
                .multilineTextAlignment(.center)
            TextField("Server", text: $settings.server, prompt: Text("host:port"))
                .keyboardType(.URL)
                .textContentType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .focused($isFocused)
                .onSubmit(finish)
                .padding(.horizontal)
                .frame(minHeight: 52)
                .background(.fill.tertiary, in: .rect(cornerRadius: 16))
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .safeAreaInset(edge: .bottom) {
            Button(action: finish) {
                Text("Fertig")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.glassProminent)
            .controlSize(.extraLarge)
            .disabled(!settings.isComplete)
            .padding()
        }
        .task { isFocused = true }
    }

    private func finish() {
        guard call.settings.isComplete else { return }
        isFocused = false
        dismiss()
    }
}
