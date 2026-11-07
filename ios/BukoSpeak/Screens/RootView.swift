import SwiftUI

struct RootView: View {
    @Environment(Channel.self) private var channel
    @Environment(Settings.self) private var settings
    @Environment(\.call) private var call
    @Environment(\.scenePhase) private var scenePhase
    @State private var showsSettings: Bool

    init(showsSettings: Bool = false) {
        _showsSettings = State(initialValue: showsSettings)
    }

    var body: some View {
        Group {
            if settings.isComplete {
                ChannelView(showsSettings: $showsSettings)
                    .transition(.opacity)
            } else {
                WelcomeView()
                    .transition(.opacity)
            }
        }
        .animation(Motion.spring, value: settings.isComplete)
        .onChange(of: watched, initial: true) { _, watched in call.watch(watched != nil) }
        .sensoryFeedback(.impact(weight: .medium), trigger: channel.isMuted)
        .sensoryFeedback(.success, trigger: channel.phase) { _, phase in phase == .connected }
    }

    // The server, so a new one is watched instead.
    private var watched: String? {
        guard scenePhase == .active, settings.isComplete, !channel.isInCall else { return nil }
        return settings.server
    }
}
