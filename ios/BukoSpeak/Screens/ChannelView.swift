import SwiftUI

/// The channel: before a call the logo and Beitreten; in a call who is in it and who speaks, and
/// the microphone, the output and hanging up.
struct ChannelView: View {
    @Environment(Call.self) private var call
    @State private var showsSettings = false

    var body: some View {
        VStack(spacing: 0) {
            header
            if isInCall {
                channel
            } else {
                lobby
            }
        }
        .padding([.horizontal, .bottom], Metrics.margin)
        .background(Palette.background)
        .animation(Motion.spring, value: call.phase)
        .animation(Motion.spring, value: call.members)
        .sensoryFeedback(.impact(weight: .medium), trigger: call.isMuted)
        .sensoryFeedback(.success, trigger: call.phase) { _, phase in phase == .connected }
        .sheet(isPresented: $showsSettings) {
            SettingsView()
        }
    }

    private var isInCall: Bool {
        switch call.phase {
        case .connected, .reconnecting: true
        case .idle, .joining, .failed: false
        }
    }

    private var header: some View {
        HStack {
            if isInCall {
                Text("BukoSpeak")
                    .font(.title.bold())
                    .foregroundStyle(Palette.text)
            }
            Spacer()
            Button {
                showsSettings = true
            } label: {
                Image(systemName: "gearshape.fill")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Palette.text)
                    .frame(width: 36, height: 36)
                    .background(Palette.raised, in: .circle)
            }
            .accessibilityLabel("Einstellungen")
        }
        .frame(height: 52)
    }

    private var lobby: some View {
        VStack(spacing: 0) {
            if case .failed(let message) = call.phase {
                notice(message)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
            Spacer()
            BrandMark()
            Text("BukoSpeak")
                .font(.largeTitle.bold())
                .foregroundStyle(Palette.text)
                .padding(.top, 20)
            Spacer()
            Button(action: join) {
                if call.phase == .joining {
                    ProgressView().tint(Palette.onPrimary)
                } else {
                    Text("Beitreten")
                }
            }
            .buttonStyle(PrimaryButtonStyle())
            .disabled(call.phase == .joining)
        }
    }

    /// Without a name, server or room key, Beitreten opens the settings to fill them in.
    private func join() {
        if call.settings.isComplete {
            Task { await call.join() }
        } else {
            showsSettings = true
        }
    }

    private func notice(_ message: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: call.changedServer == nil ? "exclamationmark.triangle.fill" : "lock.fill")
                .foregroundStyle(Palette.warning)
            Text(call.changedServer == nil ? message : "Zertifikat geändert")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Palette.text)
                .lineLimit(2)
            Spacer(minLength: 0)
            if let server = call.changedServer {
                Button("Vertrauen") { call.trustAnew(server) }
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Palette.onPrimary)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Palette.text, in: .capsule)
            }
        }
        .padding(.leading, 16)
        .padding([.trailing, .vertical], 12)
        .background(Palette.surface, in: .rect(cornerRadius: Metrics.tileRadius))
        .overlay {
            RoundedRectangle(cornerRadius: Metrics.tileRadius).strokeBorder(Palette.line)
        }
    }

    private var channel: some View {
        VStack(spacing: 0) {
            if let status {
                HStack(spacing: 6) {
                    Circle()
                        .fill(Palette.warning)
                        .frame(width: 7, height: 7)
                    Text(status)
                }
                .font(.footnote)
                .foregroundStyle(Palette.secondaryText)
                .frame(maxWidth: .infinity, alignment: .leading)
                .transition(.opacity)
            }
            ScrollView {
                LazyVGrid(
                    columns: Array(repeating: GridItem(spacing: Metrics.spacing), count: 2),
                    spacing: Metrics.spacing
                ) {
                    ForEach(call.members) { member in
                        MemberTile(
                            member: member,
                            speaks: call.speaking.contains(member.id),
                            isMuted: member.id == call.you && call.isMuted
                        )
                        .transition(.scale(scale: 0.9).combined(with: .opacity))
                    }
                }
                .padding(.vertical, Metrics.margin)
            }
            .scrollIndicators(.hidden)
            .opacity(call.phase == .reconnecting ? 0.4 : 1)
            controls
        }
    }

    /// What isn't as it should be in the call, if anything.
    private var status: String? {
        if call.phase == .reconnecting {
            "Verbinde neu…"
        } else if call.isHeld {
            "Gehalten"
        } else {
            nil
        }
    }

    private var controls: some View {
        HStack(spacing: 20) {
            RoutePicker()
                .frame(width: Metrics.roundButton, height: Metrics.roundButton)
                .background(Palette.raised, in: .circle)
            Button {
                Task { await call.toggleMute() }
            } label: {
                Image(systemName: call.isMuted ? "mic.slash.fill" : "mic.fill")
                    .contentTransition(.symbolEffect(.replace))
            }
            .buttonStyle(RoundButtonStyle(kind: .main, isOff: call.isMuted))
            .accessibilityLabel(call.isMuted ? "Stummschaltung aufheben" : "Stummschalten")
            Button {
                Task { await call.leave() }
            } label: {
                Image(systemName: "phone.down.fill")
            }
            .buttonStyle(RoundButtonStyle(kind: .destructive))
            .accessibilityLabel("Verlassen")
        }
        .padding(.top, 8)
    }
}
