import AVFAudio
import CallKit
import Observation
import os

/// The call, which joins and leaves the channel through CallKit: the system treats it like a
/// phone call, so it keeps the app and the microphone while the screen is locked and gives
/// them back after another call.
@Observable
final class Call: NSObject {
    enum Phase: Equatable {
        case idle
        case joining
        case connected
        /// The connection was lost while the call was up, and the call joins again.
        case reconnecting
        case failed(String)
    }

    private(set) var phase = Phase.idle
    /// Everyone in the channel, this member included.
    private(set) var members: [Member] = []
    private(set) var speaking: Set<UInt16> = []
    private(set) var you: UInt16?
    private(set) var isMuted = false
    /// Whether another call holds this one.
    private(set) var isHeld = false
    /// The server whose certificate changed since it was pinned, for the user to trust anew.
    private(set) var changedServer: String?

    @ObservationIgnored let settings: Settings
    @ObservationIgnored private let engine: Engine
    @ObservationIgnored private let controller = CXCallController()
    /// Made on the first join: a provider tells the system the app makes calls.
    @ObservationIgnored private lazy var provider: CXProvider = {
        let provider = CXProvider(configuration: Self.configuration)
        provider.setDelegate(self, queue: nil)
        return provider
    }()
    /// The call CallKit knows, from asking it to start until it ended.
    @ObservationIgnored private var id: UUID?
    /// Whether CallKit activated the audio session for the call.
    @ObservationIgnored private var isSessionActive = false
    /// Whether an interruption took the active session away.
    @ObservationIgnored private var isInterrupted = false
    /// Whether the call plays on the microphone and speaker.
    @ObservationIgnored private var isPlaying = false

    private static let log = Logger(subsystem: "digital.buko.BukoSpeak", category: "call")

    init(settings: Settings, engine: Engine) {
        self.settings = settings
        self.engine = engine
        super.init()
        controller.callObserver.setDelegate(self, queue: nil)
        Task { [weak self, events = engine.events] in
            for await event in events {
                self?.handle(event)
            }
        }
        Task { [weak self] in
            for await _ in NotificationCenter.default.notifications(
                named: AVAudioSession.mediaServicesWereResetNotification
            ) {
                self?.mediaServicesWereReset()
            }
        }
        Task { [weak self] in
            for await notification in NotificationCenter.default.notifications(
                named: AVAudioSession.interruptionNotification
            ) {
                self?.interrupted(notification)
            }
        }
    }

    /// Starts a call that joins the channel.
    func join() async {
        guard id == nil, phase != .joining else { return }
        phase = .joining
        changedServer = nil
        guard await AudioSession.microphoneAllowed() else {
            phase = .failed("BukoSpeak may not use the microphone. Allow it in the Settings app.")
            return
        }
        let id = UUID()
        self.id = id
        let start = CXStartCallAction(call: id, handle: CXHandle(type: .generic, value: "BukoSpeak"))
        // Configured again before every call: iOS can otherwise hold on to a stale audio session
        // for the provider and never activate the new call's.
        provider.configuration = Self.configuration
        do {
            try await controller.request(CXTransaction(action: start))
        } catch {
            self.id = nil
            phase = .failed("The call didn't start: \(error.localizedDescription)")
        }
    }

    /// Ends the call.
    func leave() async {
        guard let id else { return }
        await request(CXEndCallAction(call: id))
    }

    func toggleMute() async {
        guard let id else { return }
        await request(CXSetMutedCallAction(call: id, muted: !isMuted))
    }

    /// Applies the noise suppression setting to the call.
    func noiseSuppressionChanged() {
        engine.setNoiseSuppression(settings.noiseSuppression)
    }

    /// Trusts the server's new certificate on the next join, as after it was set up anew.
    func trustAnew(_ server: String) {
        settings.forgetPin(for: server)
        changedServer = nil
        phase = .idle
    }

    private static var configuration: CXProviderConfiguration {
        let configuration = CXProviderConfiguration()
        configuration.supportsVideo = false
        configuration.maximumCallsPerCallGroup = 1
        configuration.supportedHandleTypes = [.generic]
        configuration.includesCallsInRecents = false
        return configuration
    }

    /// What the call can do: be held for another call, nothing more.
    private static var capabilities: CXCallUpdate {
        let update = CXCallUpdate()
        update.localizedCallerName = "BukoSpeak"
        update.supportsHolding = true
        update.supportsGrouping = false
        update.supportsUngrouping = false
        update.supportsDTMF = false
        update.hasVideo = false
        return update
    }

    private func request(_ action: CXAction) async {
        do {
            try await controller.request(CXTransaction(action: action))
        } catch {
            Self.log.error("CallKit refused \(action): \(error)")
        }
    }

    /// Joins the channel with the settings, pinning the server on the first join.
    private func joinChannel() async throws(JoinError) -> Joined {
        let server = settings.server.trimmingCharacters(in: .whitespaces)
        engine.setNoiseSuppression(settings.noiseSuppression)
        do throws(JoinError) {
            let joined = try await engine.join(
                server: server,
                name: settings.name.trimmingCharacters(in: .whitespaces),
                roomKey: settings.roomKey,
                pin: settings.pin(for: server)
            )
            settings.pin(joined.fingerprint, for: server)
            return joined
        } catch {
            if case .fingerprintChanged = error {
                changedServer = server
            }
            throw error
        }
    }

    private func connected(_ joined: Joined) {
        you = joined.you
        members = joined.members
        speaking = []
        phase = .connected
        updateAudio()
    }

    private func handle(_ event: EngineEvent) {
        guard phase == .connected else { return }
        switch event {
        case .joined(let member):
            members.append(member)
        case .left(let id):
            members.removeAll { $0.id == id }
            speaking.remove(id)
        case .speaking(let now):
            speaking = now
        case .disconnected(let reason):
            Task { await reconnect(after: reason) }
        }
    }

    /// Joins again, once, when the connection was lost while the call was up, as when iOS
    /// suspended the app through another call; ends the call if that fails too.
    private func reconnect(after reason: String) async {
        guard let id else { return }
        phase = .reconnecting
        speaking = []
        updateAudio()
        do {
            let joined = try await joinChannel()
            guard self.id == id else { return }
            // A join starts unmuted.
            engine.setMuted(isMuted)
            connected(joined)
        } catch {
            guard self.id == id else { return }
            provider.reportCall(with: id, endedAt: nil, reason: .failed)
            await end(failure: "The connection was lost: \(reason)")
        }
    }

    /// Plays the call on the microphone and speaker exactly while it is connected and its
    /// audio session active: CallKit may activate the session before the join is done.
    private func updateAudio() {
        let plays = phase == .connected && isSessionActive && !isInterrupted
        guard plays != isPlaying else { return }
        if plays {
            do throws(AudioError) {
                try engine.startAudio()
            } catch {
                return fail("The microphone and speaker didn't start: \(error.message)")
            }
            Self.log.info("the call's audio plays")
        } else {
            engine.stopAudio()
            Self.log.info("the call's audio stopped")
        }
        isPlaying = plays
    }

    /// Ends a call that is up, after telling CallKit it failed.
    private func fail(_ reason: String) {
        guard let id else { return }
        provider.reportCall(with: id, endedAt: nil, reason: .failed)
        Task { await end(failure: reason) }
    }

    /// Leaves the channel and forgets the call, with why if it failed.
    private func end(failure: String?) async {
        // First, so a join or a reconnect under way sees the call is over.
        id = nil
        // Leaving stops the call's audio.
        await engine.leave()
        isPlaying = false
        members = []
        speaking = []
        you = nil
        isMuted = false
        isHeld = false
        phase = failure.map(Phase.failed) ?? .idle
    }

    private func interrupted(_ notification: Notification) {
        guard
            isSessionActive,
            let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
            let type = AVAudioSession.InterruptionType(rawValue: raw)
        else { return }
        Self.log.info("interruption \(raw)")
        switch type {
        case .began:
            isInterrupted = true
            updateAudio()
        case .ended:
            reactivateSession()
        @unknown default:
            break
        }
    }

    private func mediaServicesWereReset() {
        guard id != nil else { return }
        Self.log.info("media services were reset")
        isInterrupted = true
        updateAudio()
        do {
            try AudioSession.configure()
        } catch {
            return fail(
                "The audio system restarted, and the call's audio couldn't be set up again: \(error.localizedDescription)"
            )
        }
        if isSessionActive {
            reactivateSession()
        }
    }

    /// Activates the audio session again after it was taken away, the one case CallKit leaves
    /// to the app.
    private func reactivateSession() {
        do {
            try AVAudioSession.sharedInstance().setActive(true)
        } catch {
            return fail("The call's audio didn't come back: \(error.localizedDescription)")
        }
        isInterrupted = false
        updateAudio()
    }
}

extension Call: @MainActor CXProviderDelegate {
    func providerDidReset(_ provider: CXProvider) {
        // CallKit ended every call.
        isSessionActive = false
        Task { await end(failure: nil) }
    }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        do {
            try AudioSession.configure()
        } catch {
            action.fail()
            id = nil
            phase = .failed("The audio couldn't be set up: \(error.localizedDescription)")
            return
        }
        provider.reportOutgoingCall(with: action.callUUID, startedConnectingAt: nil)
        Task {
            do throws(JoinError) {
                let joined = try await joinChannel()
                guard id == action.callUUID else { return action.fail() }
                connected(joined)
                action.fulfill()
                provider.reportOutgoingCall(with: action.callUUID, connectedAt: nil)
                provider.reportCall(with: action.callUUID, updated: Self.capabilities)
            } catch {
                action.fail()
                guard id == action.callUUID else { return }
                await end(failure: error.message)
            }
        }
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        Task {
            await end(failure: nil)
            action.fulfill()
        }
    }

    func provider(_ provider: CXProvider, perform action: CXSetHeldCallAction) {
        // The audio follows the session, which CallKit deactivates and activates.
        isHeld = action.isOnHold
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        isMuted = action.isMuted
        engine.setMuted(action.isMuted)
        action.fulfill()
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        Self.log.info("CallKit activated the audio session")
        isSessionActive = true
        isInterrupted = false
        updateAudio()
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        Self.log.info("CallKit deactivated the audio session")
        isSessionActive = false
        updateAudio()
    }
}

extension Call: @MainActor CXCallObserverDelegate {
    func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) {
        // When the call that held this one ends, iOS resumes this one only if the user hung up;
        // if the other side did, it stays held until asked.
        guard let id, call.uuid != id, call.hasEnded, isHeld else { return }
        Task { await request(CXSetHeldCallAction(call: id, onHold: false)) }
    }
}

extension JoinError {
    /// What went wrong, for the user.
    var message: String {
        switch self {
        case .fingerprintChanged(_, _, let message), .failed(let message): message
        }
    }
}
