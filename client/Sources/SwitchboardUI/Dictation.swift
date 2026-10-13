import Foundation
import Observation

#if canImport(Speech) && canImport(AVFoundation)
import AVFoundation
import Speech
#endif

/// What dictation has heard, as utterances: the ones finished, joined
/// and kept, and the one in progress, revised in place until the
/// recognizer moves on. Apple's recognizer does not keep one running
/// transcription across a pause; after silence its results cover only
/// the new phrase. So a result is taken as a revision of the live
/// utterance while it starts where the live one started, and as the
/// next utterance once it starts later, which commits the live text
/// first. Pure, so the Mac tests cover it without the Speech framework.
public struct Utterances: Equatable, Sendable {
    /// The finished utterances, joined.
    public var committed = ""
    /// The utterance in progress, as last revised.
    public var live = ""
    /// Where the live utterance starts in the recognizer's audio, to
    /// tell a revision from the next utterance.
    public var liveStart: TimeInterval?
    /// The live utterance was edited by hand, so its revisions are
    /// dropped until the next utterance starts.
    public var ignoring = false

    public init() {}

    /// Everything heard so far.
    public var heard: String { Dictation.joined(committed, live) }

    /// Takes a result: `text` starting at `start` seconds into the
    /// audio; `finished` when the recognizer has closed the utterance.
    /// A result that starts after the live utterance did is the next
    /// one, so the live text is committed first.
    public mutating func take(_ text: String, start: TimeInterval?, finished: Bool) {
        if let liveStart, let start, start > liveStart + Self.slack {
            commit()
        } else if ignoring {
            if finished { commit() }
            return
        }
        live = text
        if liveStart == nil { liveStart = start }
        if finished { commit() }
    }

    /// Moves the live utterance into the finished text.
    public mutating func commit() {
        committed = Dictation.joined(committed, live)
        live = ""
        liveStart = nil
        ignoring = false
    }

    /// The text is in the field now (the user edited around it), so
    /// what was heard is no longer separate from what was typed. The
    /// live utterance keeps revising after the field's text when the
    /// edit left it whole; an edit inside it keeps the edit, and the
    /// recognizer's revisions of that utterance are dropped.
    public mutating func absorb(keepingLive keep: Bool) {
        committed = ""
        if !keep {
            live = ""
            ignoring = liveStart != nil
        }
    }

    /// How much later a result may start and still be the same
    /// utterance: the recognizer re-times its first word a little as
    /// it revises.
    static let slack: TimeInterval = 0.4
}

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
    /// What has been heard since the start: the finished utterances
    /// and the live one, which the recognizer revises as more arrives.
    public private(set) var heard = ""
    /// The live utterance alone, for a view that marks it.
    public private(set) var live = ""
    /// Why dictation could not run, for the window.
    public private(set) var problem: String?

    private var utterances = Utterances()
    /// Counts starts and stops, so a result from a task that was
    /// stopped or replaced is dropped.
    private var generation = 0

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

    /// The field was edited by hand while listening: what was heard
    /// is now part of the field's text. The live utterance goes on
    /// revising after it when the edit left it whole at the end.
    public func absorb(keepingLive keep: Bool) {
        utterances.absorb(keepingLive: keep)
        publish()
    }

    private func publish() {
        heard = utterances.heard
        live = utterances.live
    }

    #if canImport(Speech) && canImport(AVFoundation)
    private let engine = AVAudioEngine()
    /// The request the microphone tap feeds, replaced when the
    /// recognizer ends a task and listening goes on with a new one.
    private let feed = Feed()
    private var task: SFSpeechRecognitionTask?
    private var recognizer: SFSpeechRecognizer?

    public func toggle() async {
        if listening { stop() } else { await start() }
    }

    public func start() async {
        problem = nil
        utterances = Utterances()
        publish()
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
        do {
            try Self.activateAudioSession()
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            input.removeTap(onBus: 0)
            let feed = self.feed
            input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
                feed.append(buffer)
            }
            engine.prepare()
            try engine.start()
        } catch {
            problem = "The microphone could not start: \(error.localizedDescription)"
            engine.inputNode.removeTap(onBus: 0)
            return
        }
        self.recognizer = recognizer
        listening = true
        generation += 1
        beginTask()
    }

    /// One recognition task. The recognizer ends a task on its own
    /// after a long pause (its result is then final); listening goes
    /// on with the next task, into the same utterances.
    private func beginTask() {
        guard let recognizer else { return }
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.addsPunctuation = true
        if recognizer.supportsOnDeviceRecognition {
            request.requiresOnDeviceRecognition = true
        }
        feed.replace(request)
        let generation = self.generation
        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            Task { @MainActor [weak self] in
                guard let self, self.generation == generation else { return }
                if let result {
                    let start = result.bestTranscription.segments.first?.timestamp
                    self.utterances.take(
                        result.bestTranscription.formattedString, start: start, finished: result.isFinal)
                    self.publish()
                    if result.isFinal { self.beginTask() }
                } else if let error {
                    self.problem = error.localizedDescription
                    self.stop()
                }
            }
        }
    }

    public func stop() {
        guard listening else { return }
        listening = false
        generation += 1
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        feed.replace(nil)
        task?.cancel()
        task = nil
        recognizer = nil
        utterances.commit()
        publish()
        Self.deactivateAudioSession()
    }

    /// The request the audio tap appends to, swapped from the main
    /// actor while the tap runs on the audio thread.
    private final class Feed: @unchecked Sendable {
        private let lock = NSLock()
        private var request: SFSpeechAudioBufferRecognitionRequest?

        func replace(_ next: SFSpeechAudioBufferRecognitionRequest?) {
            lock.lock()
            let old = request
            request = next
            lock.unlock()
            old?.endAudio()
        }

        func append(_ buffer: AVAudioPCMBuffer) {
            lock.lock()
            let request = self.request
            lock.unlock()
            request?.append(buffer)
        }
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
