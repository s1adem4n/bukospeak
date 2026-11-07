import Foundation
import Observation

@Observable
final class Settings {
    var server: String {
        didSet { defaults.set(server, forKey: Key.server) }
    }
    var noiseSuppression: Bool {
        didSet { defaults.set(noiseSuppression, forKey: Key.noiseSuppression) }
    }
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

    var isComplete: Bool {
        !server.trimmingCharacters(in: .whitespaces).isEmpty
    }
}
