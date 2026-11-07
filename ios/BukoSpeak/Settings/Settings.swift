import Foundation
import Observation

/// What the user set up: where the channel is and how the call sounds, kept across launches.
@Observable
final class Settings {
    /// The server as `host:port`.
    var server: String {
        didSet { defaults.set(server, forKey: Key.server) }
    }
    /// Whether RNNoise suppresses the microphone's noise, on top of the voice processing's own.
    var noiseSuppression: Bool {
        didSet { defaults.set(noiseSuppression, forKey: Key.noiseSuppression) }
    }
    /// How loud the others play: 1 as they come, up to `Engine.maxVolume`.
    var volume: Double {
        didSet { defaults.set(volume, forKey: Key.volume) }
    }

    @ObservationIgnored private let defaults: UserDefaults

    private enum Key {
        static let server = "server"
        static let noiseSuppression = "noise-suppression"
        static let volume = "volume"
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        server = defaults.string(forKey: Key.server) ?? ""
        // Off unless chosen: the voice processing suppresses noise already, and a second
        // suppressor after it dulls the voice more than it removes.
        noiseSuppression = defaults.object(forKey: Key.noiseSuppression) as? Bool ?? false
        volume = defaults.object(forKey: Key.volume) as? Double ?? 1
    }

    /// Whether there is enough to join.
    var isComplete: Bool {
        !server.trimmingCharacters(in: .whitespaces).isEmpty
    }
}
