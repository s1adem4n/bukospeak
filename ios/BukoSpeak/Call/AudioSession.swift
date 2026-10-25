import AVFAudio

/// The app's audio session, as a call needs it. CallKit activates and deactivates it; the app
/// only sets it up, and activates it itself only when an interruption ends.
enum AudioSession {
    /// Voice chat on the speaker unless a headset or Bluetooth is there, at the call's rate,
    /// in the engine's 10 ms frames.
    static func configure() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP, .defaultToSpeaker])
        try session.setPreferredSampleRate(48_000)
        try session.setPreferredIOBufferDuration(0.01)
    }

    /// Asks once whether the app may use the microphone.
    static func microphoneAllowed() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }
}
