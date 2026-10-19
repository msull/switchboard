import Foundation
import SwitchboardClientCore
import SwitchboardProtocol
import SwitchboardTransport

/// A host in memory: whatever the test puts in it, at the revision and
/// epoch it says, with the failures it is told to produce.
final class FakeHost: HostAPI, @unchecked Sendable {
    let lock = NSLock()
    var serverID: ServerID
    var name: String
    var spki: String
    var epoch = "e1"
    var revision: UInt64 = 1
    var projects: [ProjectView] = []
    var spaces: [SpaceView] = []
    var sets: [String: [SetView]] = [:]
    var sessions: [String: [SessionView]] = [:]
    /// Thrown by every call while set.
    var failure: RemoteError?
    /// How long a single-session read takes, whatever happens to the
    /// task meanwhile: a request in flight when its pump is replaced.
    var sessionDelay: TimeInterval = 0
    var pairCode = "ABCD2345"
    var pairs = 0
    /// Every call by name; a frame a device sent is `watch:<id>` or
    /// `unwatch:<id>`.
    var calls: [String] = []
    /// What `screen` answers, by session id; a session not here is
    /// refused as not running.
    var screens: [String: ScreenBody] = [:]
    var lastResponses: [String: LastResponseBody] = [:]
    /// The turns as a read of one whole gives them, when they differ
    /// from the page's copies in `conversations`.
    var turns: [String: [TurnView]] = [:]
    /// Whole conversations; a page is cut from one by `from` and `limit`.
    var conversations: [String: ConversationBody] = [:]
    var runs: [String: [RunView]] = [:]
    var runOutputs: [RunKey: RunOutputBody] = [:]
    /// Commands get no answer this many times (the request is still
    /// recorded, as one that reached the host would be).
    var commandSilences = 0
    /// Thrown by every command while set, after the call is recorded.
    var commandRefusal: RemoteError?
    /// What `opStatus` answers, by op; an op not here is unknown.
    var opStatuses: [String: OpStatusView] = [:]
    /// The drafts, by session: finals append, replace sets, undo takes
    /// the last change back, send clears and is a `send:` call.
    var drafts: [String: DraftView] = [:]
    var draftHistory: [String: [String]] = [:]
    /// `(recognizer session, events)` as received.
    var dictations: [(UInt64, [DictationEvent])] = []
    var capabilities = ["read", "control", "draft"]
    /// Dispatch's tickets; none is a host without the capability.
    var tickets: TicketsBody?
    /// Every decide as received, in order.
    var decides: [(String, DecideRequest)] = []
    /// Decides get no answer this many times (the answer is still
    /// recorded on the ticket, as one that reached the host would be).
    var decideSilences = 0
    /// A ticket's files by key, and its events.
    var ticketFiles: [String: TicketFileBody] = [:]
    var ticketEvents: [String: TicketEventsBody] = [:]
    /// The pinned files, by `project/path`; a missing one is `not_pinned`.
    var files: [String: FileBody] = [:]
    /// The op of every command, in order.
    var ops: [String] = []
    /// Runs after a `sessions` reply is built and before it is returned:
    /// a mutation between the snapshot and its delivery.
    var afterSessionsBuilt: (() -> Void)?
    private var subscribers: [Int: AsyncThrowingStream<EventFrame, Error>.Continuation] = [:]
    private var nextSubscriber = 0

    init(serverID: ServerID, name: String, spki: String) {
        self.serverID = serverID
        self.name = name
        self.spki = spki
    }

    func set(_ change: (FakeHost) -> Void) {
        lock.lock()
        change(self)
        lock.unlock()
    }

    private func answer<T: Sendable>(_ call: String, _ body: () -> T) throws -> Envelope<T> {
        lock.lock()
        defer { lock.unlock() }
        calls.append(call)
        if let failure { throw failure }
        return Envelope(epoch: epoch, revision: revision, body: body())
    }

    /// The events socket: `hello` at the current revision, then
    /// whatever the test emits.
    func events() async throws -> EventSubscription {
        lock.lock()
        calls.append("events")
        if let failure {
            lock.unlock()
            throw failure
        }
        let hello = EventFrame(kind: .hello, serverID: serverID, epoch: epoch, revision: revision)
        var continuation: AsyncThrowingStream<EventFrame, Error>.Continuation!
        let stream = AsyncThrowingStream<EventFrame, Error> { continuation = $0 }
        continuation.yield(hello)
        let id = nextSubscriber
        nextSubscriber += 1
        subscribers[id] = continuation
        lock.unlock()
        return EventSubscription(
            frames: stream,
            send: { [self] text in
                // A `watch` is answered `watching` to this subscriber only.
                let frame = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any]
                let type = frame?["type"] as? String ?? "?"
                let resource = frame?["resource"] as? [String: String]
                let sessionID = resource?["id"] ?? "?"
                lock.lock()
                calls.append("\(type):\(sessionID)")
                let target = subscribers[id]
                let (epoch, revision, serverID) = (self.epoch, self.revision, self.serverID)
                lock.unlock()
                if type == "watch", let kind = resource?["kind"] {
                    target?.yield(
                        EventFrame(
                            kind: .watching, serverID: serverID, epoch: epoch, revision: revision,
                            resource: ResourceRef(kind: kind, id: sessionID)))
                }
            },
            close: { [self] in
                lock.lock()
                let held = subscribers.removeValue(forKey: id)
                lock.unlock()
                held?.finish()
            })
    }

    /// An event to every subscriber, at the host's current revision and
    /// epoch unless the frame says otherwise.
    func emit(_ kind: EventFrame.Kind, _ resource: ResourceRef? = nil, revision: UInt64? = nil) {
        lock.lock()
        let frame = EventFrame(
            kind: kind, serverID: serverID, epoch: epoch, revision: revision ?? self.revision,
            resource: resource)
        let targets = Array(subscribers.values)
        lock.unlock()
        for c in targets { c.yield(frame) }
    }

    /// Close every subscriber's socket, as a host restart or a
    /// revocation would.
    func closeSockets() {
        lock.lock()
        let targets = Array(subscribers.values)
        subscribers = [:]
        lock.unlock()
        for c in targets { c.finish() }
    }

    var subscriberCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return subscribers.count
    }

    func info() async throws -> Envelope<ServerInfo> {
        try answer("info") {
            try! Wire.decode(
                ServerInfo.self,
                from: Data(
                    """
                    {"server_id":"\(serverID)","server_name":"\(name)","protocol":1,
                     "capabilities":[\(capabilities.map { "\"\($0)\"" }.joined(separator: ","))],"spki_sha256":"\(spki)",
                     "limits":{"body_bytes":1,"string_bytes":1,"notes_bytes":1,"screen_lines":1},
                     "pairing_open":false}
                    """.utf8))
        }
    }

    private func check(_ request: PairRequest) -> (wrongCode: Bool, wrongKey: Bool) {
        lock.lock()
        defer { lock.unlock() }
        return (request.code != pairCode, request.spkiSha256 != spki)
    }

    func pair(_ request: PairRequest) async throws -> Envelope<PairReply> {
        let (wrongCode, wrongKey) = check(request)
        if wrongCode {
            throw RemoteError.refused(status: 403, code: "wrong_code", message: "wrong code")
        }
        if wrongKey {
            throw RemoteError.refused(status: 403, code: "wrong_key", message: "another host's key")
        }
        return try answer("pair") {
            pairs += 1
            return try! Wire.decode(
                PairReply.self,
                from: Data(
                    """
                    {"token":"tok-\(pairs)","scope":"read","server_id":"\(serverID)","device_id":"d1"}
                    """.utf8))
        }
    }

    func projects() async throws -> Envelope<ProjectsBody> {
        try answer("projects") { ProjectsBody(projects: projects) }
    }

    func spaces() async throws -> Envelope<SpacesBody> {
        try answer("spaces") { SpacesBody(spaces: spaces) }
    }

    func sets(space: String) async throws -> Envelope<SetsBody> {
        try answer("sets") { SetsBody(sets: sets[space] ?? []) }
    }

    func sessions(project: String) async throws -> Envelope<SessionsBody> {
        let reply = try answer("sessions") { SessionsBody(sessions: sessions[project] ?? []) }
        lock.lock()
        let hook = afterSessionsBuilt
        afterSessionsBuilt = nil
        lock.unlock()
        hook?()
        return reply
    }

    func session(_ id: String) async throws -> Envelope<SessionBody> {
        lock.lock()
        let delay = sessionDelay
        lock.unlock()
        if delay > 0 {
            // Not `Task.sleep`: cancellation must not cut it short.
            await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
                DispatchQueue.global().asyncAfter(deadline: .now() + delay) { c.resume() }
            }
        }
        return try answer("session") {
            SessionBody(session: sessions.values.flatMap { $0 }.first { $0.id == id }!)
        }
    }

    func waiting() async throws -> Envelope<SessionsBody> {
        try answer("waiting") { SessionsBody(sessions: sessions.values.flatMap { $0 }.filter(\.waiting)) }
    }

    func screen(_ id: String, lines: Int?) async throws -> Envelope<ScreenBody> {
        lock.lock()
        let held = screens[id]
        lock.unlock()
        guard let held else {
            throw RemoteError.refused(status: 400, code: "failed", message: "not running")
        }
        return try answer("screen") { held }
    }

    func conversation(_ id: String, from: String?, before: Int?, limit: Int?) async throws
        -> Envelope<ConversationBody>
    {
        lock.lock()
        let whole = conversations[id]
        lock.unlock()
        guard let whole else {
            throw RemoteError.refused(status: 400, code: "no_conversation", message: "no conversation")
        }
        let limit = limit ?? 20
        // Anchored at the end: the last `limit` turns before the anchor.
        let end = before.map { min($0 - 1, whole.turns.count) }
        let first =
            if let end { max(1, end - limit + 1) }
            else if from == "last" || from == nil { max(1, whole.turns.count - limit + 1) }
            else { Int(from!) ?? 1 }
        let page = whole.turns.filter { $0.n >= first && (end == nil || $0.n <= end!) }.prefix(limit)
        let next = page.last.flatMap { last in whole.turns.contains { $0.n > last.n } ? last.n + 1 : nil }
        return try answer("conversation") {
            ConversationBody(
                resumeID: whole.resumeID, total: whole.turns.count, from: first, next: next,
                turns: Array(page), title: whole.title)
        }
    }

    func turn(_ id: String, n: Int) async throws -> Envelope<TurnBody> {
        lock.lock()
        let whole = conversations[id]
        let full = turns[id]
        lock.unlock()
        guard let whole else {
            throw RemoteError.refused(status: 400, code: "no_conversation", message: "no conversation")
        }
        let turn = (full ?? whole.turns).first { $0.n == n }
        return try answer("turn:\(n)") { TurnBody(resumeID: whole.resumeID, total: whole.turns.count, turn: turn) }
    }

    func command(_ id: String, _ verb: CommandVerb, op: String, text: String?) async throws
        -> Envelope<CommandBody>
    {
        lock.lock()
        calls.append(text.map { "\(verb.rawValue):\(id):\($0)" } ?? "\(verb.rawValue):\(id)")
        ops.append(op)
        let silent = commandSilences > 0
        if silent { commandSilences -= 1 }
        let refusal = commandRefusal
        lock.unlock()
        if silent { throw RemoteError.transport("timed out") }
        if let refusal { throw refusal }
        return try answer("command") { CommandBody() }
    }

    func opStatus(_ op: String) async throws -> Envelope<OpStatusBody> {
        lock.lock()
        let status = opStatuses[op] ?? .unknown
        lock.unlock()
        return try answer("op:\(op)") { OpStatusBody(status: status) }
    }

    func tickets() async throws -> Envelope<TicketsBody> {
        lock.lock()
        let held = tickets
        lock.unlock()
        guard let held else {
            throw RemoteError.refused(status: 404, code: "not_found", message: "not available on this port")
        }
        return try answer("tickets") { held }
    }

    func ticket(_ id: String) async throws -> Envelope<TicketView> {
        lock.lock()
        let held = tickets?.tickets.first { $0.id == id }
        lock.unlock()
        guard let held else {
            throw RemoteError.refused(status: 404, code: "not_found", message: "no such ticket")
        }
        return try answer("ticket") { held }
    }

    /// The decision answered on the ticket as the host records it,
    /// then handed back; a silence keeps the record and drops the reply.
    func decide(_ ticket: String, _ request: DecideRequest) async throws -> Envelope<DecidedBody> {
        lock.lock()
        decides.append((ticket, request))
        guard var held = tickets, let i = held.tickets.firstIndex(where: { $0.id == ticket }),
            let j = held.tickets[i].decisions.firstIndex(where: { $0.id == request.decision })
        else {
            lock.unlock()
            throw RemoteError.refused(status: 404, code: "not_found", message: "no such decision on the ticket")
        }
        let was = held.tickets[i].decisions[j]
        if !was.pending {
            lock.unlock()
            throw RemoteError.refused(status: 409, code: "answered", message: "the decision is \(was.state)")
        }
        let answered = TicketDecisionView(
            id: was.id, stage: was.stage, name: was.name, question: was.question, options: was.options,
            recommendation: was.recommendation, multiple: was.multiple, state: "answered",
            answer: request.answer, note: request.note, madeMs: was.madeMs, answeredBy: "user",
            answeredMs: 99, needsNote: was.needsNote)
        held.tickets[i].decisions[j] = answered
        tickets = held
        if decideSilences > 0 {
            decideSilences -= 1
            lock.unlock()
            throw RemoteError.transport("cut short")
        }
        lock.unlock()
        return try answer("decide") { DecidedBody(decision: answered) }
    }

    func resumeTicket(_ ticket: String) async throws -> Envelope<TicketResumedBody> {
        lock.lock()
        guard var held = tickets, let i = held.tickets.firstIndex(where: { $0.id == ticket }) else {
            lock.unlock()
            throw RemoteError.refused(status: 404, code: "not_found", message: "no such ticket")
        }
        let was = held.tickets[i]
        let resumed = TicketView(
            id: was.id, project: was.project, kind: was.kind, number: was.number, title: was.title,
            state: "active", stage: was.stage, stages: was.stages, standing: "queued",
            decisions: was.decisions, createdMs: was.createdMs, updatedMs: was.updatedMs + 1)
        held.tickets[i] = resumed
        tickets = held
        lock.unlock()
        return try answer("resume_ticket") { TicketResumedBody(ticket: resumed) }
    }

    func closeTicket(_ ticket: String) async throws -> Envelope<TicketResumedBody> {
        lock.lock()
        guard var held = tickets, let i = held.tickets.firstIndex(where: { $0.id == ticket }) else {
            lock.unlock()
            throw RemoteError.refused(status: 404, code: "not_found", message: "no such ticket")
        }
        let was = held.tickets[i]
        guard was.closable else {
            lock.unlock()
            throw RemoteError.refused(status: 409, code: "not_closable", message: "the ticket is \(was.state)")
        }
        let closing = TicketView(
            id: was.id, project: was.project, kind: was.kind, number: was.number, title: was.title,
            state: "closing", reason: "closed by hand", stage: was.stage, stages: was.stages,
            standing: "closing", decisions: was.decisions, createdMs: was.createdMs, updatedMs: was.updatedMs + 1)
        held.tickets[i] = closing
        tickets = held
        lock.unlock()
        return try answer("close_ticket") { TicketResumedBody(ticket: closing) }
    }

    func ticketFile(_ ticket: String, key: String) async throws -> Envelope<TicketFileBody> {
        lock.lock()
        let held = ticketFiles[key]
        lock.unlock()
        guard let held else {
            throw RemoteError.refused(status: 404, code: "not_found", message: "no such file")
        }
        return try answer("ticket_file:\(key)") { held }
    }

    func ticketEvents(_ ticket: String) async throws -> Envelope<TicketEventsBody> {
        lock.lock()
        let held = ticketEvents[ticket] ?? TicketEventsBody(events: [])
        lock.unlock()
        return try answer("ticket_events:\(ticket)") { held }
    }

    func file(project: String, path: String) async throws -> Envelope<FileBody> {
        lock.lock()
        let file = files["\(project)/\(path)"]
        lock.unlock()
        guard let file else {
            throw RemoteError.refused(status: 404, code: "not_pinned", message: "no working set pins this file")
        }
        return try answer("file:\(project)/\(path)") { file }
    }

    func draft(_ id: String) async throws -> Envelope<DraftBody> {
        lock.lock()
        let draft = drafts[id]
        lock.unlock()
        guard let draft else { throw RemoteError.refused(status: 400, code: "no_draft", message: "no draft") }
        return try answer("draft:\(id)") { DraftBody(draft: draft) }
    }

    private func change(_ id: String, _ edit: (inout DraftView) -> Void) {
        var draft = drafts[id] ?? DraftView()
        draftHistory[id, default: []].append(draft.text)
        edit(&draft)
        draft.canUndo = !(draftHistory[id] ?? []).isEmpty
        drafts[id] = draft
    }

    func draftAction(_ id: String, _ body: DraftActionBody) async throws -> Envelope<DraftBody> {
        lock.lock()
        calls.append("draft:\(id):\(body.action)")
        let silent = commandSilences > 0
        if silent { commandSilences -= 1 }
        let refusal = commandRefusal
        if body.action == "send" { ops.append(body.op ?? "") }
        if !silent, refusal == nil {
            switch body.action {
            case "replace": change(id) { $0.text = body.text ?? "" }
            case "undo":
                if let last = draftHistory[id]?.popLast() {
                    drafts[id]?.text = last
                    drafts[id]?.canUndo = !(draftHistory[id] ?? []).isEmpty
                }
            case "clear": change(id) { $0.text = "" }
            case "send":
                let text = drafts[id]?.text ?? ""
                calls.append("send:\(id):\(text)")
                change(id) { $0.text = "" }
            default: break
            }
        }
        let draft = drafts[id] ?? DraftView()
        lock.unlock()
        if silent { throw RemoteError.transport("timed out") }
        if let refusal { throw refusal }
        return try answer("draft-reply") { DraftBody(draft: draft) }
    }

    /// The committed text before the live span, which a partial or a
    /// final replaces from.
    private static func beforeLive(_ draft: DraftView) -> String {
        guard let live = draft.live else { return draft.text }
        return String(decoding: draft.text.utf8.prefix(live[0]), as: UTF8.self)
            .trimmingCharacters(in: .whitespaces)
    }

    func dictate(_ id: String, _ body: DictationBody) async throws -> Envelope<DraftBody> {
        lock.lock()
        dictations.append((body.session, body.events))
        for event in body.events {
            switch event.kind {
            case .started: change(id) { $0.status = "listening" }
            case .stopped: change(id) { $0.status = "idle"; $0.live = nil }
            case .partial:
                change(id) { d in
                    let base = FakeHost.beforeLive(d)
                    let joined = base.isEmpty ? event.text : base + " " + event.text
                    d.live = [base.isEmpty ? 0 : base.utf8.count + 1, joined.utf8.count]
                    d.text = joined
                }
            case .final:
                change(id) { d in
                    let base = FakeHost.beforeLive(d)
                    d.text = base.isEmpty ? event.text : base + " " + event.text
                    d.live = nil
                }
            case .voiceStarted, .voiceEnded: break
            }
        }
        let draft = drafts[id] ?? DraftView()
        lock.unlock()
        return try answer("dictate:\(id)") { DraftBody(draft: draft) }
    }

    func lastResponse(_ id: String) async throws -> Envelope<LastResponseBody> {
        lock.lock()
        let held = lastResponses[id]
        lock.unlock()
        guard let held else {
            throw RemoteError.refused(status: 400, code: "no_conversation", message: "no conversation")
        }
        return try answer("last_response") { held }
    }

    func runs(_ id: String) async throws -> Envelope<RunsBody> {
        try answer("runs") { RunsBody(runs: runs[id] ?? []) }
    }

    func runOutput(_ id: String, run: Int, lines: Int?) async throws -> Envelope<RunOutputBody> {
        lock.lock()
        let held = runOutputs[RunKey(session: id, n: run)]
        lock.unlock()
        guard let held else {
            throw RemoteError.refused(status: 404, code: "no_run", message: "no such run")
        }
        return try answer("run_output") { held }
    }
}


/// Every host a test reaches, by address, and what each connection was
/// opened with. Each call is routed afresh, as a new TCP connection
/// would be: a host that moved is not found at its old address, and a
/// pin that is not the host's current key fails as the handshake does,
/// with nothing sent.
final class FakeNetwork: @unchecked Sendable {
    private let lock = NSLock()
    var hosts: [Address: FakeHost] = [:]
    var opened: [(Address, PinPolicy, String?)] = []

    var factory: HostAPIFactory {
        { [self] address, pin, token in
            lock.lock()
            opened.append((address, pin, token))
            lock.unlock()
            return Route(network: self, address: address, pin: pin)
        }
    }

    func route(_ address: Address, _ pin: PinPolicy) throws -> FakeHost {
        lock.lock()
        defer { lock.unlock() }
        guard let host = hosts[address] else {
            throw RemoteError.transport("no host at \(address)")
        }
        if case .expect(let expected) = pin, expected != host.spki {
            throw RemoteError.pinMismatch(expected: expected, seen: host.spki)
        }
        return host
    }
}

/// One connection's view of the network: the handshake on every call.
struct Route: HostAPI {
    let network: FakeNetwork
    let address: Address
    let pin: PinPolicy

    func info() async throws -> Envelope<ServerInfo> { try await network.route(address, pin).info() }
    func events() async throws -> EventSubscription { try await network.route(address, pin).events() }
    func pair(_ request: PairRequest) async throws -> Envelope<PairReply> {
        try await network.route(address, pin).pair(request)
    }
    func projects() async throws -> Envelope<ProjectsBody> { try await network.route(address, pin).projects() }
    func spaces() async throws -> Envelope<SpacesBody> { try await network.route(address, pin).spaces() }
    func sets(space: String) async throws -> Envelope<SetsBody> {
        try await network.route(address, pin).sets(space: space)
    }
    func sessions(project: String) async throws -> Envelope<SessionsBody> {
        try await network.route(address, pin).sessions(project: project)
    }
    func session(_ id: String) async throws -> Envelope<SessionBody> {
        try await network.route(address, pin).session(id)
    }
    func waiting() async throws -> Envelope<SessionsBody> { try await network.route(address, pin).waiting() }
    func screen(_ id: String, lines: Int?) async throws -> Envelope<ScreenBody> {
        try await network.route(address, pin).screen(id, lines: lines)
    }
    func conversation(_ id: String, from: String?, before: Int?, limit: Int?) async throws
        -> Envelope<ConversationBody>
    {
        try await network.route(address, pin).conversation(id, from: from, before: before, limit: limit)
    }
    func turn(_ id: String, n: Int) async throws -> Envelope<TurnBody> {
        try await network.route(address, pin).turn(id, n: n)
    }
    func command(_ id: String, _ verb: CommandVerb, op: String, text: String?) async throws
        -> Envelope<CommandBody>
    {
        try await network.route(address, pin).command(id, verb, op: op, text: text)
    }
    func opStatus(_ op: String) async throws -> Envelope<OpStatusBody> {
        try await network.route(address, pin).opStatus(op)
    }
    func draft(_ id: String) async throws -> Envelope<DraftBody> {
        try await network.route(address, pin).draft(id)
    }
    func draftAction(_ id: String, _ body: DraftActionBody) async throws -> Envelope<DraftBody> {
        try await network.route(address, pin).draftAction(id, body)
    }
    func dictate(_ id: String, _ body: DictationBody) async throws -> Envelope<DraftBody> {
        try await network.route(address, pin).dictate(id, body)
    }
    func file(project: String, path: String) async throws -> Envelope<FileBody> {
        try await network.route(address, pin).file(project: project, path: path)
    }
    func lastResponse(_ id: String) async throws -> Envelope<LastResponseBody> {
        try await network.route(address, pin).lastResponse(id)
    }
    func runs(_ id: String) async throws -> Envelope<RunsBody> {
        try await network.route(address, pin).runs(id)
    }
    func tickets() async throws -> Envelope<TicketsBody> { try await network.route(address, pin).tickets() }
    func ticket(_ id: String) async throws -> Envelope<TicketView> { try await network.route(address, pin).ticket(id) }
    func decide(_ ticket: String, _ request: DecideRequest) async throws -> Envelope<DecidedBody> {
        try await network.route(address, pin).decide(ticket, request)
    }
    func resumeTicket(_ ticket: String) async throws -> Envelope<TicketResumedBody> {
        try await network.route(address, pin).resumeTicket(ticket)
    }
    func closeTicket(_ ticket: String) async throws -> Envelope<TicketResumedBody> {
        try await network.route(address, pin).closeTicket(ticket)
    }
    func ticketFile(_ ticket: String, key: String) async throws -> Envelope<TicketFileBody> {
        try await network.route(address, pin).ticketFile(ticket, key: key)
    }
    func ticketEvents(_ ticket: String) async throws -> Envelope<TicketEventsBody> {
        try await network.route(address, pin).ticketEvents(ticket)
    }
    func runOutput(_ id: String, run: Int, lines: Int?) async throws -> Envelope<RunOutputBody> {
        try await network.route(address, pin).runOutput(id, run: run, lines: lines)
    }
}
