import SwiftUI

/// The channel: before a call the logo and Beitreten; in a call who is in it and who speaks, and
/// the output, the microphone and hanging up.
struct ChannelView: View {
    @Environment(Call.self) private var call
    @Environment(\.scenePhase) private var scenePhase
    @State private var showsSettings = false
    /// Where the logo on the start screen becomes the microphone button of the call.
    @Namespace private var stage

    var body: some View {
        NavigationStack {
            Group {
                if isInCall {
                    channel
                } else {
                    lobby
                }
            }
            .navigationTitle(isInCall ? "BukoSpeak" : "")
            // Inline in both, so the bar keeps its height: a large title coming or going changes
            // it after the logo has landed, and the start screen jumps.
            .toolbarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Einstellungen", systemImage: "gearshape") { showsSettings = true }
                }
            }
        }
        .animation(Motion.spring, value: call.phase)
        .animation(Motion.spring, value: call.isHeld)
        .animation(Motion.spring, value: call.members)
        .animation(Motion.spring, value: call.present)
        .onChange(of: watched, initial: true) { _, watched in call.watch(watched != nil) }
        .sensoryFeedback(.impact(weight: .medium), trigger: call.isMuted)
        .sensoryFeedback(.success, trigger: call.phase) { _, phase in phase == .connected }
        .sheet(isPresented: $showsSettings) {
            SettingsView()
        }
        .alert(failure ?? "", isPresented: showsFailure) {
            if let server = call.changedServer {
                Button("Abbrechen", role: .cancel) {}
                Button("Vertrauen") { call.trustAnew(server) }
            }
        }
    }

    private var isInCall: Bool {
        switch call.phase {
        // Joining shows the channel at once, with who the start screen showed.
        case .joining, .connected, .reconnecting: true
        case .idle, .failed: false
        }
    }

    /// Why the call isn't live, shown in the middle of the channel.
    private enum Pause {
        case connecting(String)
        /// Another call holds it.
        case held
    }

    /// Nil while the call is live.
    private var pause: Pause? {
        switch call.phase {
        case .joining: .connecting("Verbinde…")
        case .reconnecting: .connecting("Verbinde neu…")
        case .connected: call.isHeld ? .held : nil
        case .idle, .failed: nil
        }
    }

    /// The channel recedes while nobody hears it; joining shows who is there in full.
    private var isDimmed: Bool {
        call.phase == .reconnecting || call.isHeld
    }

    /// The server and room key the start screen watches while it shows in the foreground.
    private var watched: String? {
        guard scenePhase == .active, call.phase == .idle || failure != nil else { return nil }
        return "\(call.settings.server)\n\(call.settings.roomKey)"
    }

    private var failure: String? {
        guard case .failed(let message) = call.phase else { return nil }
        return call.changedServer == nil ? message : "Zertifikat geändert"
    }

    private var showsFailure: Binding<Bool> {
        Binding(get: { failure != nil }, set: { if !$0 { call.dismissFailure() } })
    }

    private var lobby: some View {
        VStack(spacing: 20) {
            Spacer()
            BrandMark()
                .matchedGeometryEffect(id: "microphone", in: stage)
            Text("BukoSpeak")
                .font(.largeTitle.bold())
            present
            Spacer()
            Button(action: join) {
                Text("Beitreten")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.glassProminent)
            .controlSize(.extraLarge)
        }
        .padding()
        .background { Spotlight() }
    }

    /// Who is in the channel, before joining it.
    @ViewBuilder private var present: some View {
        if !call.present.isEmpty {
            VStack(spacing: 10) {
                HStack(spacing: -10) {
                    ForEach(call.present.prefix(5)) { member in
                        Avatar(name: member.name, size: 36)
                            .overlay { Circle().strokeBorder(Color(.systemBackground), lineWidth: 2) }
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

    /// Without a name, server or room key, Beitreten opens the settings to fill them in.
    private func join() {
        if call.settings.isComplete {
            Task { await call.join() }
        } else {
            showsSettings = true
        }
    }

    private var channel: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 96), spacing: 16)], spacing: 24) {
                ForEach(Array(call.members.enumerated()), id: \.element.id) { index, member in
                    MemberTile(
                        member: member,
                        speaks: call.speaking.contains(member.id),
                        // Your own as you set it, before the server tells it back.
                        isMuted: member.id == call.you ? call.isMuted : member.muted
                    )
                    // One after the other on entering, as the logo lands.
                    .transition(
                        .scale(scale: 0.6).combined(with: .opacity)
                            .animation(Motion.spring.delay(0.1 + Double(index) * 0.05))
                    )
                }
            }
            .padding()
        }
        .opacity(isDimmed ? 0.4 : 1)
        .overlay {
            switch pause {
            case .connecting(let label):
                ProgressView(label)
                    .controlSize(.large)
                    .transition(.opacity)
            case .held:
                VStack(spacing: 8) {
                    Image(systemName: "pause.fill")
                        .font(.largeTitle)
                    Text("Gehalten")
                }
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .transition(.opacity)
            case nil:
                EmptyView()
            }
        }
        .safeAreaInset(edge: .bottom) {
            controls
        }
    }

    private var controls: some View {
        GlassEffectContainer(spacing: 20) {
            HStack(spacing: 20) {
                RoutePicker()
                    .frame(width: 64, height: 64)
                    .glassEffect(in: .circle)
                muteButton
                    .matchedGeometryEffect(id: "microphone", in: stage)
                    // The microphone is off while another call holds this one.
                    .disabled(call.isHeld)
                Button(role: .destructive) {
                    Task { await call.leave() }
                } label: {
                    Image(systemName: "phone.down.fill")
                        .font(.title2)
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.glassProminent)
                .tint(.red)
                .accessibilityLabel("Verlassen")
            }
            .buttonBorderShape(.circle)
        }
        .padding(.bottom, 8)
    }

    /// The microphone: plain glass while it is on, red while muted.
    @ViewBuilder private var muteButton: some View {
        let button = Button {
            Task { await call.toggleMute() }
        } label: {
            Image(systemName: call.isMuted ? "mic.slash.fill" : "mic.fill")
                .font(.title)
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 56, height: 56)
        }
        .accessibilityLabel(call.isMuted ? "Stummschaltung aufheben" : "Stummschalten")
        if call.isMuted {
            button.buttonStyle(.glassProminent).tint(.red)
        } else {
            button.buttonStyle(.glass)
        }
    }
}
