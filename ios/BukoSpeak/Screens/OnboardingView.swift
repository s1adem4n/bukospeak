import SwiftUI

/// The first launch: the name, then where the channel is; `done` once there is enough to join.
/// The pages slide within one screen, so the button and the keyboard stay where they are.
struct OnboardingView: View {
    let done: () -> Void

    @Environment(Call.self) private var call
    @State private var step = Step.name
    @FocusState private var focused: Field?

    private enum Step {
        case name, channel
    }

    private enum Field {
        case name, server, roomKey
    }

    // Without a navigation stack: the system's, backed by UIKit, vanishes at once when the
    // onboarding ends instead of fading out.
    var body: some View {
        VStack(spacing: 0) {
            // Always as high, so the pages stand still when the back button comes and goes.
            HStack {
                if step == .channel {
                    Button("Zurück", systemImage: "chevron.backward") { go(to: .name) }
                        .labelStyle(.iconOnly)
                        .font(.title3.weight(.medium))
                        .frame(width: 44, height: 44)
                        .buttonStyle(.glass)
                        .buttonBorderShape(.circle)
                        .transition(.opacity)
                }
                Spacer()
            }
            .frame(height: 44)
            ZStack(alignment: .top) {
                switch step {
                // Each off its own side: the name to the left, the channel to the right.
                case .name: namePage.transition(.move(edge: .leading).combined(with: .opacity))
                case .channel: channelPage.transition(.move(edge: .trailing).combined(with: .opacity))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .padding()
        .safeAreaInset(edge: .bottom) {
            Button(action: proceed) {
                Text(step == .name ? "Weiter" : "Fertig")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.glassProminent)
            .controlSize(.extraLarge)
            .disabled(!isReady)
            .padding()
        }
        .background(Color(.systemBackground))
        .task { focused = .name }
    }

    private var namePage: some View {
        @Bindable var settings = call.settings
        return VStack(spacing: 24) {
            Avatar(name: settings.name.trimmingCharacters(in: .whitespaces), size: 96)
            title("Wie heißt du?")
            Fields {
                TextField("Name", text: $settings.name)
                    .textContentType(.nickname)
                    .submitLabel(.next)
                    .focused($focused, equals: .name)
                    .onSubmit(proceed)
            }
        }
    }

    private var channelPage: some View {
        @Bindable var settings = call.settings
        return VStack(spacing: 24) {
            Image(systemName: "server.rack")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
                .frame(width: 96, height: 96)
            title("Wo trefft ihr euch?")
            Fields {
                TextField("Server", text: $settings.server, prompt: Text("host:port"))
                    .keyboardType(.URL)
                    .textContentType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.next)
                    .focused($focused, equals: .server)
                    .onSubmit { focused = .roomKey }
                SecureField("Raumschlüssel", text: $settings.roomKey)
                    .submitLabel(.done)
                    .focused($focused, equals: .roomKey)
                    .onSubmit(proceed)
            }
        }
    }

    private func title(_ text: String) -> some View {
        Text(text)
            .font(.largeTitle.bold())
            .multilineTextAlignment(.center)
    }

    private var isReady: Bool {
        switch step {
        case .name: !call.settings.name.trimmingCharacters(in: .whitespaces).isEmpty
        case .channel: call.settings.isComplete
        }
    }

    private func proceed() {
        guard isReady else { return }
        switch step {
        case .name: go(to: .channel)
        case .channel:
            focused = nil
            done()
        }
    }

    /// Slides to the page, its first field taking the keyboard in the same change, so it stays up.
    private func go(to next: Step) {
        withAnimation(Motion.spring) {
            step = next
            focused = next == .name ? .name : .server
        }
    }
}

/// Text fields in one rounded group, one above the other, as the system's forms show them.
private struct Fields<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        Group(subviews: content) { fields in
            VStack(spacing: 0) {
                ForEach(fields) { field in
                    if field.id != fields.first?.id {
                        Divider()
                            .padding(.leading)
                    }
                    field
                        .padding(.horizontal)
                        .frame(minHeight: 52)
                }
            }
        }
        .background(.fill.tertiary, in: .rect(cornerRadius: 16))
    }
}
