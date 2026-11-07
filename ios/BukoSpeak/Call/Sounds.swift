import AVFAudio
import Synchronization
import os

// The server's sounds for the cues, in place of their tones; off the main thread, as they
// download and decode.
nonisolated enum Sounds {
    private static let folder = URL.applicationSupportDirectory.appending(path: "sounds", directoryHint: .isDirectory)
    private static let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!
    private static let log = Logger(subsystem: "digital.buko.BukoSpeak", category: "sounds")

    static func load(into engine: Engine) {
        for cue in Cue.allCases {
            engine.setSound(cue, samples(of: folder.appending(path: cue.file).appendingPathExtension("wav")))
        }
    }

    static func sync(from server: String, into engine: Engine) {
        do throws(SoundsError) {
            try Engine.syncSounds(server: server, into: folder)
        } catch {
            // Without them, the cues keep their tones.
            return log.error("installing the server's sounds: \(error.message)")
        }
        load(into: engine)
    }

    private static func samples(of file: URL) -> [Float]? {
        guard FileManager.default.fileExists(atPath: file.path(percentEncoded: false)) else { return nil }
        do {
            return try decoded(file)
        } catch {
            log.error("\(file.lastPathComponent): \(error)")
            return nil
        }
    }

    private static func decoded(_ file: URL) throws -> [Float] {
        let source = try AVAudioFile(forReading: file)
        guard
            let read = AVAudioPCMBuffer(
                pcmFormat: source.processingFormat,
                frameCapacity: AVAudioFrameCount(source.length)
            ),
            let converter = AVAudioConverter(from: source.processingFormat, to: format)
        else { throw CocoaError(.fileReadCorruptFile) }
        try source.read(into: read)
        converter.downmix = true
        let ratio = format.sampleRate / source.processingFormat.sampleRate
        guard
            let converted = AVAudioPCMBuffer(
                pcmFormat: format,
                frameCapacity: AVAudioFrameCount(Double(read.frameLength) * ratio) + 1
            )
        else { throw CocoaError(.fileReadCorruptFile) }
        let unread = Mutex<AVAudioPCMBuffer?>(read)
        var failure: NSError?
        converter.convert(to: converted, error: &failure) { _, status in
            let buffer = unread.withLock { $0.take() }
            status.pointee = buffer == nil ? .endOfStream : .haveData
            return buffer
        }
        if let failure { throw failure }
        return Array(UnsafeBufferPointer(start: converted.floatChannelData![0], count: Int(converted.frameLength)))
    }
}
