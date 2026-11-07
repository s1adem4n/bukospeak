import SwiftUI

struct ChannelView: View {
    @Environment(Call.self) private var call
    @Environment(\.scenePhase) private var scenePhase
    @State private var showsSettings = false
    @State private var isOnboarding = false
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
    }

    private var isInCall: Bool {
        switch call.phase {
        case .joining, .connected, .reconnecting: true
        case .idle, .failed: false
        }
    }

    private var watched: String? {
        guard scenePhase == .active, !isInCall else { return nil }
        return call.settings.server
    }

    private func join() {
        if call.settings.isComplete {
            Task { await call.join() }
        } else {
            showsSettings = true
        }
    }
}
