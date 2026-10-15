import Foundation
import Network
import SwitchboardProtocol
import SwitchboardTransport

/// A session as the unified view shows it: qualified, labelled with its
/// source, and marked when its host is not connected.
public struct UnifiedSession: Identifiable, Sendable, Equatable {
    public let ref: SessionRef
    public let session: SessionView
    public let serverName: String
    public let stale: Bool

    public var id: SessionRef { ref }

    public init(ref: SessionRef, session: SessionView, serverName: String, stale: Bool) {
        self.ref = ref
        self.session = session
        self.serverName = serverName
        self.stale = stale
    }
}

public struct UnifiedSet: Identifiable, Sendable, Equatable {
    public let ref: WorkingSetRef
    public let set: SetView
    public let serverName: String
    public let stale: Bool

    public var id: WorkingSetRef { ref }
}

/// One host as the server screen lists it.
public struct HostStatus: Identifiable, Sendable, Equatable {
    public let server: KnownServer
    public let state: ConnectionState
    public let identityChanged: Bool
    public let lastError: String?
    public let fetchedAt: Date?

    public var id: ServerID { server.id }
}

/// The user's filters over the unified view (§8).
public struct Filters: Sendable, Equatable, Codable {
    public var servers: Set<ServerID> = []
    public var kinds: Set<SessionKind> = []
    public var cards: Set<String> = []

    public init() {}

    public func admits(_ item: UnifiedSession) -> Bool {
        (servers.isEmpty || servers.contains(item.ref.serverID))
            && (kinds.isEmpty || kinds.contains(item.session.kind))
            && (cards.isEmpty || cards.contains(item.session.card))
    }
}

/// The cross-host order: the host's own rank, then last activity,
/// then the record id.
public enum UnifiedOrder {
    public static func sort(_ items: [UnifiedSession]) -> [UnifiedSession] {
        items.sorted { a, b in
            let ra = CardRank.rank(of: a.session.card)
            let rb = CardRank.rank(of: b.session.card)
            if ra != rb { return ra < rb }
            let ta = a.session.lastStopAtMs ?? 0
            let tb = b.session.lastStopAtMs ?? 0
            if ta != tb { return ta > tb }
            return a.ref.recordID < b.ref.recordID
        }
    }
}

/// The unified read model over every host, built from the snapshots.
public struct UnifiedStore: Sendable, Equatable {
    public var hosts: [HostStatus] = []
    public var sessions: [UnifiedSession] = []
    public var sets: [UnifiedSet] = []

    public init() {}

    public static func build(from parts: [(KnownServer, ConnectionState, HostSnapshot?, Bool, String?)])
        -> UnifiedStore
    {
        var store = UnifiedStore()
        // Two hosts with one display name (two installations on one
        // Mac, say) are told apart by the short id on the label.
        var names: [String: Int] = [:]
        for (server, _, _, _, _) in parts { names[server.name, default: 0] += 1 }
        for (server, state, snapshot, changed, error) in parts {
            let label = names[server.name, default: 0] > 1
                ? "\(server.name) (\(server.id.short))" : server.name
            store.hosts.append(
                HostStatus(
                    server: server, state: state, identityChanged: changed, lastError: error,
                    fetchedAt: snapshot?.fetchedAt))
            guard let snapshot else { continue }
            let stale = !state.isConnected
            for session in snapshot.sessions {
                store.sessions.append(
                    UnifiedSession(
                        ref: SessionRef(serverID: server.id, recordID: session.id),
                        session: session, serverName: label, stale: stale))
            }
            for set in snapshot.sets {
                store.sets.append(
                    UnifiedSet(
                        ref: WorkingSetRef(serverID: server.id, setID: set.id), set: set,
                        serverName: label, stale: stale))
            }
        }
        store.sessions = UnifiedOrder.sort(store.sessions)
        store.sets.sort { ($0.serverName, $0.set.name, $0.ref.setID) < ($1.serverName, $1.set.name, $1.ref.setID) }
        return store
    }
}

/// What a discovered host says of itself before any trust: where it
/// was reached and its `/info`, whose key the owner checks against the
/// host's dialog before typing the code (§5).
public struct HostLookup: Sendable, Equatable {
    public let address: Address
    public let info: ServerInfo

    public init(address: Address, info: ServerInfo) {
        self.address = address
        self.info = info
    }
}

/// The address behind a browsed endpoint; `Discovery.resolve` outside
/// the tests.
public typealias EndpointResolver = @Sendable (NWEndpoint) async throws -> Address

/// Owns one `ServerConnection` per known server, the registry, and the
/// unified store. A connected host drives the store through its
/// events; the loop only retries hosts that are not connected, with
/// their backoff. Discovery adds candidates and addresses, never trust.
public actor ConnectionManager {
    public let registry: RegistryStore
    public let credentials: CredentialStore
    public let deviceName: String
    private let factory: HostAPIFactory
    private var connections: [ServerID: ServerConnection] = [:]
    private var servers: [KnownServer] = []
    public private(set) var discovered: Set<DiscoveredServer> = []
    public private(set) var store = UnifiedStore()
    private var loop: Task<Void, Never>?
    private var discovery: Discovery?
    private var onChange: (@Sendable () -> Void)?
    private var resolver: EndpointResolver = { try await Discovery.resolve($0) }

    public init(
        registry: RegistryStore, credentials: CredentialStore, deviceName: String,
        factory: @escaping HostAPIFactory = liveHostAPI
    ) {
        self.registry = registry
        self.credentials = credentials
        self.deviceName = deviceName
        self.factory = factory
    }

    public var known: [KnownServer] { servers }

    /// How a browsed endpoint becomes an address; the tests answer it
    /// themselves.
    public func setResolver(_ resolver: @escaping EndpointResolver) {
        self.resolver = resolver
    }

    /// Told after every rebuild of the store, so a view model mirrors
    /// it when it moves rather than on a timer.
    public func setOnChange(_ handler: @escaping @Sendable () -> Void) {
        onChange = handler
    }

    /// Load the registry and make a connection per server.
    public func load() async throws {
        servers = try registry.load()
        for server in servers where connections[server.id] == nil {
            connections[server.id] = await make(server)
        }
    }

    private func make(_ server: KnownServer) async -> ServerConnection {
        let connection = ServerConnection(
            server: server, credentials: credentials, deviceName: deviceName, factory: factory)
        await connection.setOnChange { [weak self] in
            guard let self else { return }
            Task { await self.rebuild() }
        }
        return connection
    }

    private func persist() async {
        var saved: [KnownServer] = []
        for server in servers {
            if let connection = connections[server.id] {
                saved.append(await connection.server)
            } else {
                saved.append(server)
            }
        }
        servers = saved
        try? registry.save(saved)
    }

    /// A host added by hand or from discovery, unpaired.
    @discardableResult
    public func add(id: ServerID, name: String, address: Address) async -> ServerConnection {
        if let existing = connections[id] {
            await existing.setAddresses([address] + (await existing.server.addresses).filter { $0 != address })
            await persist()
            return existing
        }
        let server = KnownServer(id: id, name: name, addresses: [address])
        servers.append(server)
        let connection = await make(server)
        connections[id] = connection
        await persist()
        return connection
    }

    /// Look a discovered host up before pairing: resolve its record,
    /// read `/info` under no trust, and refuse a host that is not the
    /// one the record named. Nothing here is believed; the key it
    /// presents is for the owner to compare with the host's dialog.
    public func lookup(_ record: DiscoveredServer) async throws -> HostLookup {
        let address = try await resolver(record.endpoint)
        let info = try await factory(address, .observe, nil).info().body
        if let id = record.serverID, id != info.serverID {
            throw RemoteError.badReply(
                "the host at \(address) is \(info.serverName), not the one advertised")
        }
        return HostLookup(address: address, info: info)
    }

    /// Pair with a host looked up, by the code typed from its dialog.
    /// The key it presented is pinned for the pairing, so the code goes
    /// only to the host whose key the owner checked; spaces and case
    /// in the code are the owner's to get wrong.
    public func pair(_ lookup: HostLookup, code: String) async throws {
        let code = code.filter { !$0.isWhitespace }.uppercased()
        try await pair(
            invite: PairingInvite(
                serverID: lookup.info.serverID, spkiSha256: lookup.info.spkiSha256, code: code,
                addresses: []),
            address: lookup.address)
    }

    /// Pair from an invite: the host is added if new, the key pinned
    /// before the code goes out, and connected on success.
    public func pair(invite: PairingInvite, address: Address? = nil) async throws {
        guard let address = address ?? invite.addresses.compactMap(Address.init).first else {
            throw RemoteError.transport("the invite names no address; enter one")
        }
        let connection = await add(id: invite.serverID, name: invite.serverID.short, address: address)
        try await connection.pair(code: invite.code, spkiSha256: invite.spkiSha256, at: address)
        await connection.connect()
        await persist()
        await rebuild()
    }

    public func connection(_ id: ServerID) -> ServerConnection? {
        connections[id]
    }

    /// Hold a watch on a session's screen on its host (§4.4).
    public func watch(screen ref: SessionRef) async {
        await connections[ref.serverID]?.watch(screen: ref.recordID)
    }

    public func unwatch(screen ref: SessionRef) async {
        await connections[ref.serverID]?.unwatch(screen: ref.recordID)
    }

    /// The screen last read for a session, if its host gave one.
    public func screen(for ref: SessionRef) async -> ScreenBody? {
        await connections[ref.serverID]?.screens[ref.recordID]
    }

    /// Hold a watch on a session's conversation; the last response and
    /// the conversation both move with it.
    public func watch(conversation ref: SessionRef) async {
        await connections[ref.serverID]?.watch(.conversation(session: ref.recordID))
    }

    public func unwatch(conversation ref: SessionRef) async {
        await connections[ref.serverID]?.unwatch(.conversation(session: ref.recordID))
    }

    public func readLastResponse(_ ref: SessionRef) async {
        await connections[ref.serverID]?.readLastResponse(ref.recordID)
    }

    public func lastResponse(for ref: SessionRef) async -> LastResponseBody? {
        await connections[ref.serverID]?.lastResponses[ref.recordID]
    }

    // MARK: Pinned files (§4.3)

    public func watch(file ref: FileRef) async {
        await connections[ref.serverID]?.watch(file: ref.projectID, path: ref.path)
    }

    public func unwatch(file ref: FileRef) async {
        await connections[ref.serverID]?.unwatch(file: ref.projectID, path: ref.path)
    }

    public func file(for ref: FileRef) async -> FileBody? {
        await connections[ref.serverID]?.files["\(ref.projectID)/\(ref.path)"]
    }

    // MARK: The draft (§4.5)

    /// Whether a host keeps a draft per session.
    public func hasDraft(_ id: ServerID) async -> Bool {
        await connections[id]?.hasDraft ?? false
    }

    public func draft(for ref: SessionRef) async -> DraftView? {
        await connections[ref.serverID]?.drafts[ref.recordID]
    }

    public func readDraft(_ ref: SessionRef) async {
        await connections[ref.serverID]?.readDraft(ref.recordID)
    }

    public func draftAction(_ ref: SessionRef, _ tool: DraftTool) async -> DraftOutcome {
        await connections[ref.serverID]?.draftAction(ref.recordID, tool) ?? .unanswered("no host")
    }

    public func replaceDraft(_ ref: SessionRef, text: String) async -> DraftOutcome {
        await connections[ref.serverID]?.replaceDraft(ref.recordID, text: text) ?? .unanswered("no host")
    }

    public func sendDraft(_ ref: SessionRef) async -> DraftOutcome {
        await connections[ref.serverID]?.sendDraft(ref.recordID) ?? .unanswered("no host")
    }

    public func dictate(_ ref: SessionRef, recognizer: UInt64, events: [DictationEvent]) async -> DraftOutcome {
        await connections[ref.serverID]?.dictate(ref.recordID, recognizer: recognizer, events: events)
            ?? .unanswered("no host")
    }

    /// Read a page of the conversation from `from`, or its end.
    public func readConversation(_ ref: SessionRef, from: Int?, limit: Int?) async {
        await connections[ref.serverID]?.readConversation(ref.recordID, from: from, limit: limit)
    }

    /// Text into the session's pane, as the app's own message box does.
    public func send(_ ref: SessionRef, text: String) async -> CommandOutcome {
        await connections[ref.serverID]?.command(ref.recordID, .send, text: text) ?? .unanswered("no host")
    }

    /// Resume a not-running agent; spends a turn, so the window confirms first.
    public func resume(_ ref: SessionRef) async -> CommandOutcome {
        await connections[ref.serverID]?.command(ref.recordID, .resume) ?? .unanswered("no host")
    }

    /// Answer Claude Code's folder question with yes.
    public func trust(_ ref: SessionRef) async -> CommandOutcome {
        await connections[ref.serverID]?.command(ref.recordID, .trust) ?? .unanswered("no host")
    }

    public func readTurn(_ ref: SessionRef, n: Int) async {
        await connections[ref.serverID]?.readTurn(ref.recordID, n: n)
    }

    public func readConversation(_ ref: SessionRef, before: Int, limit: Int?) async {
        await connections[ref.serverID]?.readConversation(
            ref.recordID, from: nil, before: before, limit: limit)
    }

    public func conversation(for ref: SessionRef) async -> ConversationState? {
        await connections[ref.serverID]?.conversations[ref.recordID]
    }

    public func readRuns(_ ref: SessionRef) async {
        await connections[ref.serverID]?.readRuns(ref.recordID)
    }

    public func runs(for ref: SessionRef) async -> [RunView]? {
        await connections[ref.serverID]?.runs[ref.recordID]
    }

    /// Hold a watch on one run's output.
    public func watch(runOutput ref: RunRef) async {
        await connections[ref.session.serverID]?.watch(
            .runOutput(session: ref.session.recordID, run: ref.n))
    }

    public func unwatch(runOutput ref: RunRef) async {
        await connections[ref.session.serverID]?.unwatch(
            .runOutput(session: ref.session.recordID, run: ref.n))
    }

    public func readRunOutput(_ ref: RunRef, lines: Int? = nil) async {
        await connections[ref.session.serverID]?.readRunOutput(
            ref.session.recordID, run: ref.n, lines: lines)
    }

    public func runOutput(for ref: RunRef) async -> RunOutputBody? {
        await connections[ref.session.serverID]?.runOutputs[RunKey(session: ref.session.recordID, n: ref.n)]
    }

    public func setEnabled(_ id: ServerID, _ enabled: Bool) async {
        await connections[id]?.setEnabled(enabled)
        await persist()
        await rebuild()
    }

    /// Remove the server, its credentials, its snapshot and its views.
    public func forget(_ id: ServerID) async {
        if let connection = connections.removeValue(forKey: id) {
            await connection.forget()
        }
        try? credentials.forget(id)
        servers.removeAll { $0.id == id }
        try? registry.save(servers)
        await rebuild()
    }

    /// What Bonjour shows now. A record whose `id` names a known server
    /// updates that server's address once it resolves; an unknown one
    /// is a candidate on the server screen. Two records under one id
    /// are two candidate addresses, and the pin decides which is real.
    public func discovered(_ set: Set<DiscoveredServer>) async {
        discovered = set
        for record in set {
            guard let id = record.serverID, let connection = connections[id] else { continue }
            if let address = try? await resolver(record.endpoint) {
                await connection.setAddresses([address])
            }
        }
    }

    /// A host that is not connected is looked up again in what Bonjour
    /// shows before each try: a host that moved sends no goodbye for
    /// its old record, so the change is seen by resolving, not browsing.
    /// Each host resolves on its own task, so one that answers slowly
    /// or not at all holds up no other host's try.
    private func reresolve(_ id: ServerID, _ connection: ServerConnection) async {
        guard let record = discovered.first(where: { $0.serverID == id }),
            let address = try? await resolver(record.endpoint)
        else { return }
        await connection.setAddresses([address])
    }

    /// Connect every enabled host, each on its own task.
    public func connectAll() async {
        await withTaskGroup(of: Void.self) { group in
            for connection in connections.values {
                group.addTask { await connection.connect() }
            }
        }
        await rebuild()
    }

    public func refreshAll() async {
        await withTaskGroup(of: Void.self) { group in
            for (id, connection) in connections {
                group.addTask {
                    let state = await connection.state
                    if !state.isConnected, state != .unpaired {
                        await self.reresolve(id, connection)
                    }
                    await connection.refresh()
                }
            }
        }
        await rebuild()
    }

    /// Connect what is not connected, every `interval` seconds, each
    /// host under its own backoff; a connected host needs nothing from
    /// the loop. Bonjour feeds addresses in between.
    public func start(interval: TimeInterval = 2, browse: Bool = true) {
        loop?.cancel()
        if browse, discovery == nil {
            let discovery = Discovery()
            self.discovery = discovery
            discovery.start { [weak self] set in
                guard let self else { return }
                Task { await self.discovered(set) }
            }
        }
        loop = Task { [weak self] in
            guard let self else { return }
            await self.connectAll()
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
                if Task.isCancelled { break }
                await self.retryOffline()
            }
        }
    }

    /// One pass of the loop: re-resolve and reconnect every host that
    /// is not connected and whose backoff has elapsed. A host in the
    /// middle of a connect is left to it: a second connect would close
    /// the first's subscription under its fetch.
    public func retryOffline() async {
        await withTaskGroup(of: Void.self) { group in
            for (id, connection) in connections {
                group.addTask {
                    let state = await connection.state
                    guard !state.isConnected, state != .unpaired, state != .connecting,
                        state != .syncing, await connection.dueForTry
                    else { return }
                    await self.reresolve(id, connection)
                    await connection.connect()
                }
            }
        }
        await rebuild()
    }

    public func stop() {
        loop?.cancel()
        loop = nil
        discovery?.stop()
        discovery = nil
    }

    /// Rebuild the unified store from every connection.
    public func rebuild() async {
        var parts: [(KnownServer, ConnectionState, HostSnapshot?, Bool, String?)] = []
        for server in servers {
            guard let connection = connections[server.id] else { continue }
            parts.append(
                (
                    await connection.server, await connection.state, await connection.snapshot,
                    await connection.identityChanged, await connection.lastError
                ))
        }
        store = UnifiedStore.build(from: parts)
        // Merge by id: a server added while this rebuild awaited the
        // connections is not in `parts` and must not be dropped.
        for (server, _, _, _, _) in parts {
            if let i = servers.firstIndex(where: { $0.id == server.id }) {
                servers[i] = server
            }
        }
        onChange?()
    }
}
