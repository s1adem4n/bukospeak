import SwiftUI

/// The channel: before a call the start screen, in a call who is in it and the controls. The
/// settings and the first launch's welcome come up over it.
struct ChannelView: View {
    @Environment(Call.self) private var call
    @Environment(\.scenePhase) private var scenePhase
    @State private var showsSettings = false
    /// Whether the welcome sheet asks for what a join needs, as on the first launch.
    @State private var isOnboarding = false
    /// Where the logo on the start screen becomes the microphone button of the call.
    @Namespace private var stage

    var body: some View {
        NavigationStack {
            Group {
                if isInCall {
                    CallView(stage: stage)
                } else {
                    LobbyView(stage: stage, join: join)
                }
            }
            // Inline throughout, so the bar keeps its height: a large title coming or going
            // changes it after the logo has landed, and the start screen jumps.
            .toolbarTitleDisplayMode(.inline)
            .toolbar {
                if isInCall {
                    ToolbarItem(placement: .principal) {
                        CallStatus()
                    }
                }
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
        .sheet(isPresented: $isOnboarding) {
            OnboardingView()
                .interactiveDismissDisabled()
        }
        .onAppear { isOnboarding = !call.settings.isComplete }
        .alert("Zertifikat geändert", isPresented: asksToTrust) {
            Button("Abbrechen", role: .cancel) {}
            if let server = call.changedServer {
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

    /// The server and room key the start screen watches while it shows in the foreground.
    private var watched: String? {
        guard scenePhase == .active, !isInCall else { return nil }
        return "\(call.settings.server)\n\(call.settings.roomKey)"
    }

    /// Whether the server's certificate changed, for the user to decide on: the one failure the
    /// start screen doesn't tell by itself.
    private var asksToTrust: Binding<Bool> {
        Binding(get: { call.changedServer != nil }, set: { if !$0 { call.dismissFailure() } })
    }

    /// Without a name, server or room key, joining opens the settings to fill them in.
    private func join() {
        if call.settings.isComplete {
            Task { await call.join() }
        } else {
            showsSettings = true
        }
    }
}
