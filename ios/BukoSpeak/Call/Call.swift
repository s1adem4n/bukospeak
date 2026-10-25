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
        /// Nothing comes from the server, or the connection was lost and the engine joins again.
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
    /// Who is in the channel, watched from the start screen.
    private(set) var present: [Member] = []

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
    /// The engine's call left last: updates of it or of one before, still on their way, are
    /// stale.
    @ObservationIgnored private var leftCall: UInt64 = 0
    /// The engine's state of the call, which may join again before its join came back here.
    @ObservationIgnored private var engineState = ChannelUpdate.State.out
    /// The leave of the call before, which a join waits for.
    @ObservationIgnored private var leaving: Task<Void, Never>?
    /// The call's join, under way from the tap while CallKit starts the call.
    @ObservationIgnored private var joining: Task<Result<Void, JoinError>, Never>?
    /// When the user tapped to join, for the log to tell how long each step of the join took.
    @ObservationIgnored private var tapped = ContinuousClock.now

    private static let log = Logger(subsystem: "digital.buko.BukoSpeak", category: "call")

    init(settings: Settings, engine: Engine) {
        self.settings = settings
        self.engine = engine
        super.init()
        controller.callObserver.setDelegate(self, queue: nil)
        Task { [weak self, updates = engine.updates] in
            for await update in updates {
                self?.update(update)
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
        tapped = .now
        phase = .joining
        // Who the start screen showed, until the server tells who is in the channel.
        members = present
        changedServer = nil
        await leaving?.value
        guard await AudioSession.microphoneAllowed() else {
            members = []
            phase = .failed("Mikrofon nicht erlaubt")
            return
        }
        let id = UUID()
        self.id = id
        // Under way while CallKit starts the call, which takes about as long as the join.
        joining = Task { await joinChannel() }
        let start = CXStartCallAction(call: id, handle: CXHandle(type: .generic, value: "BukoSpeak"))
        // Configured again before every call: iOS can otherwise hold on to a stale audio session
        // for the provider and never activate the new call's
        // (https://developer.apple.com/forums/thread/837211).
        provider.configuration = Self.configuration
        do {
            try await controller.request(CXTransaction(action: start))
        } catch {
            Self.log.error("the call didn't start: \(error)")
            end(failure: "Anruf nicht gestartet")
        }
    }

    /// Ends the call: over on the screen at once, while CallKit takes a moment to hear of it.
    func leave() async {
        guard let id else { return }
        end(failure: nil)
        await request(CXEndCallAction(call: id))
    }

    func toggleMute() async {
        guard let id else { return }
        await request(CXSetMutedCallAction(call: id, muted: !isMuted))
    }

    /// Watches who is in the channel while `on`, for the start screen: only out of a call, and
    /// only a server pinned by a join before.
    func watch(_ on: Bool) {
        let server = settings.server.trimmingCharacters(in: .whitespaces)
        if on, id == nil, settings.isComplete, let pin = settings.pin(for: server) {
            engine.watch(server: server, roomKey: settings.roomKey, pin: pin)
        } else {
            engine.unwatch()
        }
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

    /// Forgets the failure the user has seen.
    func dismissFailure() {
        guard case .failed = phase else { return }
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
    private func joinChannel() async -> Result<Void, JoinError> {
        let server = settings.server.trimmingCharacters(in: .whitespaces)
        engine.setNoiseSuppression(settings.noiseSuppression)
        engine.setMuted(isMuted)
        do throws(JoinError) {
            let joined = try await engine.join(
                server: server,
                name: settings.name.trimmingCharacters(in: .whitespaces),
                roomKey: settings.roomKey,
                pin: settings.pin(for: server)
            )
            settings.pin(joined.fingerprint, for: server)
            logStep("the server welcomed the join")
            return .success(())
        } catch {
            if case .fingerprintChanged = error {
                changedServer = server
            }
            return .failure(error)
        }
    }

    /// Logs a step of the join, with how long after the tap it came.
    private func logStep(_ step: String) {
        let milliseconds = Int((ContinuousClock.now - tapped) / .milliseconds(1))
        Self.log.info("\(step, privacy: .public): \(milliseconds) ms after the tap")
    }

    /// Follows the engine's state once the join came back.
    private func connected() {
        phase = engineState == .reconnecting ? .reconnecting : .connected
        updateAudio()
    }

    private func update(_ update: ChannelUpdate) {
        // Watched out of a call, so before the guard for one.
        if present != update.present { present = update.present }
        // Out of a call, or of one left whose last updates still arrive.
        guard id != nil, update.call > leftCall else { return }
        let changed = update.state != engineState
        engineState = update.state
        // Set only when changed: each set redraws the screen, and a redraw during a tap loses it.
        if you != update.you { you = update.you }
        if members != update.members { members = update.members }
        if speaking != update.speaking { speaking = update.speaking }
        if let lost = update.lost {
            Self.log.error("the connection was lost: \(lost)")
            return fail("Verbindung verloren")
        }
        // Only on a change: an update of who speaks shouldn't start audio that failed again.
        if changed && phase != .joining {
            connected()
        }
    }

    /// Plays the call on the microphone and speaker exactly while it is up, not held and its
    /// audio session active: CallKit may activate the session before the join is done. It plays
    /// on while reconnecting, which a dropout of seconds may be, and the engine keeps them.
    private func updateAudio() {
        let isUp = phase == .connected || phase == .reconnecting
        let plays = isUp && !isHeld && isSessionActive && !isInterrupted
        guard plays != isPlaying else { return }
        if plays {
            let call = id
            engine.startAudio { [weak self] failure in
                self?.audioStarted(failure, for: call)
            }
        } else {
            engine.stopAudio()
            Self.log.info("the call's audio stopped")
        }
        isPlaying = plays
    }

    /// Ends the call the audio was started for if it didn't start, unless that call is over.
    private func audioStarted(_ failure: AudioError?, for call: UUID?) {
        guard id == call else { return }
        if let failure {
            Self.log.error("the microphone and speaker didn't start: \(failure.message)")
            return fail("Audio nicht gestartet")
        }
        logStep("the call's audio plays")
    }

    /// Ends a call that is up, after telling CallKit it failed.
    private func fail(_ reason: String) {
        guard let id else { return }
        provider.reportCall(with: id, endedAt: nil, reason: .failed)
        end(failure: reason)
    }

    /// Forgets the call, with why if it failed, and leaves the channel: the app shows the call
    /// over at once, while the leave stops the audio, waits for the server, and a join waits for
    /// the leave.
    private func end(failure: String?) {
        // First, so a join under way sees the call is over.
        id = nil
        // Not cancelled: the leave below waits for it, then leaves the call it joined.
        joining = nil
        // The leave stops the audio off the main thread: disposing the voice unit takes long
        // enough to freeze the screen.
        isPlaying = false
        engineState = .out
        members = []
        speaking = []
        you = nil
        isMuted = false
        engine.setMuted(false)
        isHeld = false
        phase = failure.map(Phase.failed) ?? .idle
        leaving = Task { [weak self, engine] in
            let left = await engine.leave()
            self?.leftCall = left
        }
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
            let raw = notification.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
            // Otherwise the session waits for CallKit to activate it.
            if AVAudioSession.InterruptionOptions(rawValue: raw).contains(.shouldResume) {
                reactivateSession()
            }
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
            Self.log.error("the audio couldn't be set up again after media services were reset: \(error)")
            return fail("Audio nicht gestartet")
        }
        if isSessionActive {
            reactivateSession()
        }
    }

    /// Activates the audio session again after it was taken away, the one case CallKit leaves
    /// to the app; if that fails, the call waits for CallKit to activate it.
    private func reactivateSession() {
        do {
            try AVAudioSession.sharedInstance().setActive(true)
        } catch {
            return Self.log.error("the audio session didn't activate again: \(error)")
        }
        isInterrupted = false
        updateAudio()
    }
}

extension Call: @MainActor CXProviderDelegate {
    func providerDidReset(_ provider: CXProvider) {
        // CallKit ended every call.
        isSessionActive = false
        end(failure: nil)
    }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        logStep("CallKit started the call")
        guard let joining, id == action.callUUID else { return action.fail() }
        do {
            try AudioSession.configure()
        } catch {
            action.fail()
            Self.log.error("the audio couldn't be set up: \(error)")
            return end(failure: "Audio nicht gestartet")
        }
        provider.reportOutgoingCall(with: action.callUUID, startedConnectingAt: nil)
        Task {
            let joined = await joining.value
            guard id == action.callUUID else { return action.fail() }
            switch joined {
            case .success:
                connected()
                action.fulfill()
                provider.reportOutgoingCall(with: action.callUUID, connectedAt: nil)
                provider.reportCall(with: action.callUUID, updated: Self.capabilities)
            case .failure(let error):
                action.fail()
                Self.log.error("the join failed: \(error.message)")
                end(failure: "Beitritt fehlgeschlagen")
            }
        }
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        // Ended here already if the app asked; otherwise the system did, say from the lock screen.
        if id == action.callUUID {
            end(failure: nil)
        }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetHeldCallAction) {
        isHeld = action.isOnHold
        updateAudio()
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        isMuted = action.isMuted
        engine.setMuted(action.isMuted)
        action.fulfill()
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        logStep("CallKit activated the audio session")
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
    /// What went wrong, for the log.
    var message: String {
        switch self {
        case .fingerprintChanged(_, _, let message), .failed(let message): message
        }
    }
}
