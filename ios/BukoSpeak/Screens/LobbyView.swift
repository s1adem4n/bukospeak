import SwiftUI

/// The start screen, out of a call: the logo in its light, the server and who is in the channel
/// there, and the button to join; after a failed call, why, and the button to join again.
struct LobbyView: View {
    @Environment(Call.self) private var call
    /// Where the logo becomes the call's microphone button.
    let stage: Namespace.ID
    let join: () -> Void

    var body: some View {
        VStack(spacing: 20) {
            Spacer()
            BrandMark(height: 200)
                .matchedGeometryEffect(id: "microphone", in: stage)
            VStack(spacing: 6) {
                Text("BukoSpeak")
                    .font(.largeTitle.bold())
                if !server.isEmpty {
                    Text(server)
                        .font(.subheadline.monospaced())
                        .foregroundStyle(.secondary)
                }
            }
            present
            Spacer()
            VStack(spacing: 16) {
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.red)
                        .transition(.opacity)
                }
                Button(action: join) {
                    Text(failure == nil ? "Beitreten" : "Erneut beitreten")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.glassProminent)
                .controlSize(.extraLarge)
            }
        }
        .padding()
        .background { Spotlight() }
    }

    private var server: String {
        call.settings.server.trimmingCharacters(in: .whitespaces)
    }

    /// Why the last call failed; a changed certificate asks the user instead.
    private var failure: String? {
        guard case .failed(let message) = call.phase, call.changedServer == nil else { return nil }
        return message
    }

    /// Who is in the channel, before joining it.
    @ViewBuilder private var present: some View {
        if !call.present.isEmpty {
            VStack(spacing: 10) {
                HStack(spacing: -10) {
                    ForEach(call.present.prefix(5)) { member in
                        Avatar(name: member.name, size: 36)
                            .padding(2)
                            .background(Color(.systemBackground), in: .circle)
                            .transition(.scale.combined(with: .opacity))
                    }
                }
                Text(ListFormatter.localizedString(byJoining: call.present.map(\.name)))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .accessibilityElement(children: .combine)
            .transition(.opacity)
        }
    }
}
