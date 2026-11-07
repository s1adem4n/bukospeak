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
    /// Why the call ended without a leave.
    let lost: EngineError?
    /// Who is in the channel while the engine watches it from outside.
    let present: [Member]
}

/// Why a join failed or a call was lost.
nonisolated struct EngineError: Error {
    enum Reason: Sendable {
        /// Anything else: the server didn't answer, the connection broke, and the like.
        case other
        /// The server's tailnet doesn't know this device: it isn't on that tailnet, or
        /// Tailscale is off.
        case stranger
        /// The server couldn't ask its tailnet who this device is.
        case unidentified
        /// This device joined the channel again elsewhere, which took the call's place.
        case replaced
    }

    let reason: Reason
    /// What happened, in English, for the log.
    let message: String
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

    /// Leaves the call before, if there is one, and joins the channel of `server`, as
    /// `host:port`, as this device, which the server knows from its tailnet.
    func join(server: String) async throws(EngineError) {
        let result = await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: self.join(server)) }
        }
        try result.get()
    }

    /// Stops the call's audio and leaves the call, once the server knows. Returns the call the
    /// engine joined last: updates of it or of one before that are still on their way are stale.
    func leave() async -> UInt64 {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: bukospeak_engine_leave(self.engine)) }
        }
    }

    /// Watches who is in the channel of `server` without joining it, until `unwatch` or a join;
    /// the updates tell them as present.
    func watch(server: String) {
        bukospeak_engine_watch(engine, server)
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

    /// How loud the others play: 1 as they come, up to `maxVolume`, in this call and the ones
    /// after.
    func setVolume(_ volume: Double) {
        bukospeak_engine_set_volume(engine, Float(volume))
    }

    /// The loudest the others play: twice their level.
    static let maxVolume = Double(bukospeak_max_volume())

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

    private func join(_ server: String) -> Result<Void, EngineError> {
        var result = Result<Void, EngineError>.success(())
        withUnsafeMutablePointer(to: &result) { result in
            bukospeak_engine_join(engine, server, result) { context, error in
                guard let error else { return }
                context!.assumingMemoryBound(to: Result<Void, EngineError>.self).pointee =
                    .failure(EngineError(error.pointee))
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
        self.init(
            call: update.call,
            state: state,
            you: update.you == 0 ? nil : update.you,
            members: UnsafeBufferPointer(start: update.members, count: update.members_len).map(Member.init),
            speaking: Set(UnsafeBufferPointer(start: update.speaking, count: update.speaking_len)),
            lost: update.lost.map { EngineError($0.pointee) },
            present: UnsafeBufferPointer(start: update.present, count: update.present_len).map(Member.init)
        )
    }
}

nonisolated extension EngineError {
    fileprivate init(_ error: BukoSpeakError) {
        let reason: Reason =
            switch error.reason {
            case BUKO_SPEAK_REASON_OTHER: .other
            case BUKO_SPEAK_REASON_STRANGER: .stranger
            case BUKO_SPEAK_REASON_UNIDENTIFIED: .unidentified
            case BUKO_SPEAK_REASON_REPLACED: .replaced
            default: fatalError("the engine has no other reasons")
            }
        self.init(reason: reason, message: String(error.message))
    }
}
