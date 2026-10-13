import Foundation

// The wire crate's views (`control/src/lib.rs`), field for field. A
// field the host added later is optional or defaulted here, as it is
// there, so an older host still decodes.

public enum SessionKind: String, Codable, Sendable {
    case claude, codex, shell, command, service
}

/// Whether the pane exists and what it is doing. Serde's external
/// tagging: `"running"`, `"missing"`, or `{"exited": {"code": n}}`.
public enum Liveness: Equatable, Sendable, Decodable {
    case running
    case exited(code: Int?)
    case missing

    public init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer(),
            let word = try? single.decode(String.self)
        {
            switch word {
            case "running": self = .running
            case "missing": self = .missing
            default:
                throw DecodingError.dataCorruptedError(
                    in: single, debugDescription: "unknown liveness \(word)")
            }
            return
        }
        let container = try decoder.container(keyedBy: Key.self)
        let exited = try container.nestedContainer(keyedBy: Key.self, forKey: .exited)
        self = .exited(code: try exited.decodeIfPresent(Int.self, forKey: .code))
    }

    private enum Key: String, CodingKey { case exited, code }
}

public struct SessionView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let project: String
    public let name: String
    public let kind: SessionKind
    public let cwd: String
    public let notes: String
    public let liveness: Liveness
    /// The card's word: "working", "idle", "waiting on you", ...
    public let card: String
    public let lastExit: Int?
    public let lastStopAtMs: UInt64?
    public let quietSecs: UInt64?
    public let waiting: Bool
    public let waitingReason: String?
    public let trustQuestion: Bool
    public let promptRefusal: String?
    public let resumeID: String?
    public let op: String?
    public let pending: [String]
    public let wakeupAtMs: UInt64?
    public let heldUntilMs: UInt64?
    /// The fields the host cut to its string bound.
    public let truncated: [String]

    enum CodingKeys: String, CodingKey {
        case id, project, name, kind, cwd, notes, liveness, card, waiting, op, pending, truncated
        case lastExit = "last_exit"
        case lastStopAtMs = "last_stop_at_ms"
        case quietSecs = "quiet_secs"
        case waitingReason = "waiting_reason"
        case trustQuestion = "trust_question"
        case promptRefusal = "prompt_refusal"
        case resumeID = "resume_id"
        case wakeupAtMs = "wakeup_at_ms"
        case heldUntilMs = "held_until_ms"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        project = try c.decode(String.self, forKey: .project)
        name = try c.decode(String.self, forKey: .name)
        kind = try c.decode(SessionKind.self, forKey: .kind)
        cwd = try c.decode(String.self, forKey: .cwd)
        notes = try c.decode(String.self, forKey: .notes)
        liveness = try c.decode(Liveness.self, forKey: .liveness)
        card = try c.decode(String.self, forKey: .card)
        lastExit = try c.decodeIfPresent(Int.self, forKey: .lastExit)
        lastStopAtMs = try c.decodeIfPresent(UInt64.self, forKey: .lastStopAtMs)
        quietSecs = try c.decodeIfPresent(UInt64.self, forKey: .quietSecs)
        waiting = try c.decode(Bool.self, forKey: .waiting)
        waitingReason = try c.decodeIfPresent(String.self, forKey: .waitingReason)
        trustQuestion = try c.decodeIfPresent(Bool.self, forKey: .trustQuestion) ?? false
        promptRefusal = try c.decodeIfPresent(String.self, forKey: .promptRefusal)
        resumeID = try c.decodeIfPresent(String.self, forKey: .resumeID)
        op = try c.decodeIfPresent(String.self, forKey: .op)
        pending = try c.decodeIfPresent([String].self, forKey: .pending) ?? []
        wakeupAtMs = try c.decodeIfPresent(UInt64.self, forKey: .wakeupAtMs)
        heldUntilMs = try c.decodeIfPresent(UInt64.self, forKey: .heldUntilMs)
        truncated = try c.decodeIfPresent([String].self, forKey: .truncated) ?? []
    }

    /// A view built by hand, for tests and previews.
    public init(
        id: String, project: String, name: String, kind: SessionKind = .claude,
        cwd: String = "/", notes: String = "", liveness: Liveness = .missing,
        card: String = "not running", waiting: Bool = false, waitingReason: String? = nil,
        lastStopAtMs: UInt64? = nil, resumeID: String? = nil
    ) {
        self.id = id
        self.project = project
        self.name = name
        self.kind = kind
        self.cwd = cwd
        self.notes = notes
        self.liveness = liveness
        self.card = card
        self.lastExit = nil
        self.lastStopAtMs = lastStopAtMs
        self.quietSecs = nil
        self.waiting = waiting
        self.waitingReason = waitingReason
        self.trustQuestion = false
        self.promptRefusal = nil
        self.resumeID = resumeID
        self.op = nil
        self.pending = []
        self.wakeupAtMs = nil
        self.heldUntilMs = nil
        self.truncated = []
    }
}

/// The host's own order for cards: waiting first, then working,
/// starting, idle, exited, not running (`CardState::rank`).
public enum CardRank {
    public static func rank(of card: String) -> Int {
        switch card {
        case "waiting on you": return 0
        case "working": return 1
        case "starting": return 2
        case "idle": return 3
        case "exited": return 4
        case "not running": return 5
        default: return 6
        }
    }
}

public struct ProjectView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let root: String
    public let space: String
    public let op: String?

    public init(id: String, name: String, root: String = "/", space: String, op: String? = nil) {
        self.id = id
        self.name = name
        self.root = root
        self.space = space
        self.op = op
    }
}

public struct SpaceView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let op: String?
    /// The global space: a view of every space rather than a record.
    public let view: Bool

    enum CodingKeys: String, CodingKey { case id, name, op, view }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        op = try c.decodeIfPresent(String.self, forKey: .op)
        view = try c.decodeIfPresent(Bool.self, forKey: .view) ?? false
    }

    public init(id: String, name: String, view: Bool = false) {
        self.id = id
        self.name = name
        self.op = nil
        self.view = view
    }
}

/// Grid units, not pixels: a hint for the client's own layout.
public struct Rect: Decodable, Sendable, Equatable {
    public let x: UInt32
    public let y: UInt32
    public let w: UInt32
    public let h: UInt32
}

/// `{"session": {"session": id}}` or `{"file": {"project": id, "path": p}}`.
public enum PinTarget: Equatable, Sendable, Decodable {
    case session(String)
    case file(project: String, path: String)

    private enum Key: String, CodingKey { case session, file, project, path }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Key.self)
        if c.contains(.session) {
            let inner = try c.nestedContainer(keyedBy: Key.self, forKey: .session)
            self = .session(try inner.decode(String.self, forKey: .session))
        } else {
            let inner = try c.nestedContainer(keyedBy: Key.self, forKey: .file)
            self = .file(
                project: try inner.decode(String.self, forKey: .project),
                path: try inner.decode(String.self, forKey: .path))
        }
    }
}

public struct Pin: Decodable, Sendable, Equatable {
    public let target: PinTarget
    public let rect: Rect
}

/// What chooses a rule set's cards.
public enum SetRule: Equatable, Sendable, Decodable {
    case recent(hours: UInt32)

    private enum Key: String, CodingKey { case kind, hours }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Key.self)
        let kind = try c.decode(String.self, forKey: .kind)
        switch kind {
        case "recent": self = .recent(hours: try c.decode(UInt32.self, forKey: .hours))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: c, debugDescription: "unknown rule \(kind)")
        }
    }
}

public struct SetView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let space: String
    public let items: [Pin]
    public let op: String?
    public let rule: SetRule?

    enum CodingKeys: String, CodingKey { case id, name, space, items, op, rule }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        space = try c.decode(String.self, forKey: .space)
        items = try c.decode([Pin].self, forKey: .items)
        op = try c.decodeIfPresent(String.self, forKey: .op)
        rule = try c.decodeIfPresent(SetRule.self, forKey: .rule)
    }

    public init(id: String, name: String, space: String) {
        self.id = id
        self.name = name
        self.space = space
        self.items = []
        self.op = nil
        self.rule = nil
    }
}

// The reply bodies: the wire `Reply` with its tag removed.

public struct ProjectsBody: Decodable, Sendable {
    public let projects: [ProjectView]
    public init(projects: [ProjectView]) { self.projects = projects }
}
public struct SpacesBody: Decodable, Sendable {
    public let spaces: [SpaceView]
    public init(spaces: [SpaceView]) { self.spaces = spaces }
}
public struct SetsBody: Decodable, Sendable {
    public let sets: [SetView]
    public init(sets: [SetView]) { self.sets = sets }
}
public struct SessionsBody: Decodable, Sendable {
    public let sessions: [SessionView]
    public init(sessions: [SessionView]) { self.sessions = sessions }
}
public struct SessionBody: Decodable, Sendable {
    public let session: SessionView
    public init(session: SessionView) { self.session = session }
}
public struct ScreenBody: Decodable, Sendable {
    public let text: String
    public let truncated: Bool

    public init(text: String, truncated: Bool = false) {
        self.text = text
        self.truncated = truncated
    }

    enum CodingKeys: String, CodingKey { case text, truncated }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decode(String.self, forKey: .text)
        truncated = try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false
    }
}
