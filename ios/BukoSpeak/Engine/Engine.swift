import BukoSpeakFFI
import Foundation

nonisolated struct Member: Identifiable, Hashable, Sendable {
    let id: UInt16
    let name: String
    let muted: Bool
    let reconnecting: Bool
}

nonisolated struct ChannelUpdate: Sendable {
    enum State: Sendable {
        case out
        case connected
        case reconnecting
    }

    let call: UInt64
    let state: State
    let you: UInt16?
    let members: [Member]
    let speaking: Set<UInt16>
    let lost: EngineError?
    let present: [Member]
}

nonisolated struct EngineError: Error {
    let failure: Failure
    let message: String
}

nonisolated enum Cue: CaseIterable, Sendable {
    case joined
    case left
    case came
    case went
    case muted
    case unmuted

    var file: String {
        String(bukospeak_cue_file(raw))
    }

    fileprivate var raw: BukoSpeakCue {
        switch self {
        case .joined: BUKO_SPEAK_CUE_JOINED
        case .left: BUKO_SPEAK_CUE_LEFT
        case .came: BUKO_SPEAK_CUE_CAME
        case .went: BUKO_SPEAK_CUE_WENT
        case .muted: BUKO_SPEAK_CUE_MUTED
        case .unmuted: BUKO_SPEAK_CUE_UNMUTED
        }
    }
}

nonisolated struct AudioError: Error {
    let message: String
}

nonisolated struct SoundsError: Error {
    let message: String
}

// The engine may be called from any thread.
nonisolated final class Engine: @unchecked Sendable {
    let updates: AsyncStream<ChannelUpdate>

    private let engine: OpaquePointer
    private let listener: Unmanaged<Listener>
    // Joins and leaves block while the server answers, so they wait here, one after the other.
    private let queue = DispatchQueue(label: "digital.buko.BukoSpeak.engine")

    private final class Listener: Sendable {
        let continuation: AsyncStream<ChannelUpdate>.Continuation

        init(_ continuation: AsyncStream<ChannelUpdate>.Continuation) {
            self.continuation = continuation
        }
    }

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

    func join(server: String) async throws(EngineError) {
        let result = await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: self.join(server)) }
        }
        try result.get()
    }

    func leave() async -> UInt64 {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: bukospeak_engine_leave(self.engine)) }
        }
    }

    func watch(server: String) {
        bukospeak_engine_watch(engine, server)
    }

    func unwatch() {
        bukospeak_engine_unwatch(engine)
    }

    func setMuted(_ muted: Bool) {
        bukospeak_engine_set_muted(engine, muted)
    }

    func setNoiseSuppression(_ on: Bool) {
        bukospeak_engine_set_noise_suppression(engine, on)
    }

    func setVolume(_ volume: Double) {
        bukospeak_engine_set_volume(engine, Float(volume))
    }

    static let maxVolume = Double(bukospeak_max_volume())

    // Without samples, the cue's own tone.
    func setSound(_ cue: Cue, _ samples: [Float]?) {
        guard let samples else {
            return bukospeak_engine_set_sound(engine, cue.raw, nil, 0)
        }
        samples.withUnsafeBufferPointer { samples in
            bukospeak_engine_set_sound(engine, cue.raw, samples.baseAddress, samples.count)
        }
    }

    // Blocks while it downloads, so it runs off the main thread.
    static func syncSounds(server: String, into folder: URL) throws(SoundsError) {
        var message = ""
        let synced = withUnsafeMutablePointer(to: &message) { message in
            bukospeak_sync_sounds(server, folder.path(percentEncoded: false), message) { context, text in
                context!.assumingMemoryBound(to: String.self).pointee = String(text)
            }
        }
        if !synced { throw SoundsError(message: message) }
    }

    // On the queue: creating the voice processing unit takes long enough to freeze the screen.
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

    // On the queue, so it follows a start asked for before.
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
        let failure: Failure =
            switch error.reason {
            case BUKO_SPEAK_REASON_VERSION: .version
            case BUKO_SPEAK_REASON_STRANGER: .stranger
            case BUKO_SPEAK_REASON_UNIDENTIFIED: .unidentified
            case BUKO_SPEAK_REASON_FULL: .full
            case BUKO_SPEAK_REASON_UNAVAILABLE: .unavailable
            case BUKO_SPEAK_REASON_NOT_FOUND: .notFound
            case BUKO_SPEAK_REASON_NO_ANSWER: .noAnswer
            case BUKO_SPEAK_REASON_UNREACHABLE: .unreachable
            case BUKO_SPEAK_REASON_AUDIO: .audio
            case BUKO_SPEAK_REASON_CONNECTION_LOST: .connectionLost
            case BUKO_SPEAK_REASON_REPLACED: .replaced
            default: fatalError("the engine has no other reasons")
            }
        self.init(failure: failure, message: String(error.message))
    }
}
