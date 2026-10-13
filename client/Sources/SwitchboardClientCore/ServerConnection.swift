import Foundation
import SwitchboardProtocol
import SwitchboardTransport

/// Where one host stands (§6.2).
/// What became of a command (§4.5).
/// What became of a change to a draft (§4.5).
public enum DraftOutcome: Sendable, Equatable {
    /// The host applied it; this is the draft now.
    case applied(DraftView)
    /// The host refused, with its reason.
    case refused(String)
    /// A send the host started whose outcome is unknown.
    case interrupted
    /// No answer.
    case unanswered(String)

    public var note: String? {
        switch self {
        case .applied: return nil
        case .refused(let why): return why
        case .interrupted: return "The host started the send; check the session."
        case .unanswered(let why): return why
        }
    }
}

public enum CommandOutcome: Equatable, Sendable {
    /// The host ran it, or had already.
    case accepted
    /// The host refused it and nothing ran; the reason is the host's.
    case refused(String)
    /// The host started it and the outcome is not known. The session's
    /// state says what happened; it is never resubmitted.
    case interrupted
    /// No answer either way, and the host never saw it.
    case unanswered(String)

    /// A line for the window.
    public var note: String? {
        switch self {
        case .accepted: return nil
        case .refused(let why): return why
        case .interrupted: return "The host started this; check the session."
        case .unanswered(let why): return "No answer from the host: \(why)"
        }
    }
}

public enum ConnectionState: Equatable, Sendable {
    case discovered
    case unpaired
    case pairing
    case connecting
    case syncing
    case connected
    case reconnecting(attempt: Int)
    case offline(reason: String)

    public var isConnected: Bool { self == .connected }
}

/// One host's read model as last fetched: whole collections at the
/// revision they were observed.
public struct HostSnapshot: Sendable, Equatable {
    public var epoch: String
    public var revision: UInt64
    public var projects: [ProjectView] = []
    public var spaces: [SpaceView] = []
    public var sets: [SetView] = []
    public var sessions: [SessionView] = []
    public var fetchedAt: Date

    public init(epoch: String, revision: UInt64, fetchedAt: Date = Date()) {
        self.epoch = epoch
        self.revision = revision
        self.fetchedAt = fetchedAt
    }
}

/// Makes the network side for an address, so tests pass a fake.
public typealias HostAPIFactory = @Sendable (Address, PinPolicy, String?) -> HostAPI

public let liveHostAPI: HostAPIFactory = { address, pin, token in
    HostClient(address: address, pin: pin, token: token)
}

/// How long `hello` may take after the socket opens.
let helloTimeout: TimeInterval = 10

/// One known host: its state machine, its credentials, its snapshot,
/// its event subscription, and the revision rule that drops an older
/// reply arriving late. The manager owns one per known server; a stall
/// here never touches another host.
///
/// Subscribe first, then snapshot (§4.4): the socket is opened and
/// `hello` read before any collection is fetched; events that arrive
/// meanwhile are buffered and applied after the snapshots, each against
/// the revision its resource was fetched at, so nothing can fall
/// between the subscription and the snapshot.
public actor ServerConnection {
    public private(set) var server: KnownServer
    public private(set) var state: ConnectionState
    public private(set) var snapshot: HostSnapshot?
    public private(set) var info: ServerInfo?
    /// The host's key changed under its id: nothing is trusted until a
    /// new pairing, and the cached snapshot is dropped.
    public private(set) var identityChanged = false
    public private(set) var lastError: String?
    /// How many times the host told this client to resync.
    public private(set) var resyncs = 0

    private let credentials: CredentialStore
    private let factory: HostAPIFactory
    private let deviceName: String
    private var api: HostAPI?
    private var held: [ResourceKey: UInt64] = [:]
    private var failures = 0
    private var nextTry = Date.distantPast
    private var subscription: EventSubscription?
    private var pump: Task<Void, Never>?
    private var buffer: [EventFrame] = []
    private var buffering = false
    /// How many views hold each watched content key; `watch` goes out
    /// at the first and `unwatch` at the last (§4.4), and every watch
    /// is resent after `hello` on a reconnect.
    private var watches: [ResourceKey: Int] = [:]
    /// The text last read per session, watched or not, at the revision
    /// its key holds.
    public private(set) var screens: [String: ScreenBody] = [:]
    /// The last response per session, read from the conversation key.
    public private(set) var lastResponses: [String: LastResponseBody] = [:]
    /// The conversation per session, the pages read merged by turn.
    public private(set) var conversations: [String: ConversationState] = [:]
    /// The runs per command or service session.
    public private(set) var runs: [String: [RunView]] = [:]
    /// The output tail per run read.
    public private(set) var runOutputs: [RunKey: RunOutputBody] = [:]
    /// The drafts held, by session: the host's editor as last read.
    public private(set) var drafts: [String: DraftView] = [:]
    /// Told whenever the snapshot or the state moved, so the owner can
    /// rebuild its view without polling.
    private var onChange: (@Sendable () -> Void)?

    public init(
        server: KnownServer, credentials: CredentialStore, deviceName: String,
        factory: @escaping HostAPIFactory = liveHostAPI
    ) {
        self.server = server
        self.credentials = credentials
        self.deviceName = deviceName
        self.factory = factory
        self.state = server.needsPairing ? .unpaired : .connecting
    }

    public func setOnChange(_ handler: @escaping @Sendable () -> Void) {
        onChange = handler
    }

    private func changed() {
        onChange?()
    }

    /// Where it will be reached next.
    public var address: Address? { server.addresses.first }

    /// Whether the backoff allows a try now.
    public var dueForTry: Bool { Date() >= nextTry }

    public func setAddresses(_ addresses: [Address]) {
        guard !addresses.isEmpty, addresses != server.addresses else { return }
        server.addresses = addresses
        api = nil
        nextTry = .distantPast
    }

    public func setEnabled(_ enabled: Bool) {
        server.enabled = enabled
        if !enabled {
            disconnect(to: .offline(reason: "disabled"))
        }
    }

    /// Pair with the code from the host's dialog. The key hash from the
    /// QR is pinned before the code is sent: a substituted certificate
    /// fails the handshake and the code never leaves the device.
    public func pair(code: String, spkiSha256: String, at address: Address? = nil) async throws {
        guard let address = address ?? self.address else {
            throw RemoteError.transport("no address for \(server.name)")
        }
        state = .pairing
        let client = factory(address, .expect(spkiSha256: spkiSha256), nil)
        do {
            let reply = try await client.pair(
                PairRequest(code: code, deviceName: deviceName, spkiSha256: spkiSha256))
            guard reply.body.serverID == server.id else {
                throw RemoteError.badReply(
                    "the host at \(address) is \(reply.body.serverID), not \(server.id)")
            }
            try credentials.setPin(spkiSha256, for: server.id)
            try credentials.setToken(reply.body.token, for: server.id)
            server.needsPairing = false
            server.pairedAt = Date()
            server.scope = reply.body.scope
            if !server.addresses.contains(address) {
                server.addresses.insert(address, at: 0)
            }
            identityChanged = false
            api = nil
            nextTry = .distantPast
            state = .connecting
            lastError = nil
        } catch {
            state = .unpaired
            lastError = "\(error)"
            throw error
        }
    }

    /// Verify the pin, present the token, check `/info`, subscribe,
    /// then fetch the collections and apply what arrived meanwhile.
    /// Each failure is this host's alone.
    public func connect() async {
        guard server.enabled else { return }
        guard let address else {
            state = .offline(reason: "no address")
            return
        }
        guard let pin = try? credentials.pin(for: server.id),
            let token = try? credentials.token(for: server.id)
        else {
            server.needsPairing = true
            state = .unpaired
            return
        }
        if api == nil {
            api = factory(address, .expect(spkiSha256: pin), token)
        }
        guard let api else { return }
        closeSubscription()
        state = .connecting
        changed()
        do {
            let reply = try await api.info()
            let info = reply.body
            guard info.serverID == server.id else {
                state = .offline(
                    reason: "the host at \(address) is another installation (\(info.serverName))")
                self.api = nil
                changed()
                return
            }
            guard info.protocolVersion == RemoteProtocol.version else {
                state = .offline(
                    reason: "protocol \(info.protocolVersion) is not the \(RemoteProtocol.version) this client speaks")
                changed()
                return
            }
            self.info = info
            server.name = info.serverName
            let subscription = try await api.events()
            let hello = try await firstFrame(of: subscription)
            guard hello.kind == .hello, let epoch = hello.epoch else {
                subscription.close()
                throw RemoteError.badReply("the first frame was not hello")
            }
            if let snapshot, snapshot.epoch != epoch {
                // The host restarted: everything is refetched from zero.
                held = [:]
                self.snapshot = nil
            }
            self.subscription = subscription
            buffering = true
            buffer = []
            startPump(subscription)
            for key in watchedKeys {
                try await send(watch: true, key: key, on: subscription)
            }
            state = .syncing
            try await fetchAll(api: api)
            buffering = false
            let pending = buffer
            buffer = []
            for frame in pending {
                try await apply(frame, api: api)
            }
            for key in watchedKeys {
                await read(key)
            }
            failures = 0
            nextTry = .distantPast
            lastError = nil
            state = .connected
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// The frames after `hello`, each applied as it arrives.
    private func startPump(_ subscription: EventSubscription) {
        pump?.cancel()
        pump = Task { [weak self] in
            do {
                for try await frame in subscription.frames {
                    guard let self else { return }
                    await self.received(frame)
                }
                await self?.socketEnded(reason: "the host closed the events socket")
            } catch {
                await self?.socketEnded(reason: "\(error)")
            }
        }
    }

    private func received(_ frame: EventFrame) async {
        if buffering {
            buffer.append(frame)
            return
        }
        guard let api else { return }
        do {
            try await apply(frame, api: api)
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    private func socketEnded(reason: String) {
        guard subscription != nil else { return }
        subscription = nil
        pump = nil
        if case .unpaired = state { return }
        lastError = reason
        failures += 1
        nextTry = Date().addingTimeInterval(backoff)
        state = .reconnecting(attempt: failures)
        changed()
    }

    private func firstFrame(of subscription: EventSubscription) async throws -> EventFrame {
        try await withThrowingTaskGroup(of: EventFrame.self) { group in
            group.addTask {
                for try await frame in subscription.frames {
                    return frame
                }
                throw RemoteError.transport("the events socket closed before hello")
            }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(helloTimeout * 1_000_000_000))
                throw RemoteError.transport("no hello within \(Int(helloTimeout)) s")
            }
            let first = try await group.next()!
            group.cancelAll()
            return first
        }
    }

    /// Fetch every collection; a reply older than the held revision
    /// for its key is dropped.
    private func fetchAll(api: HostAPI) async throws {
        let projects = try await api.projects()
        let spaces = try await api.spaces()
        var next = snapshot ?? HostSnapshot(epoch: projects.epoch, revision: 0)
        if next.epoch != projects.epoch {
            held = [:]
            clearContent()
            next = HostSnapshot(epoch: projects.epoch, revision: 0)
        }
        next.fetchedAt = Date()
        apply(.projects, at: projects.revision) { next.projects = projects.body.projects }
        apply(.spaces, at: spaces.revision) { next.spaces = spaces.body.spaces }
        for project in next.projects {
            let reply = try await api.sessions(project: project.id)
            apply(.sessions(project: project.id), at: reply.revision) {
                next.sessions.removeAll { $0.project == project.id }
                next.sessions.append(contentsOf: reply.body.sessions)
            }
        }
        for space in next.spaces {
            let reply = try await api.sets(space: space.id)
            apply(.sets(space: space.id), at: reply.revision) {
                next.sets.removeAll { $0.space == space.id }
                next.sets.append(contentsOf: reply.body.sets)
            }
        }
        next.revision = held.values.max() ?? 0
        snapshot = next
    }

    /// One event against the held revisions: a `changed` past what a
    /// resource was fetched at refetches that resource, a `removed`
    /// drops it, and anything at or below the held revision was already
    /// in the snapshot.
    private func apply(_ frame: EventFrame, api: HostAPI) async throws {
        switch frame.kind {
        case .hello, .watching, .unknown:
            return
        case .resyncRequired:
            resyncs += 1
            held = [:]
            closeSubscription()
            lastError = "the host asked for a resync"
            state = .reconnecting(attempt: failures)
            nextTry = .distantPast
            changed()
            return
        case .changed, .removed:
            break
        }
        guard let ref = frame.resource, let key = ResourceKey(ref), let revision = frame.revision
        else { return }
        if let epoch = frame.epoch, let snapshot, epoch != snapshot.epoch {
            // A frame from another run of the host: start over.
            held = [:]
            self.snapshot = nil
            clearContent()
            try await fetchAll(api: api)
            changed()
            return
        }
        if frame.kind == .removed {
            remove(key, at: revision)
            changed()
            return
        }
        if let heldRevision = heldRevision(for: key), revision <= heldRevision { return }
        if key.isContent || { if case .runs = key { return true } else { return false } }()
            || { if case .draft = key { return true } else { return false } }()
        {
            await read(key)
            return
        }
        try await refetch(key, api: api)
        changed()
    }

    // MARK: Watched content

    /// Hold a watch on a content key: the host polls its source and
    /// says `changed` when it moves, and what is held on it is read
    /// then. The first holder sends `watch`; a second changes nothing
    /// on the host. The holder reads what it wants after this.
    public func watch(_ key: ResourceKey) async {
        let holders = (watches[key] ?? 0) + 1
        watches[key] = holders
        guard holders == 1, let subscription else { return }
        try? await send(watch: true, key: key, on: subscription)
    }

    /// Release one hold; the last sends `unwatch`.
    public func unwatch(_ key: ResourceKey) async {
        guard let holders = watches[key] else { return }
        if holders > 1 {
            watches[key] = holders - 1
            return
        }
        watches[key] = nil
        if let subscription {
            try? await send(watch: false, key: key, on: subscription)
        }
    }

    /// Hold a watch on a session's screen; the first holder reads it.
    public func watch(screen session: String) async {
        let first = (watches[.screen(session: session)] ?? 0) == 0
        await watch(.screen(session: session))
        if first {
            await readScreen(session)
        }
    }

    public func unwatch(screen session: String) async {
        await unwatch(.screen(session: session))
    }

    /// The keys watched now, in a fixed order.
    public var watchedKeys: [ResourceKey] {
        watches.keys.sorted { $0.wire.kind + $0.wire.id < $1.wire.kind + $1.wire.id }
    }

    /// The sessions whose screens are watched, for tests.
    public var watchedScreens: [String] {
        watches.keys.compactMap {
            if case .screen(let session) = $0 { return session } else { return nil }
        }.sorted()
    }

    private func send(watch: Bool, key: ResourceKey, on subscription: EventSubscription)
        async throws
    {
        let (kind, id) = key.wire
        let frame: [String: Any] = [
            "type": watch ? "watch" : "unwatch",
            "resource": ["kind": kind, "id": id],
        ]
        let data = try JSONSerialization.data(withJSONObject: frame)
        try await subscription.send(String(decoding: data, as: UTF8.self))
    }

    /// Re-read what is held on a key: after `changed`, and after a
    /// reconnect for every watched key. A conversation key refreshes
    /// the last response and the conversation, whichever are held,
    /// the latter from the last turn held on (it was in progress).
    private func read(_ key: ResourceKey) async {
        switch key {
        case .screen(let session):
            await readScreen(session)
        case .conversation(let session):
            if lastResponses[session] != nil || conversations[session] == nil {
                await readLastResponse(session)
            }
            // From the last turn held to the end: a page holds only
            // so many turns, and more than that may have arrived.
            var from = conversations[session]?.lastHeld
            for _ in 0..<Self.followHops {
                guard from != nil else { break }
                from = await readConversation(session, from: from, before: nil, limit: nil)
            }
        case .runs(let session):
            if runs[session] != nil { await readRuns(session) }
        case .runOutput(let session, let run):
            if runOutputs[RunKey(session: session, n: run)] != nil {
                await readRunOutput(session, run: run, lines: nil)
            }
        case .draft(let session):
            if drafts[session] != nil { await readDraft(session) }
        default:
            return
        }
    }

    private func clearContent() {
        screens = [:]
        lastResponses = [:]
        conversations = [:]
        runs = [:]
        runOutputs = [:]
        drafts = [:]
    }

    // MARK: The draft

    /// Whether the host keeps a draft per session (§4.5): an older
    /// host without the capability leaves the device its own field.
    public var hasDraft: Bool {
        info?.capabilities.contains("draft") ?? false
    }

    /// Read a session's draft now, under the revision rule. A refusal
    /// (not an agent session) clears what was held.
    public func readDraft(_ session: String) async {
        guard let api else { return }
        do {
            let reply = try await api.draft(session)
            apply(.draft(session: session), at: reply.revision) {
                drafts[session] = reply.body.draft
            }
            changed()
        } catch RemoteError.refused(let status, _, _) where status == 400 {
            drafts[session] = nil
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// What became of a change to the draft.
    private func took(_ session: String, _ reply: Envelope<DraftBody>) -> DraftOutcome {
        apply(.draft(session: session), at: reply.revision) {
            drafts[session] = reply.body.draft
        }
        changed()
        return .applied(reply.body.draft)
    }

    private func draftRefusal(_ status: Int, _ code: String, _ message: String) -> DraftOutcome {
        if status == 409 || code == "interrupted" { return .interrupted }
        if status == 401 || (status == 403 && code != "scope") {
            handle(.refused(status: status, code: code, message: message))
        }
        return .refused(message)
    }

    /// One of the editor's tools on the draft, or the text replaced
    /// whole. The reply is the draft as it stands.
    public func draftAction(_ session: String, _ tool: DraftTool) async -> DraftOutcome {
        await draftAction(session, DraftActionBody(action: tool.rawValue))
    }

    public func replaceDraft(_ session: String, text: String) async -> DraftOutcome {
        await draftAction(session, DraftActionBody(action: "replace", text: text))
    }

    private func draftAction(_ session: String, _ body: DraftActionBody) async -> DraftOutcome {
        guard let api else { return .unanswered("not connected") }
        do {
            return took(session, try await api.draftAction(session, body))
        } catch RemoteError.refused(let status, let code, let message) {
            return draftRefusal(status, code, message)
        } catch let error as RemoteError {
            if case .pinMismatch = error { handle(error) }
            return .unanswered("\(error)")
        } catch {
            return .unanswered("\(error)")
        }
    }

    /// Send the draft to the pane under an op of its own (§4.5): a
    /// timeout is followed by the op's status, never a blind resend,
    /// and a send that went has cleared the draft on the host, which
    /// is read again then.
    public func sendDraft(_ session: String, op: String = UUID().uuidString) async -> DraftOutcome {
        guard let api else { return .unanswered("not connected") }
        for attempt in 0...1 {
            do {
                return took(session, try await api.draftAction(session, DraftActionBody(action: "send", op: op)))
            } catch RemoteError.refused(let status, let code, let message) {
                return draftRefusal(status, code, message)
            } catch let error as RemoteError {
                if case .pinMismatch = error {
                    handle(error)
                    return .unanswered("\(error)")
                }
                lastError = "\(error)"
            } catch {
                lastError = "\(error)"
            }
            switch await status(of: op) {
            case .some(.done(failed: nil)):
                await readDraft(session)
                return drafts[session].map(DraftOutcome.applied) ?? .unanswered("sent; the draft could not be read")
            case .some(.done(failed: .some(let reason))): return .refused(reason)
            case .some(.interrupted), .some(.inProgress): return .interrupted
            case .some(.unknown): if attempt == 0 { continue }
            case .none: break
            }
            break
        }
        return .unanswered(lastError ?? "no answer")
    }

    /// The device's speech events into the draft, in order. The reply
    /// is the draft with them landed.
    public func dictate(_ session: String, recognizer: UInt64, events: [DictationEvent]) async -> DraftOutcome {
        guard let api else { return .unanswered("not connected") }
        do {
            return took(session, try await api.dictate(session, DictationBody(session: recognizer, events: events)))
        } catch RemoteError.refused(let status, let code, let message) {
            return draftRefusal(status, code, message)
        } catch let error as RemoteError {
            if case .pinMismatch = error { handle(error) }
            return .unanswered("\(error)")
        } catch {
            return .unanswered("\(error)")
        }
    }

    /// Read the last response now, under the revision rule. A refusal
    /// (no conversation) clears what was held.
    public func readLastResponse(_ session: String) async {
        guard let api else { return }
        do {
            let reply = try await api.lastResponse(session)
            apply(.conversation(session: session), at: reply.revision) {
                lastResponses[session] = reply.body
            }
            changed()
        } catch RemoteError.refused(let status, _, _) where status == 400 || status == 404 {
            lastResponses[session] = nil
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// Read a page of the conversation: from `from`, or the last page
    /// when `nil`, merged into what is held.
    /// How many pages a follow-up of a change reads toward the end
    /// before it leaves the rest to the next change.
    public static let followHops = 10

    /// A command under the control scope (§4.5). The op is made here
    /// and is the idempotency key: a command that gets no answer is
    /// followed by a status query, never a blind resend; `unknown`
    /// means the host never saw it and it goes again with the same op,
    /// `interrupted` means it started and the outcome is not known.
    public func command(
        _ session: String, _ verb: CommandVerb, text: String? = nil, op: String = UUID().uuidString
    ) async -> CommandOutcome {
        guard let api else { return .unanswered("not connected") }
        for attempt in 0...1 {
            do {
                _ = try await api.command(session, verb, op: op, text: text)
                return .accepted
            } catch RemoteError.refused(let status, let code, let message) {
                if status == 409 || code == "interrupted" { return .interrupted }
                if status == 401 || (status == 403 && code != "scope") {
                    handle(.refused(status: status, code: code, message: message))
                }
                return .refused(message)
            } catch let error as RemoteError {
                if case .pinMismatch = error {
                    handle(error)
                    return .unanswered("\(error)")
                }
                lastError = "\(error)"
            } catch {
                lastError = "\(error)"
            }
            // No answer either way: ask what became of the op.
            switch await status(of: op) {
            case .some(.done(failed: nil)): return .accepted
            case .some(.done(failed: .some(let reason))): return .refused(reason)
            case .some(.interrupted), .some(.inProgress): return .interrupted
            case .some(.unknown): if attempt == 0 { continue }
            case .none: break
            }
            break
        }
        return .unanswered(lastError ?? "no answer")
    }

    private func status(of op: String) async -> OpStatusView? {
        guard let api else { return nil }
        return try? await api.opStatus(op).body.status
    }

    public func readConversation(_ session: String, from: Int?, limit: Int?) async {
        _ = await readConversation(session, from: from, before: nil, limit: limit)
    }

    /// The page before turn `before` when given, else the one from
    /// `from`, or the tail when neither. Returns the page's `next`:
    /// the turn to continue from when the page stopped short.
    @discardableResult
    public func readConversation(_ session: String, from: Int?, before: Int?, limit: Int?) async
        -> Int?
    {
        guard let api else { return nil }
        do {
            let reply = try await api.conversation(
                session, from: before == nil ? from.map(String.init) ?? "last" : nil,
                before: before, limit: limit)
            apply(.conversation(session: session), at: reply.revision) {
                if var held = conversations[session] {
                    held.merge(reply.body)
                    conversations[session] = held
                } else {
                    conversations[session] = ConversationState(page: reply.body)
                }
            }
            changed()
            return reply.body.next
        } catch RemoteError.refused(let status, _, _) where status == 400 || status == 404 {
            conversations[session] = nil
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
        return nil
    }

    /// One turn whole, put in place of the page's copy.
    public func readTurn(_ session: String, n: Int) async {
        guard let api else { return }
        do {
            let reply = try await api.turn(session, n: n)
            apply(.conversation(session: session), at: reply.revision) {
                conversations[session]?.fill(reply.body)
            }
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    public func readRuns(_ session: String) async {
        guard let api else { return }
        do {
            let reply = try await api.runs(session)
            apply(.runs(session: session), at: reply.revision) {
                runs[session] = reply.body.runs
            }
            changed()
        } catch RemoteError.refused(let status, _, _) where status == 400 || status == 404 {
            runs[session] = nil
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    public func readRunOutput(_ session: String, run: Int, lines: Int?) async {
        guard let api else { return }
        let key = RunKey(session: session, n: run)
        do {
            let reply = try await api.runOutput(session, run: run, lines: lines)
            apply(.runOutput(session: session, run: run), at: reply.revision) {
                runOutputs[key] = reply.body
            }
            changed()
        } catch RemoteError.refused(let status, _, _) where status == 400 || status == 404 {
            runOutputs[key] = nil
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// Read a session's screen now, watched or not, under the revision
    /// rule. A refusal (the pane is not running) clears what was held;
    /// a transport failure is the connection's.
    public func readScreen(_ session: String, lines: Int? = nil) async {
        guard let api else { return }
        do {
            let reply = try await api.screen(session, lines: lines)
            apply(.screen(session: session), at: reply.revision) {
                screens[session] = reply.body
            }
            changed()
        } catch RemoteError.refused(let status, _, _) where status == 400 {
            screens[session] = nil
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// The revision a resource was last observed at: its own, or the
    /// collection's it was fetched as a member of.
    private func heldRevision(for key: ResourceKey) -> UInt64? {
        if let own = held[key] { return own }
        switch key {
        case .project: return held[.projects]
        case .space: return held[.spaces]
        case .session(let id):
            guard let project = snapshot?.sessions.first(where: { $0.id == id })?.project else {
                return nil
            }
            return held[.sessions(project: project)]
        case .set(let id):
            guard let space = snapshot?.sets.first(where: { $0.id == id })?.space else {
                return nil
            }
            return held[.sets(space: space)]
        default: return nil
        }
    }

    private func refetch(_ key: ResourceKey, api: HostAPI) async throws {
        var next = snapshot ?? HostSnapshot(epoch: "", revision: 0)
        switch key {
        case .projects, .project:
            let reply = try await api.projects()
            apply(.projects, at: reply.revision) { next.projects = reply.body.projects }
        case .spaces, .space:
            let reply = try await api.spaces()
            apply(.spaces, at: reply.revision) { next.spaces = reply.body.spaces }
        case .sets(let space):
            let reply = try await api.sets(space: space)
            apply(key, at: reply.revision) {
                next.sets.removeAll { $0.space == space }
                next.sets.append(contentsOf: reply.body.sets)
            }
        case .set(let id):
            // A set is read through its space's collection.
            let space = next.sets.first { $0.id == id }?.space
            let spaces = space.map { [$0] } ?? next.spaces.map(\.id)
            for space in spaces {
                let reply = try await api.sets(space: space)
                apply(.sets(space: space), at: reply.revision) {
                    next.sets.removeAll { $0.space == space }
                    next.sets.append(contentsOf: reply.body.sets)
                }
            }
        case .sessions(let project):
            let reply = try await api.sessions(project: project)
            apply(key, at: reply.revision) {
                next.sessions.removeAll { $0.project == project }
                next.sessions.append(contentsOf: reply.body.sessions)
            }
        case .session(let id):
            let reply = try await api.session(id)
            apply(key, at: reply.revision) {
                next.sessions.removeAll { $0.id == id }
                next.sessions.append(reply.body.session)
            }
        case .waiting:
            // Derived from the sessions, whose own events accompany it.
            return
        case .screen, .conversation, .runs, .runOutput, .draft:
            // Content: read through its own reader, never as a view.
            return
        }
        next.revision = held.values.max() ?? 0
        next.fetchedAt = Date()
        snapshot = next
    }

    private func remove(_ key: ResourceKey, at revision: UInt64) {
        guard var next = snapshot else { return }
        switch key {
        case .session(let id):
            next.sessions.removeAll { $0.id == id }
            screens[id] = nil
            lastResponses[id] = nil
            conversations[id] = nil
            runs[id] = nil
            drafts[id] = nil
        case .set(let id): next.sets.removeAll { $0.id == id }
        case .project(let id):
            next.projects.removeAll { $0.id == id }
            next.sessions.removeAll { $0.project == id }
        case .space(let id):
            next.spaces.removeAll { $0.id == id }
            next.sets.removeAll { $0.space == id }
        default: return
        }
        held[key] = revision
        next.revision = max(next.revision, revision)
        snapshot = next
    }

    /// The revision rule: apply only at or past the held revision.
    private func apply(_ key: ResourceKey, at revision: UInt64, _ body: () -> Void) {
        if let heldRevision = held[key], revision < heldRevision { return }
        held[key] = revision
        body()
    }

    /// Replace the sessions from a reply, for tests: the same rule as a
    /// fetch.
    public func applySessions(_ reply: Envelope<SessionsBody>) {
        var next = snapshot ?? HostSnapshot(epoch: reply.epoch, revision: reply.revision)
        if next.epoch != reply.epoch {
            held = [:]
            next = HostSnapshot(epoch: reply.epoch, revision: reply.revision)
        }
        apply(.sessions(project: "*"), at: reply.revision) { next.sessions = reply.body.sessions }
        next.revision = held.values.max() ?? 0
        snapshot = next
    }

    /// A full refetch by hand (the command line's `hosts`); events make
    /// it unnecessary while connected.
    public func refresh() async {
        guard server.enabled, let api, state.isConnected else {
            await connect()
            return
        }
        do {
            try await fetchAll(api: api)
            changed()
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    private func closeSubscription() {
        pump?.cancel()
        pump = nil
        subscription?.close()
        subscription = nil
        buffering = false
        buffer = []
    }

    private func disconnect(to next: ConnectionState) {
        closeSubscription()
        api = nil
        state = next
        changed()
    }

    private func handle(_ error: RemoteError) {
        lastError = "\(error)"
        switch error {
        case .pinMismatch:
            // A new key under a known id: the token is useless and
            // nothing from the old host is believed. The entry stays for
            // a new pairing.
            identityChanged = true
            try? credentials.setToken(nil, for: server.id)
            try? credentials.setPin(nil, for: server.id)
            snapshot = nil
            held = [:]
            clearContent()
            server.needsPairing = true
            disconnect(to: .unpaired)
        case .refused(let status, _, _) where status == 401 || status == 403:
            try? credentials.setToken(nil, for: server.id)
            server.needsPairing = true
            disconnect(to: .unpaired)
        default:
            failures += 1
            nextTry = Date().addingTimeInterval(backoff)
            closeSubscription()
            state = .reconnecting(attempt: failures)
            changed()
        }
    }

    /// The wait before the next try: bounded exponential backoff with
    /// jitter, in seconds.
    public var backoff: TimeInterval {
        guard failures > 0 else { return 0 }
        let base = min(30.0, pow(2.0, Double(failures - 1)))
        return base + Double.random(in: 0..<(base / 2))
    }

    /// Drop everything this host gave us and its credentials.
    public func forget() {
        try? credentials.forget(server.id)
        snapshot = nil
        held = [:]
        clearContent()
        watches = [:]
        info = nil
        server.needsPairing = true
        disconnect(to: .unpaired)
    }
}
