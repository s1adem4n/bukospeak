import AVFAudio
import CallKit
import Observation
import os

// Through CallKit the system keeps the app and microphone while the screen is locked and
// gives them back after another call.
@Observable
final class Call: NSObject {
    enum Phase: Equatable {
        case idle
        case joining
        case connected
        case reconnecting
        case failed(String)
    }

    private(set) var phase = Phase.idle
    private(set) var members: [Member] = []
    private(set) var speaking: Set<UInt16> = []
    private(set) var you: UInt16?
    private(set) var isMuted = false
    private(set) var isHeld = false
    private(set) var present: [Member] = []
    private(set) var since: Date?

    @ObservationIgnored let settings: Settings
    @ObservationIgnored private let engine: Engine
    @ObservationIgnored private let controller = CXCallController()
    @ObservationIgnored private lazy var provider: CXProvider = {
        let provider = CXProvider(configuration: Self.configuration)
        provider.setDelegate(self, queue: nil)
        return provider
    }()
    @ObservationIgnored private var id: UUID?
    @ObservationIgnored private var isSessionActive = false
    @ObservationIgnored private var isInterrupted = false
    @ObservationIgnored private var isPlaying = false
    @ObservationIgnored private var leftCall: UInt64 = 0
    @ObservationIgnored private var engineState = ChannelUpdate.State.out
    @ObservationIgnored private var leaving: Task<Void, Never>?
    @ObservationIgnored private var joining: Task<Result<Void, EngineError>, Never>?
    @ObservationIgnored private var tapped = ContinuousClock.now

    private static let log = Logger(subsystem: "digital.buko.BukoSpeak", category: "call")

    init(settings: Settings, engine: Engine) {
        self.settings = settings
        self.engine = engine
        super.init()
        engine.setVolume(settings.volume)
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

    func join() async {
        guard id == nil, phase != .joining else { return }
        tapped = .now
        phase = .joining
        members = present
        await leaving?.value
        guard await AudioSession.microphoneAllowed() else {
            members = []
            phase = .failed("Mikrofon nicht erlaubt")
            return
        }
        let id = UUID()
        self.id = id
        // Runs while CallKit starts the call, which takes about as long.
        joining = Task { await joinChannel() }
        let start = CXStartCallAction(call: id, handle: CXHandle(type: .generic, value: "BukoSpeak"))
        // Set before every call: iOS may otherwise keep a stale audio session and never
        // activate the new call's (https://developer.apple.com/forums/thread/837211).
        provider.configuration = Self.configuration
        do {
            try await controller.request(CXTransaction(action: start))
        } catch {
            Self.log.error("the call didn't start: \(error)")
            end(failure: "Anruf nicht gestartet")
        }
    }

    func leave() async {
        guard let id else { return }
        end(failure: nil)
        await request(CXEndCallAction(call: id))
    }

    func toggleMute() async {
        guard let id else { return }
        await request(CXSetMutedCallAction(call: id, muted: !isMuted))
    }

    func watch(_ on: Bool) {
        if on, id == nil, settings.isComplete {
            engine.watch(server: settings.server.trimmingCharacters(in: .whitespaces))
        } else {
            engine.unwatch()
        }
    }

    func noiseSuppressionChanged() {
        engine.setNoiseSuppression(settings.noiseSuppression)
    }

    func volumeChanged() {
        engine.setVolume(settings.volume)
    }

    private static var configuration: CXProviderConfiguration {
        let configuration = CXProviderConfiguration()
        configuration.supportsVideo = false
        configuration.maximumCallsPerCallGroup = 1
        configuration.supportedHandleTypes = [.generic]
        configuration.includesCallsInRecents = false
        return configuration
    }

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

    private func joinChannel() async -> Result<Void, EngineError> {
        engine.setNoiseSuppression(settings.noiseSuppression)
        engine.setMuted(isMuted)
        do throws(EngineError) {
            try await engine.join(server: settings.server.trimmingCharacters(in: .whitespaces))
            logStep("the server welcomed the join")
            return .success(())
        } catch {
            return .failure(error)
        }
    }

    private func logStep(_ step: String) {
        let milliseconds = Int((ContinuousClock.now - tapped) / .milliseconds(1))
        Self.log.info("\(step, privacy: .public): \(milliseconds) ms after the tap")
    }

    private func connected() {
        phase = engineState == .reconnecting ? .reconnecting : .connected
        if since == nil { since = .now }
        updateAudio()
    }

    private func update(_ update: ChannelUpdate) {
        if present != update.present { present = update.present }
        // Updates of a call already left can still arrive.
        guard id != nil, update.call > leftCall else { return }
        let changed = update.state != engineState
        engineState = update.state
        // Set only when changed: each set redraws the screen, and a redraw during a tap loses it.
        if you != update.you { you = update.you }
        if members != update.members { members = update.members }
        if speaking != update.speaking { speaking = update.speaking }
        if let lost = update.lost {
            Self.log.error("the call was lost: \(lost.message)")
            return fail(lost.text(otherwise: "Verbindung verloren"))
        }
        // Only on a change: an update of who speaks shouldn't start audio that failed again.
        if changed && phase != .joining {
            connected()
        }
    }

    // CallKit may activate the session before the join is done.
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

    private func audioStarted(_ failure: AudioError?, for call: UUID?) {
        guard id == call else { return }
        if let failure {
            Self.log.error("the microphone and speaker didn't start: \(failure.message)")
            return fail("Audio nicht gestartet")
        }
        logStep("the call's audio plays")
    }

    private func fail(_ reason: String) {
        guard let id else { return }
        provider.reportCall(with: id, endedAt: nil, reason: .failed)
        end(failure: reason)
    }

    private func end(failure: String?) {
        // First, so a join under way sees the call is over.
        id = nil
        // Not cancelled: the leave below waits for it, then leaves the call it joined.
        joining = nil
        // The leave stops the audio off the main thread: disposing the voice unit freezes
        // the screen.
        isPlaying = false
        engineState = .out
        members = []
        speaking = []
        you = nil
        isMuted = false
        engine.setMuted(false)
        isHeld = false
        since = nil
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

    // The one reactivation CallKit leaves to the app; on failure the call waits for CallKit.
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
                end(failure: error.text(otherwise: "Beitritt fehlgeschlagen"))
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

extension EngineError {
    func text(otherwise: String) -> String {
        switch reason {
        case .stranger: "Nicht im Tailnet des Servers"
        case .unidentified: "Server erreicht Tailscale nicht"
        case .replaced: "Anderswo beigetreten"
        case .other: otherwise
        }
    }
}
