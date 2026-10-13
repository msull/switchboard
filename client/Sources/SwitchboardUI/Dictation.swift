import Foundation
import Observation

#if canImport(Speech) && canImport(AVFoundation)
import AVFoundation
import Speech
#endif

/// Dictation into the message field, started by its microphone button.
/// The system keyboard's own mic is still there; this one starts at
/// once, without the keyboard, and keeps the text it hears beside what
/// was typed. On-device recognition is asked for where the device has
/// it. visionOS gives an app no gaze or dwell event, and "look to
/// dictate" is a system affordance of search fields only, so the
/// button is a tap.
@MainActor @Observable
public final class Dictation {
    /// Listening now.
    public private(set) var listening = false
    /// What has been heard since the start, as the recognizer has it
    /// so far; it revises earlier words as more arrive.
    public private(set) var heard = ""
    /// Why dictation could not run, for the window.
    public private(set) var problem: String?

    public init() {}

    /// The field's text with what was heard appended once, separated
    /// by a space when both sides have one to give.
    nonisolated public static func joined(_ typed: String, _ heard: String) -> String {
        let heard = heard.trimmingCharacters(in: .whitespaces)
        if heard.isEmpty { return typed }
        if typed.isEmpty { return heard }
        if typed.hasSuffix(" ") || typed.hasSuffix("\n") { return typed + heard }
        return typed + " " + heard
    }

    #if canImport(Speech) && canImport(AVFoundation)
    private let engine = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?

    public func toggle() async {
        if listening { stop() } else { await start() }
    }

    public func start() async {
        problem = nil
        heard = ""
        guard let recognizer = SFSpeechRecognizer(), recognizer.isAvailable else {
            problem = "Speech recognition is not available."
            return
        }
        let speech = await withCheckedContinuation { (c: CheckedContinuation<SFSpeechRecognizerAuthorizationStatus, Never>) in
            SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) }
        }
        guard speech == .authorized else {
            problem = "Allow speech recognition for Switchboard in Settings."
            return
        }
        guard await Self.microphoneAllowed() else {
            problem = "Allow the microphone for Switchboard in Settings."
            return
        }
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        if recognizer.supportsOnDeviceRecognition {
            request.requiresOnDeviceRecognition = true
        }
        do {
            try Self.activateAudioSession()
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            input.removeTap(onBus: 0)
            input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
                request.append(buffer)
            }
            engine.prepare()
            try engine.start()
        } catch {
            problem = "The microphone could not start: \(error.localizedDescription)"
            engine.inputNode.removeTap(onBus: 0)
            return
        }
        self.request = request
        listening = true
        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            Task { @MainActor [weak self] in
                guard let self else { return }
                if let result {
                    self.heard = result.bestTranscription.formattedString
                    if result.isFinal { self.stop() }
                }
                if let error, self.listening {
                    // A cancel on stop reports as an error too; only a
                    // failure while listening is worth a line.
                    self.problem = error.localizedDescription
                    self.stop()
                }
            }
        }
    }

    public func stop() {
        guard listening else { return }
        listening = false
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        request?.endAudio()
        task?.cancel()
        task = nil
        request = nil
        Self.deactivateAudioSession()
    }

    // The audio session is an iOS and visionOS thing; the package also
    // builds on the Mac for the tests, where the engine runs without one.
    #if os(iOS) || os(visionOS)
    private static func microphoneAllowed() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    private static func activateAudioSession() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .measurement, options: .duckOthers)
        try session.setActive(true, options: .notifyOthersOnDeactivation)
    }

    private static func deactivateAudioSession() {
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
    #else
    private static func microphoneAllowed() async -> Bool { true }
    private static func activateAudioSession() throws {}
    private static func deactivateAudioSession() {}
    #endif
    #else
    public func toggle() async { problem = "Dictation needs the Speech framework." }
    public func start() async { problem = "Dictation needs the Speech framework." }
    public func stop() {}
    #endif
}
