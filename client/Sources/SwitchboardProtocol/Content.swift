import Foundation

// The content replies (§4.3): a conversation page, the last response,
// a command's runs and a run's output. Shaped on the host in
// `src/content.rs` and `app.rs`, not in the wire crate, so these are
// the only description of them on this side.

/// Token counts over a conversation.
public struct UsageView: Decodable, Sendable, Equatable {
    public let input: UInt64
    public let output: UInt64
    public let cacheRead: UInt64
    public let cacheCreate: UInt64

    enum CodingKeys: String, CodingKey {
        case input, output
        case cacheRead = "cache_read"
        case cacheCreate = "cache_create"
    }

    public init(input: UInt64 = 0, output: UInt64 = 0, cacheRead: UInt64 = 0, cacheCreate: UInt64 = 0) {
        self.input = input
        self.output = output
        self.cacheRead = cacheRead
        self.cacheCreate = cacheCreate
    }
}

/// What a tool was asked and what it returned, as the host capped it.
public struct ToolDetailView: Decodable, Sendable, Equatable {
    public let name: String
    public let input: String
    public let result: String
    public let path: String?

    public init(name: String, input: String, result: String, path: String? = nil) {
        self.name = name
        self.input = input
        self.result = result
        self.path = path
    }
}

public enum ActivityKindView: String, Decodable, Sendable {
    case tool, text, system
}

/// A file the agent named in its prose, as the host found it on its
/// disk under the project root: the UTF-8 byte range of the token in
/// the text as sent, the path `file.read` takes, and the line named.
public struct LinkView: Decodable, Sendable, Equatable, Hashable {
    public let start: Int
    public let end: Int
    public let path: String
    public let line: Int?

    public init(start: Int, end: Int, path: String, line: Int? = nil) {
        self.start = start
        self.end = end
        self.path = path
        self.line = line
    }
}

/// One line in a turn's activity: a tool call with its detail, text the
/// agent wrote before its answer, or a system line.
public struct ActivityView: Decodable, Sendable, Equatable, Identifiable {
    public let kind: ActivityKindView
    public let line: String
    public let atMs: UInt64?
    public let error: Bool
    public let detail: ToolDetailView?
    public let text: String?
    /// The page carries only the head of the detail or text; the turn
    /// read on its own carries it whole.
    public let more: Bool
    /// The files named in `text`, or in `line` when that is all there
    /// is; none from a host that does not find them.
    public let links: [LinkView]
    /// Position in the turn; activities are not identified on the wire.
    public var index: Int = 0

    public var id: Int { index }

    enum CodingKeys: String, CodingKey {
        case kind, line, error, detail, text, more, links
        case atMs = "at_ms"
    }

    public init(
        kind: ActivityKindView, line: String, atMs: UInt64? = nil, error: Bool = false,
        detail: ToolDetailView? = nil, text: String? = nil, more: Bool = false, links: [LinkView] = []
    ) {
        self.kind = kind
        self.line = line
        self.atMs = atMs
        self.error = error
        self.detail = detail
        self.text = text
        self.more = more
        self.links = links
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(ActivityKindView.self, forKey: .kind)
        line = try c.decode(String.self, forKey: .line)
        atMs = try c.decodeIfPresent(UInt64.self, forKey: .atMs)
        error = try c.decodeIfPresent(Bool.self, forKey: .error) ?? false
        detail = try c.decodeIfPresent(ToolDetailView.self, forKey: .detail)
        text = try c.decodeIfPresent(String.self, forKey: .text)
        more = try c.decodeIfPresent(Bool.self, forKey: .more) ?? false
        links = try c.decodeIfPresent([LinkView].self, forKey: .links) ?? []
    }
}

/// One turn read whole (`session.turn`): the page's turn with its
/// activity uncut, or no turn when the number is past the end.
public struct TurnBody: Decodable, Sendable, Equatable {
    public let resumeID: String
    public let total: Int
    public let turn: TurnView?

    enum CodingKeys: String, CodingKey {
        case total, turn
        case resumeID = "resume_id"
    }

    public init(resumeID: String, total: Int, turn: TurnView?) {
        self.resumeID = resumeID
        self.total = total
        self.turn = turn
    }
}

/// One human prompt and everything the agent did in response.
public struct TurnView: Decodable, Sendable, Equatable, Identifiable {
    public let n: Int
    public let atMs: UInt64?
    public let endMs: UInt64?
    public let user: String
    public let finalText: String
    public let activity: [ActivityView]
    public let assistantMsgs: Int
    public let tools: Int
    public let thinking: Int
    public let errors: Int
    public let permissionMode: String?
    /// The host cut the turn to its cap.
    public let truncated: Bool
    /// The files named in `finalText`.
    public let links: [LinkView]

    public var id: Int { n }

    enum CodingKeys: String, CodingKey {
        case n, user, activity, tools, thinking, errors, truncated, links
        case atMs = "at_ms"
        case endMs = "end_ms"
        case finalText = "final_text"
        case assistantMsgs = "assistant_msgs"
        case permissionMode = "permission_mode"
    }

    public init(
        n: Int, user: String, finalText: String, activity: [ActivityView] = [], atMs: UInt64? = nil,
        endMs: UInt64? = nil, assistantMsgs: Int = 1, tools: Int = 0, thinking: Int = 0,
        errors: Int = 0, permissionMode: String? = nil, truncated: Bool = false, links: [LinkView] = []
    ) {
        self.n = n
        self.atMs = atMs
        self.endMs = endMs
        self.user = user
        self.finalText = finalText
        self.activity = activity.enumerated().map { i, a in
            var a = a
            a.index = i
            return a
        }
        self.assistantMsgs = assistantMsgs
        self.tools = tools
        self.thinking = thinking
        self.errors = errors
        self.permissionMode = permissionMode
        self.truncated = truncated
        self.links = links
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            n: try c.decode(Int.self, forKey: .n),
            user: try c.decode(String.self, forKey: .user),
            finalText: try c.decode(String.self, forKey: .finalText),
            activity: try c.decodeIfPresent([ActivityView].self, forKey: .activity) ?? [],
            atMs: try c.decodeIfPresent(UInt64.self, forKey: .atMs),
            endMs: try c.decodeIfPresent(UInt64.self, forKey: .endMs),
            assistantMsgs: try c.decodeIfPresent(Int.self, forKey: .assistantMsgs) ?? 0,
            tools: try c.decodeIfPresent(Int.self, forKey: .tools) ?? 0,
            thinking: try c.decodeIfPresent(Int.self, forKey: .thinking) ?? 0,
            errors: try c.decodeIfPresent(Int.self, forKey: .errors) ?? 0,
            permissionMode: try c.decodeIfPresent(String.self, forKey: .permissionMode),
            truncated: try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false,
            links: try c.decodeIfPresent([LinkView].self, forKey: .links) ?? [])
    }
}

/// A page of a conversation: the metadata, the turns from `from`, and
/// `next`, the turn to continue from when more follow (§4.3).
public struct ConversationBody: Decodable, Sendable, Equatable {
    public let resumeID: String
    public let title: String?
    public let model: String?
    public let version: String?
    public let branch: String?
    public let startMs: UInt64?
    public let endMs: UInt64?
    public let usage: UsageView
    public let contextTokens: UInt64?
    public let total: Int
    public let from: Int
    public let next: Int?
    public let turns: [TurnView]

    enum CodingKeys: String, CodingKey {
        case title, model, version, branch, usage, total, from, next, turns
        case resumeID = "resume_id"
        case startMs = "start_ms"
        case endMs = "end_ms"
        case contextTokens = "context_tokens"
    }

    public init(
        resumeID: String, total: Int, from: Int, next: Int?, turns: [TurnView],
        title: String? = nil, model: String? = nil, version: String? = nil, branch: String? = nil,
        startMs: UInt64? = nil, endMs: UInt64? = nil, usage: UsageView = UsageView(),
        contextTokens: UInt64? = nil
    ) {
        self.resumeID = resumeID
        self.title = title
        self.model = model
        self.version = version
        self.branch = branch
        self.startMs = startMs
        self.endMs = endMs
        self.usage = usage
        self.contextTokens = contextTokens
        self.total = total
        self.from = from
        self.next = next
        self.turns = turns
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            resumeID: try c.decode(String.self, forKey: .resumeID),
            total: try c.decode(Int.self, forKey: .total),
            from: try c.decode(Int.self, forKey: .from),
            next: try c.decodeIfPresent(Int.self, forKey: .next),
            turns: try c.decodeIfPresent([TurnView].self, forKey: .turns) ?? [],
            title: try c.decodeIfPresent(String.self, forKey: .title),
            model: try c.decodeIfPresent(String.self, forKey: .model),
            version: try c.decodeIfPresent(String.self, forKey: .version),
            branch: try c.decodeIfPresent(String.self, forKey: .branch),
            startMs: try c.decodeIfPresent(UInt64.self, forKey: .startMs),
            endMs: try c.decodeIfPresent(UInt64.self, forKey: .endMs),
            usage: try c.decodeIfPresent(UsageView.self, forKey: .usage) ?? UsageView(),
            contextTokens: try c.decodeIfPresent(UInt64.self, forKey: .contextTokens))
    }
}

/// The last turn's answer: a projection of the conversation, at its
/// revision. Turn zero with no text is an empty conversation.
public struct LastResponseBody: Decodable, Sendable, Equatable {
    public let resumeID: String
    public let n: Int
    public let atMs: UInt64?
    public let endMs: UInt64?
    public let total: Int
    public let text: String
    public let truncated: Bool
    /// The files named in `text`.
    public let links: [LinkView]

    enum CodingKeys: String, CodingKey {
        case n, total, text, truncated, links
        case resumeID = "resume_id"
        case atMs = "at_ms"
        case endMs = "end_ms"
    }

    public init(
        resumeID: String, n: Int, total: Int, text: String, atMs: UInt64? = nil,
        endMs: UInt64? = nil, truncated: Bool = false, links: [LinkView] = []
    ) {
        self.resumeID = resumeID
        self.n = n
        self.atMs = atMs
        self.endMs = endMs
        self.total = total
        self.text = text
        self.truncated = truncated
        self.links = links
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            resumeID: try c.decode(String.self, forKey: .resumeID),
            n: try c.decode(Int.self, forKey: .n),
            total: try c.decodeIfPresent(Int.self, forKey: .total) ?? 0,
            text: try c.decode(String.self, forKey: .text),
            atMs: try c.decodeIfPresent(UInt64.self, forKey: .atMs),
            endMs: try c.decodeIfPresent(UInt64.self, forKey: .endMs),
            truncated: try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false,
            links: try c.decodeIfPresent([LinkView].self, forKey: .links) ?? [])
    }
}

/// One execution of a command or service, as the record holds it.
public struct RunView: Decodable, Sendable, Equatable, Identifiable {
    public let n: Int
    public let startedMs: UInt64
    public let endedMs: UInt64?
    public let exit: Int?
    /// The names of the files the run produced, never their paths.
    public let artifacts: [String]

    public var id: Int { n }
    /// Still running as far as the record knows.
    public var open: Bool { endedMs == nil }

    enum CodingKeys: String, CodingKey {
        case n, exit, artifacts
        case startedMs = "started_ms"
        case endedMs = "ended_ms"
    }

    public init(n: Int, startedMs: UInt64, endedMs: UInt64? = nil, exit: Int? = nil, artifacts: [String] = []) {
        self.n = n
        self.startedMs = startedMs
        self.endedMs = endedMs
        self.exit = exit
        self.artifacts = artifacts
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            n: try c.decode(Int.self, forKey: .n),
            startedMs: try c.decode(UInt64.self, forKey: .startedMs),
            endedMs: try c.decodeIfPresent(UInt64.self, forKey: .endedMs),
            exit: try c.decodeIfPresent(Int.self, forKey: .exit),
            artifacts: try c.decodeIfPresent([String].self, forKey: .artifacts) ?? [])
    }
}

public struct RunsBody: Decodable, Sendable, Equatable {
    public let runs: [RunView]
    public init(runs: [RunView]) { self.runs = runs }
}

/// The tail of one run's log, redacted.
public struct RunOutputBody: Decodable, Sendable, Equatable {
    public let n: Int
    public let text: String
    public let truncated: Bool

    enum CodingKeys: String, CodingKey { case n, text, truncated }

    public init(n: Int, text: String, truncated: Bool = false) {
        self.n = n
        self.text = text
        self.truncated = truncated
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        n = try c.decodeIfPresent(Int.self, forKey: .n) ?? 0
        text = try c.decode(String.self, forKey: .text)
        truncated = try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false
    }
}
