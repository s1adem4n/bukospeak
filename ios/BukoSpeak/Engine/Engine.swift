import BukoSpeakFFI
import Foundation

/// Someone in the channel.
nonisolated struct Member: Identifiable, Hashable, Sendable {
    let id: UInt16
    let name: String
    let muted: Bool
    /// The member's connection was lost, and the server holds its place for it to come back.
    let reconnecting: Bool
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

/// The channel after a change: who is in it and who speaks, kept by the engine.
nonisolated struct ChannelUpdate: Sendable {
    enum State: Sendable {
        case out
        case connected
        /// Nothing comes from the server, or the connection was lost and the engine joins again.
        case reconnecting
    }

    /// Which of the engine's joins the call is, counting from 1; it stays once the call is over.
    let call: UInt64
    let state: State
    /// This member's id, while in the channel.
    let you: UInt16?
    /// Everyone in the channel, this member included; kept while joining again.
    let members: [Member]
    /// Who speaks now, this member among them.
    let speaking: Set<UInt16>
    /// The call ended without a leave: why, after joining again failed too.
    let lost: String?
    /// Who is in the channel while the engine watches it from outside.
    let present: [Member]
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
    /// The channel after each change, in order.
    let updates: AsyncStream<ChannelUpdate>

    private let engine: OpaquePointer
    private let listener: Unmanaged<Listener>
    /// Joins and leaves block while the server answers, so they wait here, one after the other.
    private let queue = DispatchQueue(label: "digital.buko.BukoSpeak.engine")

    /// What the engine's listener holds: where its updates go.
    private final class Listener: Sendable {
        let continuation: AsyncStream<ChannelUpdate>.Continuation

        init(_ continuation: AsyncStream<ChannelUpdate>.Continuation) {
            self.continuation = continuation
        }
    }

    /// Nil if the engine's threads couldn't start.
    init?() {
        let (updates, continuation) = AsyncStream.makeStream(of: ChannelUpdate.self)
        let listener = Unmanaged.passRetained(Listener(continuation))
        guard
            let engine = bukospeak_engine_new(
                listener.toOpaque(),
                { context, update in
                    let listener = Unmanaged<Listener>.fromOpaque(context!).takeUnretainedValue()
                    listener.continuation.yield(ChannelUpdate(update!.pointee))
                }
            )
        else {
            listener.release()
            return nil
        }
        self.updates = updates
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

    /// Stops the call's audio and leaves the call, once the server knows. Returns the call the
    /// engine joined last: updates of it or of one before that are still on their way are stale.
    func leave() async -> UInt64 {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: bukospeak_engine_leave(self.engine)) }
        }
    }

    /// Watches who is in the channel without joining it, until `unwatch` or a join; the updates
    /// tell them as present. Only a server pinned by a join before is watched.
    func watch(server: String, roomKey: String, pin: Fingerprint) {
        var pin = pin.ffi
        bukospeak_engine_watch(engine, server, roomKey, &pin)
    }

    func unwatch() {
        bukospeak_engine_unwatch(engine)
    }

    /// Whether this member is muted: the microphone sends silence and the others see it muted,
    /// in this call and the ones after.
    func setMuted(_ muted: Bool) {
        bukospeak_engine_set_muted(engine, muted)
    }

    /// Whether the microphone's noise is suppressed (the default), or sent as captured.
    func setNoiseSuppression(_ on: Bool) {
        bukospeak_engine_set_noise_suppression(engine, on)
    }

    /// Plays the call on the voice processing unit, once the audio session is active; without
    /// a call, or playing already, nothing. It starts on the engine's queue, after what waits
    /// there already, since creating the unit takes long enough to freeze the screen; `started`
    /// then gets why it didn't, or nil.
    func startAudio(started: @escaping @MainActor @Sendable (AudioError?) -> Void) {
        queue.async {
            var message = ""
            let isPlaying = withUnsafeMutablePointer(to: &message) { message in
                bukospeak_engine_start_audio(self.engine, message) { context, text in
                    context!.assumingMemoryBound(to: String.self).pointee = String(text)
                }
            }
            let failure = isPlaying ? nil : AudioError(message: message)
            Task { @MainActor in started(failure) }
        }
    }

    /// Stops playing the call, for an audio session that is no longer active: on the engine's
    /// queue, after what waits there already, so it follows a start asked for before.
    func stopAudio() {
        queue.async { bukospeak_engine_stop_audio(self.engine) }
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
        self.init(
            id: member.id,
            name: String(member.name),
            muted: member.muted,
            reconnecting: member.reconnecting
        )
    }
}

nonisolated extension ChannelUpdate {
    fileprivate init(_ update: BukoSpeakUpdate) {
        let state: State =
            switch update.state {
            case BUKO_SPEAK_STATE_OUT: .out
            case BUKO_SPEAK_STATE_CONNECTED: .connected
            case BUKO_SPEAK_STATE_RECONNECTING: .reconnecting
            default: fatalError("the engine has no other states")
            }
        let lost = String(update.lost)
        self.init(
            call: update.call,
            state: state,
            you: update.you == 0 ? nil : update.you,
            members: UnsafeBufferPointer(start: update.members, count: update.members_len).map(Member.init),
            speaking: Set(UnsafeBufferPointer(start: update.speaking, count: update.speaking_len)),
            lost: lost.isEmpty ? nil : lost,
            present: UnsafeBufferPointer(start: update.present, count: update.present_len).map(Member.init)
        )
    }
}
