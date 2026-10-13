import Foundation

/// A session's prompt as its host's Prompt Box editor holds it (§4.5):
/// the text with the live utterance's range, the editor's status and
/// what its tools can do now. The device keeps no prompt of its own.
public struct DraftView: Codable, Sendable, Equatable {
    public var text: String
    /// The byte range of the utterance still being recognized.
    public var live: [Int]?
    /// `idle`, `listening`, `finishing`, `degraded` or `error`.
    public var status: String
    public var statusText: String?
    public var canUndo: Bool
    public var canRedo: Bool
    /// The AI pass is possible on the host.
    public var ai: Bool
    /// An AI pass is running.
    public var busy: Bool

    public init(
        text: String = "", live: [Int]? = nil, status: String = "idle", statusText: String? = nil,
        canUndo: Bool = false, canRedo: Bool = false, ai: Bool = false, busy: Bool = false
    ) {
        self.text = text
        self.live = live
        self.status = status
        self.statusText = statusText
        self.canUndo = canUndo
        self.canRedo = canRedo
        self.ai = ai
        self.busy = busy
    }

    /// The live utterance's range in the text, for a view that marks it.
    public var liveRange: Range<String.Index>? {
        guard let live, live.count == 2 else { return nil }
        let utf8 = text.utf8
        guard live[0] <= live[1], live[1] <= utf8.count,
            let start = utf8.index(utf8.startIndex, offsetBy: live[0], limitedBy: utf8.endIndex),
            let end = utf8.index(utf8.startIndex, offsetBy: live[1], limitedBy: utf8.endIndex),
            let s = start.samePosition(in: text), let e = end.samePosition(in: text)
        else { return nil }
        return s..<e
    }

    /// The live utterance's text.
    public var liveText: Substring? {
        liveRange.map { text[$0] }
    }

    public var listening: Bool { status == "listening" || status == "finishing" }

    enum CodingKeys: String, CodingKey {
        case text, live, status, ai, busy
        case statusText = "status_text"
        case canUndo = "can_undo"
        case canRedo = "can_redo"
    }
}

public struct DraftBody: Codable, Sendable, Equatable {
    public let draft: DraftView

    public init(draft: DraftView) {
        self.draft = draft
    }
}

/// The editor's tools a device may apply to a draft, besides replacing
/// its text and sending it.
public enum DraftTool: String, Sendable, CaseIterable {
    case undo, redo, clear
    case cleanUp = "clean_up"
    case deleteSentence = "delete_sentence"
    case deleteParagraph = "delete_paragraph"
}

/// One speech event as the device's recognizer produced it, for the
/// host's editor (§4.5). The recognizer session is sent beside the
/// list; `utterance`, `revision` and `text` apply to the kinds that
/// carry them.
public struct DictationEvent: Codable, Sendable, Equatable {
    public enum Kind: String, Codable, Sendable {
        case started
        case voiceStarted = "voice_started"
        case partial
        case final
        case voiceEnded = "voice_ended"
        case stopped
    }

    public var sequence: UInt64
    public var kind: Kind
    public var utterance: UInt64
    public var revision: UInt64
    public var text: String

    public init(sequence: UInt64, kind: Kind, utterance: UInt64 = 0, revision: UInt64 = 0, text: String = "") {
        self.sequence = sequence
        self.kind = kind
        self.utterance = utterance
        self.revision = revision
        self.text = text
    }
}

/// The body of a dictation request.
public struct DictationBody: Codable, Sendable, Equatable {
    public var session: UInt64
    public var events: [DictationEvent]

    public init(session: UInt64, events: [DictationEvent]) {
        self.session = session
        self.events = events
    }
}

/// The body of a draft action.
public struct DraftActionBody: Codable, Sendable, Equatable {
    public var action: String
    public var text: String?
    public var op: String?

    public init(action: String, text: String? = nil, op: String? = nil) {
        self.action = action
        self.text = text
        self.op = op
    }
}
