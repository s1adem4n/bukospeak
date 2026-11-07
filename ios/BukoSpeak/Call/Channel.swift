import Foundation
import Observation

nonisolated enum Failure: Equatable, Sendable {
    case version
    case stranger
    case unidentified
    case full
    case unavailable
    case notFound
    case noAnswer
    case unreachable
    case audio
    case connectionLost
    case replaced
    case microphoneDenied
    case callNotStarted

    // As the desktop app says them.
    var text: String {
        switch self {
        case .version: "Server-Version passt nicht"
        case .stranger: "Nicht im Tailscale-Netz des Servers"
        case .unidentified: "Server erreicht Tailscale nicht"
        case .full: "Kanal ist voll"
        case .unavailable: "Download nicht verfügbar"
        case .notFound: "Server nicht gefunden"
        case .noAnswer: "Server antwortet nicht"
        case .unreachable: "Keine Verbindung zum Server"
        case .audio: "Audio nicht gestartet"
        case .connectionLost: "Verbindung verloren"
        case .replaced: "Auf diesem Gerät erneut beigetreten"
        case .microphoneDenied: "Mikrofon nicht erlaubt"
        case .callNotStarted: "Anruf nicht gestartet"
        }
    }
}

enum Seat: Identifiable, Equatable {
    case member(Member, speaks: Bool)
    case you(joining: Bool)

    var id: String {
        switch self {
        case .member(let member, _): "member-\(member.id)"
        case .you: "you"
        }
    }
}

@Observable
final class Channel {
    enum Phase: Equatable {
        case idle
        case joining
        case connected
        case reconnecting
        case failed(Failure)
    }

    var phase = Phase.idle
    var members: [Member] = []
    var speaking: Set<UInt16> = []
    var you: UInt16?
    var isMuted = false
    var isHeld = false
    var present: [Member] = []
    var since: Date?

    var isInCall: Bool {
        phase == .connected || phase == .reconnecting
    }

    var failure: Failure? {
        guard case .failed(let failure) = phase else { return nil }
        return failure
    }

    var seats: [Seat] {
        guard isInCall else {
            return present.map { .member($0, speaks: false) } + [.you(joining: phase == .joining)]
        }
        return members.map { member in
            // One's own as set here, before the server tells it back.
            let shown = member.id == you ? member.muted(isMuted) : member
            return .member(shown, speaks: speaking.contains(member.id))
        }
    }
}

extension Member {
    fileprivate func muted(_ muted: Bool) -> Member {
        Member(id: id, name: name, muted: muted, reconnecting: reconnecting)
    }
}
