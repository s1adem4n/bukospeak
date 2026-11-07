#if DEBUG
    import Foundation

    // The screens in made-up states, as the desktop's harness stages them, for `ios/dev snap`:
    // launched with `-stage <name>`, the app shows the scene instead of joining anything.
    struct Staged {
        let name: String
        var server = "voice.example:9988"
        var showsSettings = false
        var stage: (Channel) -> Void = { _ in }

        static var launched: Staged? {
            guard let name = UserDefaults.standard.string(forKey: "stage") else { return nil }
            guard let staged = all.first(where: { $0.name == name }) else {
                fatalError("no scene \(name); there are \(all.map(\.name).joined(separator: ", "))")
            }
            return staged
        }

        var running: Running {
            let defaults = UserDefaults(suiteName: "stage")!
            defaults.removePersistentDomain(forName: "stage")
            let settings = Settings(defaults: defaults)
            settings.server = server
            let channel = Channel()
            stage(channel)
            return Running(channel: channel, settings: settings, actions: CallActions(), showsSettings: showsSettings)
        }

        static let all: [Staged] = [
            Staged(name: "welcome", server: ""),
            Staged(name: "lobby") { $0.present = Array(two.dropFirst()) },
            Staged(name: "lobby-three") { $0.present = Array(five[1..<4]) },
            Staged(name: "lobby-empty"),
            Staged(name: "joining") {
                $0.phase = .joining
                $0.present = Array(two.dropFirst())
            },
            Staged(name: "call") { inCall($0) },
            Staged(name: "call-four") { inCall($0, members: Array(five.prefix(4))) },
            Staged(name: "call-five") { inCall($0, members: five, speaking: [2, 4]) },
            Staged(name: "call-alone") { inCall($0, members: Array(two.prefix(1)), speaking: []) },
            Staged(name: "call-muted") {
                inCall($0, speaking: [])
                $0.isMuted = true
            },
            Staged(name: "call-speaking") { inCall($0, speaking: [you]) },
            Staged(name: "reconnecting") {
                inCall($0, speaking: [])
                $0.phase = .reconnecting
            },
            Staged(name: "member-reconnecting") {
                inCall(
                    $0,
                    members: [two[0], Member(id: 2, name: "Kiwi", muted: false, reconnecting: true)],
                    speaking: []
                )
            },
            Staged(name: "held") {
                inCall($0, speaking: [])
                $0.isHeld = true
            },
            Staged(name: "failed") {
                $0.phase = .failed(.unreachable)
                $0.present = Array(two.dropFirst())
            },
            Staged(name: "stranger") { $0.phase = .failed(.stranger) },
            Staged(name: "settings", showsSettings: true) { $0.present = Array(two.dropFirst()) },
        ]

        private static let you: UInt16 = 1

        private static let two = [
            Member(id: 1, name: "Nachtfalke", muted: false, reconnecting: false),
            Member(id: 2, name: "Kiwi", muted: false, reconnecting: false),
        ]

        private static let five =
            two + [
                Member(id: 3, name: "Mondschein", muted: true, reconnecting: false),
                Member(id: 4, name: "Pixel", muted: false, reconnecting: false),
                Member(id: 5, name: "Zugvogel", muted: false, reconnecting: false),
            ]

        private static func inCall(_ channel: Channel, members: [Member] = two, speaking: Set<UInt16> = [2]) {
            channel.phase = .connected
            channel.you = you
            channel.members = members
            channel.speaking = speaking
            channel.since = .now - (47 * 60 + 12)
        }
    }
#endif
