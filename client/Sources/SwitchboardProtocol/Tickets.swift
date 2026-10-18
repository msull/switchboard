import Foundation

// Dispatch's tickets as the port says them (§4.7), shaped on the host
// in `src/core/tickets.rs`: a listing, one ticket read alone, and the
// answers to a decision or a resume.

/// A decision as a device answers it.
public struct TicketDecisionView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let stage: String
    public let name: String
    public let question: String
    public let options: [String]
    public let recommendation: String?
    public let multiple: Bool
    /// `pending`, `answered`, `acted`, `cancelled` or `cancelling`.
    public let state: String
    public let answer: String?
    public let note: String?
    public let madeMs: UInt64
    public let answeredBy: String?
    public let answeredMs: UInt64?
    /// The options refused without a note.
    public let needsNote: [String]

    public var pending: Bool { state == "pending" }

    enum CodingKeys: String, CodingKey {
        case id, stage, name, question, options, recommendation, multiple, state, answer, note
        case madeMs = "made_ms"
        case answeredBy = "answered_by"
        case answeredMs = "answered_ms"
        case needsNote = "needs_note"
    }

    public init(
        id: String, stage: String = "plan", name: String, question: String, options: [String],
        recommendation: String? = nil, multiple: Bool = false, state: String = "pending",
        answer: String? = nil, note: String? = nil, madeMs: UInt64 = 0, answeredBy: String? = nil,
        answeredMs: UInt64? = nil, needsNote: [String] = []
    ) {
        self.id = id
        self.stage = stage
        self.name = name
        self.question = question
        self.options = options
        self.recommendation = recommendation
        self.multiple = multiple
        self.state = state
        self.answer = answer
        self.note = note
        self.madeMs = madeMs
        self.answeredBy = answeredBy
        self.answeredMs = answeredMs
        self.needsNote = needsNote
    }
}

/// The pull request an attempt is bound to.
public struct TicketPullRequestView: Decodable, Sendable, Equatable {
    public let number: UInt64
    public let url: String
    public let checks: String

    public init(number: UInt64, url: String, checks: String) {
        self.number = number
        self.url = url
        self.checks = checks
    }
}

/// One attempt of a stage, with the session it ran in.
public struct TicketAttemptView: Decodable, Sendable, Equatable, Identifiable {
    public let stage: String
    public let n: UInt32
    public let context: String
    public let kind: String
    public let state: String
    public let reason: String?
    public let session: String?
    public let pr: TicketPullRequestView?
    public let startedMs: UInt64
    public let endedMs: UInt64?

    public var id: String { "\(stage)/\(n)" }

    enum CodingKeys: String, CodingKey {
        case stage, n, context, kind, state, reason, session, pr
        case startedMs = "started_ms"
        case endedMs = "ended_ms"
    }

    public init(
        stage: String, n: UInt32, context: String = "", kind: String = "agent", state: String,
        reason: String? = nil, session: String? = nil, pr: TicketPullRequestView? = nil,
        startedMs: UInt64 = 0, endedMs: UInt64? = nil
    ) {
        self.stage = stage
        self.n = n
        self.context = context
        self.kind = kind
        self.state = state
        self.reason = reason
        self.session = session
        self.pr = pr
        self.startedMs = startedMs
        self.endedMs = endedMs
    }
}

/// A lane by its branch; paths stay on the Mac.
public struct TicketLaneView: Decodable, Sendable, Equatable, Identifiable {
    public let name: String
    public let branch: String
    public let chosen: Bool
    public let head: String?

    public var id: String { name }

    public init(name: String, branch: String, chosen: Bool = true, head: String? = nil) {
        self.name = name
        self.branch = branch
        self.chosen = chosen
        self.head = head
    }
}

/// A ticket: the summary the listing carries, and read alone the body,
/// attempts, lanes and holds too (empty in a listing).
public struct TicketView: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let project: String
    /// `github`, `pull-request`, `task-file` or `manual`.
    public let kind: String
    public let number: UInt64?
    public let title: String
    public let url: String?
    public let labels: [String]
    /// `active`, `parking`, `parked`, `closing` or `closed`.
    public let state: String
    public let reason: String?
    /// The current stage's name; none once past the last.
    public let stage: String?
    public let stages: [String]
    /// Where the ticket stands, in the Mac page's own words.
    public let standing: String
    /// It waits on the owner: a decision, or an agent at a prompt.
    public let waits: Bool
    /// Drawn as needing a look: it waits, or is not simply running.
    public let urgent: Bool
    public let currentSession: String?
    public let rootProject: String?
    public let waitingFor: String?
    public let closable: Bool
    public var decisions: [TicketDecisionView]
    public let createdMs: UInt64
    public let updatedMs: UInt64
    /// Read alone only.
    public let body: String?
    public let attempts: [TicketAttemptView]
    public let lanes: [TicketLaneView]
    public let holds: [String]

    public var pendingDecisions: [TicketDecisionView] { decisions.filter(\.pending) }
    /// `#104`, `PR #3`, or nothing for a ticket without a number.
    public var source: String {
        guard let number else { return "" }
        return kind == "pull-request" ? "PR #\(number)" : "#\(number)"
    }

    enum CodingKeys: String, CodingKey {
        case id, project, kind, number, title, url, labels, state, reason, stage, stages, standing
        case waits, urgent, closable, decisions, body, attempts, lanes, holds
        case currentSession = "current_session"
        case rootProject = "root_project"
        case waitingFor = "waiting_for"
        case createdMs = "created_ms"
        case updatedMs = "updated_ms"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        project = try c.decode(String.self, forKey: .project)
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "manual"
        number = try c.decodeIfPresent(UInt64.self, forKey: .number)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        url = try c.decodeIfPresent(String.self, forKey: .url)
        labels = try c.decodeIfPresent([String].self, forKey: .labels) ?? []
        state = try c.decodeIfPresent(String.self, forKey: .state) ?? "active"
        reason = try c.decodeIfPresent(String.self, forKey: .reason)
        stage = try c.decodeIfPresent(String.self, forKey: .stage)
        stages = try c.decodeIfPresent([String].self, forKey: .stages) ?? []
        standing = try c.decodeIfPresent(String.self, forKey: .standing) ?? ""
        waits = try c.decodeIfPresent(Bool.self, forKey: .waits) ?? false
        urgent = try c.decodeIfPresent(Bool.self, forKey: .urgent) ?? false
        currentSession = try c.decodeIfPresent(String.self, forKey: .currentSession)
        rootProject = try c.decodeIfPresent(String.self, forKey: .rootProject)
        waitingFor = try c.decodeIfPresent(String.self, forKey: .waitingFor)
        closable = try c.decodeIfPresent(Bool.self, forKey: .closable) ?? false
        decisions = try c.decodeIfPresent([TicketDecisionView].self, forKey: .decisions) ?? []
        createdMs = try c.decodeIfPresent(UInt64.self, forKey: .createdMs) ?? 0
        updatedMs = try c.decodeIfPresent(UInt64.self, forKey: .updatedMs) ?? 0
        body = try c.decodeIfPresent(String.self, forKey: .body)
        attempts = try c.decodeIfPresent([TicketAttemptView].self, forKey: .attempts) ?? []
        lanes = try c.decodeIfPresent([TicketLaneView].self, forKey: .lanes) ?? []
        holds = try c.decodeIfPresent([String].self, forKey: .holds) ?? []
    }

    /// A view built by hand, for tests and previews.
    public init(
        id: String, project: String, kind: String = "github", number: UInt64? = nil, title: String,
        url: String? = nil, labels: [String] = [], state: String = "active", reason: String? = nil,
        stage: String? = nil, stages: [String] = [], standing: String = "", waits: Bool = false,
        urgent: Bool = false, currentSession: String? = nil, rootProject: String? = nil,
        waitingFor: String? = nil, closable: Bool = false, decisions: [TicketDecisionView] = [],
        createdMs: UInt64 = 0, updatedMs: UInt64 = 0, body: String? = nil,
        attempts: [TicketAttemptView] = [], lanes: [TicketLaneView] = [], holds: [String] = []
    ) {
        self.id = id
        self.project = project
        self.kind = kind
        self.number = number
        self.title = title
        self.url = url
        self.labels = labels
        self.state = state
        self.reason = reason
        self.stage = stage
        self.stages = stages
        self.standing = standing
        self.waits = waits
        self.urgent = urgent
        self.currentSession = currentSession
        self.rootProject = rootProject
        self.waitingFor = waitingFor
        self.closable = closable
        self.decisions = decisions
        self.createdMs = createdMs
        self.updatedMs = updatedMs
        self.body = body
        self.attempts = attempts
        self.lanes = lanes
        self.holds = holds
    }
}

/// A project with a pipeline, and its supervisor's session.
public struct TicketProjectView: Decodable, Sendable, Equatable, Identifiable {
    public let name: String
    public let slots: UInt32
    public let running: UInt32
    public let pending: UInt32
    public let waitingOnMe: UInt32
    public let queue: [String]
    public let supervisorSession: String?

    public var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, slots, running, pending, queue
        case waitingOnMe = "waiting_on_me"
        case supervisorSession = "supervisor_session"
    }

    public init(
        name: String, slots: UInt32 = 1, running: UInt32 = 0, pending: UInt32 = 0,
        waitingOnMe: UInt32 = 0, queue: [String] = [], supervisorSession: String? = nil
    ) {
        self.name = name
        self.slots = slots
        self.running = running
        self.pending = pending
        self.waitingOnMe = waitingOnMe
        self.queue = queue
        self.supervisorSession = supervisorSession
    }
}

/// `GET dispatch/tickets`.
public struct TicketsBody: Decodable, Sendable, Equatable {
    /// A runner answered the host's last poll; without one the tickets
    /// are the last seen.
    public let connected: Bool
    public let projects: [TicketProjectView]
    public var tickets: [TicketView]

    public init(connected: Bool, projects: [TicketProjectView] = [], tickets: [TicketView] = []) {
        self.connected = connected
        self.projects = projects
        self.tickets = tickets
    }
}

/// `POST dispatch/tickets/{id}/decide`.
public struct DecideRequest: Encodable, Sendable, Equatable {
    /// The decision's id.
    public let decision: String
    /// The option chosen; for a `multiple` decision the options joined
    /// by commas.
    public let answer: String
    public let note: String?

    public init(decision: String, answer: String, note: String? = nil) {
        self.decision = decision
        self.answer = answer
        self.note = note
    }
}

/// The decision as recorded, answering a decide.
public struct DecidedBody: Decodable, Sendable, Equatable {
    public let decision: TicketDecisionView

    public init(decision: TicketDecisionView) {
        self.decision = decision
    }
}

/// The ticket as it stands, answering a resume.
public struct TicketResumedBody: Decodable, Sendable, Equatable {
    public let ticket: TicketView

    public init(ticket: TicketView) {
        self.ticket = ticket
    }
}
