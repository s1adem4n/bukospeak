import SwiftUI

@main
struct BukoSpeakApp: App {
    @State private var call: Call
    /// Whether the first launch asks for the name, server and room key, until they are there.
    @State private var isOnboarding: Bool

    init() {
        guard let engine = Engine() else { fatalError("the engine's threads didn't start") }
        let call = Call(settings: Settings(), engine: engine)
        _call = State(initialValue: call)
        _isOnboarding = State(initialValue: !call.settings.isComplete)
    }

    var body: some Scene {
        WindowGroup {
            ZStack {
                if isOnboarding {
                    OnboardingView {
                        withAnimation(.easeInOut(duration: 0.4)) { isOnboarding = false }
                    }
                    // Above the start screen, which shows beneath as it fades out.
                    .zIndex(1)
                    .transition(.opacity.combined(with: .scale(scale: 1.05)))
                } else {
                    ChannelView()
                }
            }
            .environment(call)
        }
    }
}
