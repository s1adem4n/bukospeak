import SwiftUI

@main
struct BukoSpeakApp: App {
    @State private var call: Call = {
        guard let engine = Engine() else { fatalError("the engine's threads didn't start") }
        return Call(settings: Settings(), engine: engine)
    }()

    var body: some Scene {
        WindowGroup {
            ChannelView()
                .environment(call)
        }
    }
}
