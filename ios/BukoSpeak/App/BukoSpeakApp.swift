import SwiftUI

@main
struct BukoSpeakApp: App {
    @State private var app = Running.launch()

    var body: some Scene {
        WindowGroup {
            RootView(showsSettings: app.showsSettings)
                .environment(app.channel)
                .environment(app.settings)
                .environment(\.call, app.actions)
                // Monochrome, as the desktop app.
                .tint(.primary)
        }
    }
}

struct Running {
    let channel: Channel
    let settings: Settings
    let actions: CallActions
    var showsSettings = false

    static func launch() -> Self {
        #if DEBUG
            if let staged = Staged.launched {
                return staged.running
            }
        #endif
        guard let engine = Engine() else { fatalError("the engine's threads didn't start") }
        let call = Call(settings: Settings(), engine: engine)
        return Running(channel: call.channel, settings: call.settings, actions: call.actions)
    }
}
