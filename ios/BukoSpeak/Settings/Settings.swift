import Foundation
import Observation
import os

/// What the user set up: who they are and where the channel is, kept across launches; the
/// room key in the Keychain.
@Observable
final class Settings {
    var name: String {
        didSet { defaults.set(name, forKey: Key.name) }
    }
    /// The server as `host:port`.
    var server: String {
        didSet { defaults.set(server, forKey: Key.server) }
    }
    var roomKey: String {
        didSet {
            do {
                try Keychain.set(roomKey, for: Key.roomKey)
            } catch {
                Logger.settings.error("the room key isn't kept: Keychain status \(error.status)")
            }
        }
    }
    /// Whether RNNoise suppresses the microphone's noise, on top of the voice processing's own.
    var noiseSuppression: Bool {
        didSet { defaults.set(noiseSuppression, forKey: Key.noiseSuppression) }
    }
    /// How loud the others play: 1 as they come, up to `Engine.maxVolume`.
    var volume: Double {
        didSet { defaults.set(volume, forKey: Key.volume) }
    }
    /// The fingerprints of servers joined before, by server, as their text.
    private var pins: [String: String] {
        didSet { defaults.set(pins, forKey: Key.pins) }
    }

    @ObservationIgnored private let defaults: UserDefaults

    private enum Key {
        static let name = "name"
        static let server = "server"
        static let roomKey = "room-key"
        static let noiseSuppression = "noise-suppression"
        static let volume = "volume"
        static let pins = "pins"
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        name = defaults.string(forKey: Key.name) ?? ""
        server = defaults.string(forKey: Key.server) ?? ""
        roomKey = Keychain.get(Key.roomKey) ?? ""
        // Off unless chosen: the voice processing suppresses noise already, and a second
        // suppressor after it dulls the voice more than it removes.
        noiseSuppression = defaults.object(forKey: Key.noiseSuppression) as? Bool ?? false
        volume = defaults.object(forKey: Key.volume) as? Double ?? 1
        pins = defaults.dictionary(forKey: Key.pins) as? [String: String] ?? [:]
    }

    /// Whether there is enough to join.
    var isComplete: Bool {
        ![name, server, roomKey].contains { $0.trimmingCharacters(in: .whitespaces).isEmpty }
    }

    func pin(for server: String) -> Fingerprint? {
        pins[server].flatMap(Fingerprint.init)
    }

    /// Trusts the server by its fingerprint from now on.
    func pin(_ fingerprint: Fingerprint, for server: String) {
        pins[server] = fingerprint.description
    }

    /// Trusts the server anew on the next join, as when it was set up anew.
    func forgetPin(for server: String) {
        pins[server] = nil
    }
}

extension Logger {
    fileprivate static let settings = Logger(subsystem: "digital.buko.BukoSpeak", category: "settings")
}
