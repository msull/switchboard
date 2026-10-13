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
                     "capabilities":["read"],"spki_sha256":"\(spki)",
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
        try answer("session") {
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
    func lastResponse(_ id: String) async throws -> Envelope<LastResponseBody> {
        try await network.route(address, pin).lastResponse(id)
    }
    func runs(_ id: String) async throws -> Envelope<RunsBody> {
        try await network.route(address, pin).runs(id)
    }
    func runOutput(_ id: String, run: Int, lines: Int?) async throws -> Envelope<RunOutputBody> {
        try await network.route(address, pin).runOutput(id, run: run, lines: lines)
    }
}
