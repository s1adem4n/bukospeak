import BukoSpeakFFI
import Foundation

/// Someone in the channel.
nonisolated struct Member: Identifiable, Hashable, Sendable {
    let id: UInt16
    let name: String
}

/// A server's certificate fingerprint, its SHA-256, pinned on the first join.
nonisolated struct Fingerprint: Hashable, Sendable, CustomStringConvertible {
    let bytes: [UInt8]

    /// The way servers log it: hex bytes, colon-separated.
    var description: String {
        bytes.map { String(format: "%02x", $0) }.joined(separator: ":")
    }

    init?(_ text: String) {
        let bytes = text.split(separator: ":").compactMap { UInt8($0, radix: 16) }
        guard bytes.count == 32, text.count == 32 * 3 - 1 else { return nil }
        self.bytes = bytes
    }

    fileprivate init(_ fingerprint: BukoSpeakFingerprint) {
        bytes = withUnsafeBytes(of: fingerprint.bytes) { Array($0) }
    }

    fileprivate var ffi: BukoSpeakFingerprint {
        var fingerprint = BukoSpeakFingerprint()
        withUnsafeMutableBytes(of: &fingerprint.bytes) { $0.copyBytes(from: bytes) }
        return fingerprint
    }
}

/// What happens in the calls, in order.
nonisolated enum EngineEvent: Sendable {
    /// A call was joined: who is in it now. Sent before the call's other events, so they are
    /// this call's until the next.
    case entered(you: UInt16, members: [Member])
    case joined(Member)
    case left(UInt16)
    /// Who speaks now, this member among them.
    case speaking(Set<UInt16>)
    /// The call ended without a leave: the connection was lost.
    case disconnected(String)
}

/// A call just joined.
nonisolated struct Joined: Sendable {
    /// The server's, to pin for the next join.
    let fingerprint: Fingerprint
}

nonisolated enum JoinError: Error {
    /// The server presented another certificate than the one pinned.
    case fingerprintChanged(pinned: Fingerprint, presented: Fingerprint, message: String)
    case failed(String)
}

nonisolated struct AudioError: Error {
    let message: String
}

/// The Rust engine, through its C interface: joins and leaves the channel and plays the call.
/// The engine may be called from any thread, so its handle may be shared.
nonisolated final class Engine: @unchecked Sendable {
    /// The engine's events, in order.
    let events: AsyncStream<EngineEvent>

    private let engine: OpaquePointer
    private let listener: Unmanaged<Listener>
    /// Joins and leaves block while the server answers, so they wait here, one after the other.
    private let queue = DispatchQueue(label: "digital.buko.BukoSpeak.engine")

    /// What the engine's listener holds: where its events go.
    private final class Listener: Sendable {
        let continuation: AsyncStream<EngineEvent>.Continuation

        init(_ continuation: AsyncStream<EngineEvent>.Continuation) {
            self.continuation = continuation
        }
    }

    /// Nil if the engine's threads couldn't start.
    init?() {
        let (events, continuation) = AsyncStream.makeStream(of: EngineEvent.self)
        let listener = Unmanaged.passRetained(Listener(continuation))
        guard
            let engine = bukospeak_engine_new(
                listener.toOpaque(),
                { context, event in
                    let listener = Unmanaged<Listener>.fromOpaque(context!).takeUnretainedValue()
                    listener.continuation.yield(EngineEvent(event!.pointee))
                }
            )
        else {
            listener.release()
            return nil
        }
        self.events = events
        self.engine = engine
        self.listener = listener
    }

    deinit {
        // The engine calls the listener until it is freed, never after.
        bukospeak_engine_free(engine)
        listener.takeRetainedValue().continuation.finish()
    }

    /// Leaves the call before, if there is one, and joins the channel, trusting the server by
    /// `pin`, or on first use if it is nil.
    func join(server: String, name: String, roomKey: String, pin: Fingerprint?) async throws(JoinError) -> Joined {
        let result = await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: self.join(server, name, roomKey, pin)) }
        }
        return try result.get()
    }

    /// Stops the call's audio and leaves the call, once the server knows.
    func leave() async {
        await withCheckedContinuation { continuation in
            queue.async {
                bukospeak_engine_leave(self.engine)
                continuation.resume()
            }
        }
    }

    /// Whether the microphone sends silence, in this call and the ones after.
    func setMuted(_ muted: Bool) {
        bukospeak_engine_set_muted(engine, muted)
    }

    /// Whether the microphone's noise is suppressed (the default), or sent as captured.
    func setNoiseSuppression(_ on: Bool) {
        bukospeak_engine_set_noise_suppression(engine, on)
    }

    /// Plays the call on the voice processing unit, once the audio session is active; without
    /// a call, or playing already, nothing.
    func startAudio() throws(AudioError) {
        var message = ""
        let started = withUnsafeMutablePointer(to: &message) { message in
            bukospeak_engine_start_audio(engine, message) { context, text in
                context!.assumingMemoryBound(to: String.self).pointee = String(text)
            }
        }
        if !started {
            throw AudioError(message: message)
        }
    }

    /// Stops playing the call, for an audio session that is no longer active.
    func stopAudio() {
        bukospeak_engine_stop_audio(engine)
    }

    private func join(
        _ server: String,
        _ name: String,
        _ roomKey: String,
        _ pin: Fingerprint?
    ) -> Result<Joined, JoinError> {
        var result = Result<Joined, JoinError>.failure(.failed(""))
        let done: BukoSpeakJoinDone = { context, joined in
            let joined = joined!.pointee
            context!.assumingMemoryBound(to: Result<Joined, JoinError>.self).pointee =
                switch joined.status {
                case BUKO_SPEAK_JOIN_STATUS_JOINED:
                    .success(
                        Joined(fingerprint: Fingerprint(joined.fingerprint))
                    )
                case BUKO_SPEAK_JOIN_STATUS_FINGERPRINT_CHANGED:
                    .failure(
                        .fingerprintChanged(
                            pinned: Fingerprint(joined.pinned),
                            presented: Fingerprint(joined.fingerprint),
                            message: String(joined.message)
                        )
                    )
                case BUKO_SPEAK_JOIN_STATUS_FAILED:
                    .failure(.failed(String(joined.message)))
                default:
                    fatalError("a join ends in no other status")
                }
        }
        withUnsafeMutablePointer(to: &result) { result in
            if var pin = pin?.ffi {
                bukospeak_engine_join(engine, server, name, roomKey, &pin, result, done)
            } else {
                bukospeak_engine_join(engine, server, name, roomKey, nil, result, done)
            }
        }
        return result
    }
}

nonisolated extension String {
    /// Text the engine lends for the duration of a callback.
    fileprivate init(_ text: BukoSpeakStr) {
        self.init(decoding: UnsafeBufferPointer(start: text.ptr, count: text.len), as: UTF8.self)
    }
}

nonisolated extension Member {
    fileprivate init(_ member: BukoSpeakMember) {
        self.init(id: member.id, name: String(member.name))
    }
}

nonisolated extension EngineEvent {
    fileprivate init(_ event: BukoSpeakEvent) {
        self =
            switch event.kind {
            case BUKO_SPEAK_EVENT_KIND_ENTERED:
                .entered(
                    you: event.you,
                    members: UnsafeBufferPointer(start: event.members, count: event.members_len).map(Member.init)
                )
            case BUKO_SPEAK_EVENT_KIND_JOINED: .joined(Member(event.member))
            case BUKO_SPEAK_EVENT_KIND_LEFT: .left(event.member.id)
            case BUKO_SPEAK_EVENT_KIND_SPEAKING:
                .speaking(Set(UnsafeBufferPointer(start: event.speaking, count: event.speaking_len)))
            case BUKO_SPEAK_EVENT_KIND_DISCONNECTED: .disconnected(String(event.reason))
            default: fatalError("the engine sends no other events")
            }
    }
}
