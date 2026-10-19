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

/// A host's projects and workspaces as last fetched, for a sheet that
/// opens a session on it.
public struct HostCatalog: Sendable, Equatable {
    public var projects: [ProjectView]
    public var spaces: [SpaceView]

    public init(projects: [ProjectView], spaces: [SpaceView]) {
        self.projects = projects
        self.spaces = spaces
    }

    /// The projects grouped by workspace, in the host's order of
    /// workspaces, the global view left out; a project whose workspace
    /// is not listed falls under its id.
    public var byWorkspace: [(name: String, projects: [ProjectView])] {
        var groups: [(name: String, projects: [ProjectView])] = []
        for space in spaces where !space.view {
            let members = projects.filter { $0.space == space.id }
            if !members.isEmpty { groups.append((space.name, members)) }
        }
        let known = Set(spaces.map(\.id))
        let orphans = projects.filter { !known.contains($0.space) }
        if !orphans.isEmpty { groups.append(("Other", orphans)) }
        return groups
    }
}

public struct UnifiedSet: Identifiable, Sendable, Equatable {
    public let ref: WorkingSetRef
    public let set: SetView
    public let serverName: String
    public let stale: Bool

    public var id: WorkingSetRef { ref }
}

/// One Dispatch ticket with its source, and the session of its
/// project's supervisor when the host names one.
public struct UnifiedTicket: Identifiable, Sendable, Equatable, Hashable {
    public let ref: TicketRef
    public let ticket: TicketView
    public let serverName: String
    public let stale: Bool
    public let supervisor: SessionRef?

    public var id: TicketRef { ref }

    public init(ref: TicketRef, ticket: TicketView, serverName: String, stale: Bool, supervisor: SessionRef?) {
        self.ref = ref
        self.ticket = ticket
        self.serverName = serverName
        self.stale = stale
        self.supervisor = supervisor
    }

    public static func == (lhs: UnifiedTicket, rhs: UnifiedTicket) -> Bool {
        lhs.ref == rhs.ref && lhs.ticket == rhs.ticket && lhs.serverName == rhs.serverName
            && lhs.stale == rhs.stale && lhs.supervisor == rhs.supervisor
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(ref)
        hasher.combine(ticket.updatedMs)
    }
}

/// One host as the server screen lists it.
public struct HostStatus: Identifiable, Sendable, Equatable {
    public let server: KnownServer
    public let state: ConnectionState
    public let identityChanged: Bool
    public let lastError: String?
    public let fetchedAt: Date?
    /// The host app's version as its `/info` said, once read.
    public let version: String?

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

/// How the ticket list is ordered.
public enum TicketSort: String, Sendable, Equatable, Codable, CaseIterable {
    /// Last moved first.
    case recent
    /// The ones waiting on the owner first, then last moved.
    case waiting
    /// Newest ticket first.
    case created
    /// By project, then last moved.
    case project

    public var label: String {
        switch self {
        case .recent: return "Recent"
        case .waiting: return "Waiting first"
        case .created: return "Newest"
        case .project: return "By project"
        }
    }
}

/// The user's filters over the ticket list, and its order. An empty
/// set admits everything; closed tickets are hidden unless asked for.
public struct TicketFilters: Sendable, Equatable, Codable {
    public var servers: Set<ServerID> = []
    public var projects: Set<String> = []
    /// `active`, `parked` or `closed` (which takes `closing` with it).
    public var states: Set<String> = []
    /// Only the tickets waiting on the owner.
    public var waitingOnly = false
    public var sort: TicketSort = .recent

    public init() {}

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        servers = try c.decodeIfPresent(Set<ServerID>.self, forKey: .servers) ?? []
        projects = try c.decodeIfPresent(Set<String>.self, forKey: .projects) ?? []
        states = try c.decodeIfPresent(Set<String>.self, forKey: .states) ?? []
        waitingOnly = try c.decodeIfPresent(Bool.self, forKey: .waitingOnly) ?? false
        sort = try c.decodeIfPresent(TicketSort.self, forKey: .sort) ?? .recent
    }

    /// The state word a ticket is filtered by: `closing` counts as closed,
    /// `parking` as parked.
    public static func stateWord(_ ticket: TicketView) -> String {
        switch ticket.state {
        case "closed", "closing": return "closed"
        case "parked", "parking": return "parked"
        default: return "active"
        }
    }

    public func admits(_ item: UnifiedTicket) -> Bool {
        let word = Self.stateWord(item.ticket)
        if states.isEmpty && word == "closed" { return false }
        return (servers.isEmpty || servers.contains(item.ref.serverID))
            && (projects.isEmpty || projects.contains(item.ticket.project))
            && (states.isEmpty || states.contains(word))
            && (!waitingOnly || item.ticket.waits)
    }

    /// The admitted tickets in the chosen order; ties keep the store's
    /// order, so a list is stable between pulls.
    public func apply(_ items: [UnifiedTicket]) -> [UnifiedTicket] {
        let kept = items.filter(admits)
        func recent(_ a: UnifiedTicket, _ b: UnifiedTicket) -> Bool {
            a.ticket.updatedMs > b.ticket.updatedMs
        }
        switch sort {
        case .recent:
            return kept.stableSorted(by: recent)
        case .waiting:
            return kept.stableSorted { a, b in
                if a.ticket.waits != b.ticket.waits { return a.ticket.waits }
                return recent(a, b)
            }
        case .created:
            return kept.stableSorted { $0.ticket.createdMs > $1.ticket.createdMs }
        case .project:
            return kept.stableSorted { a, b in
                if a.ticket.project != b.ticket.project { return a.ticket.project < b.ticket.project }
                return recent(a, b)
            }
        }
    }
}

extension Array {
    /// A sort that keeps equal elements in their order.
    fileprivate func stableSorted(by before: (Element, Element) -> Bool) -> [Element] {
        enumerated().sorted { a, b in
            if before(a.element, b.element) { return true }
            if before(b.element, a.element) { return false }
            return a.offset < b.offset
        }.map(\.element)
    }
}

/// What the ticket filter menu offers: the hosts with tickets, the
/// projects and the state words seen. Equal across polls that change
/// nothing the menu shows, as `FilterChoices`.
public struct TicketFilterChoices: Sendable, Equatable {
    public var hosts: [FilterChoices.Host] = []
    public var projects: [String] = []
    public var states: [String] = []

    public init() {}

    public init(store: UnifiedStore) {
        var seen: [ServerID: String] = [:]
        for item in store.tickets { seen[item.ref.serverID] = item.serverName }
        hosts = store.hosts.compactMap { host in
            seen[host.id].map { FilterChoices.Host(id: host.id, name: $0) }
        }
        projects = Array(Set(store.tickets.map(\.ticket.project))).sorted()
        states = ["active", "parked", "closed"].filter { word in
            store.tickets.contains { TicketFilters.stateWord($0.ticket) == word }
        }
    }
}

/// What the filter menu offers: the hosts by name and the card words
/// the listed sessions are in. A value of its own, equal across polls
/// that change nothing the menu shows, so an open menu is not rebuilt
/// by every refresh of the store (a rebuild sends it back to the top).
public struct FilterChoices: Sendable, Equatable {
    public struct Host: Identifiable, Sendable, Equatable {
        public let id: ServerID
        public let name: String

        public init(id: ServerID, name: String) {
            self.id = id
            self.name = name
        }
    }

    public var hosts: [Host] = []
    public var states: [String] = []

    public init() {}

    public init(store: UnifiedStore) {
        hosts = store.hosts.map { Host(id: $0.id, name: $0.server.name) }
        states = Array(Set(store.sessions.map(\.session.card))).sorted()
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
    /// Every host's tickets: the ones waiting on the owner first, then
    /// the host's own order (by project, oldest first).
    public var tickets: [UnifiedTicket] = []
    /// The hosts that serve tickets and whose runner answered last.
    public var dispatchHosts: [ServerID] = []

    public init() {}

    public static func build(
        from parts: [(KnownServer, ConnectionState, HostSnapshot?, Bool, String?)],
        versions: [ServerID: String] = [:]
    ) -> UnifiedStore {
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
                    fetchedAt: snapshot?.fetchedAt, version: versions[server.id]))
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
            if let listing = snapshot.tickets {
                if listing.connected { store.dispatchHosts.append(server.id) }
                for ticket in listing.tickets {
                    let supervisor = listing.projects.first { $0.name == ticket.project }?.supervisorSession
                    store.tickets.append(
                        UnifiedTicket(
                            ref: TicketRef(serverID: server.id, ticketID: ticket.id), ticket: ticket,
                            serverName: label, stale: stale,
                            supervisor: supervisor.map { SessionRef(serverID: server.id, recordID: $0) }))
                }
            }
        }
        store.tickets = store.tickets.enumerated().sorted { a, b in
            if a.element.ticket.waits != b.element.ticket.waits { return a.element.ticket.waits }
            return a.offset < b.offset
        }.map(\.element)
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

    /// One ticket read alone.
    public func readTicket(_ ref: TicketRef) async -> TicketView? {
        await connections[ref.serverID]?.readTicket(ref.ticketID)
    }

    /// Answer a ticket's decision.
    public func decide(_ ref: TicketRef, decision: String, answer: String, note: String?) async
        -> CommandOutcome
    {
        await connections[ref.serverID]?.decide(ref.ticketID, decision: decision, answer: answer, note: note)
            ?? .unanswered("no host")
    }

    /// Resume a parked ticket.
    public func resumeTicket(_ ref: TicketRef) async -> CommandOutcome {
        await connections[ref.serverID]?.resumeTicket(ref.ticketID) ?? .unanswered("no host")
    }

    /// Close a ticket.
    public func closeTicket(_ ref: TicketRef) async -> CommandOutcome {
        await connections[ref.serverID]?.closeTicket(ref.ticketID) ?? .unanswered("no host")
    }

    /// One file of a ticket by its key.
    public func readTicketFile(_ ref: TicketRef, key: String) async throws -> TicketFileBody {
        guard let connection = connections[ref.serverID] else { throw RemoteError.transport("no host") }
        return try await connection.readTicketFile(ref.ticketID, key: key)
    }

    /// A ticket's events, oldest first.
    public func readTicketEvents(_ ref: TicketRef) async throws -> TicketEventsBody {
        guard let connection = connections[ref.serverID] else { throw RemoteError.transport("no host") }
        return try await connection.readTicketEvents(ref.ticketID)
    }

    /// The hosts a session can be opened on from here: connected,
    /// under the control scope, and serving `create`.
    public var createHosts: Set<ServerID> {
        get async {
            var hosts: Set<ServerID> = []
            for (id, connection) in connections where await connection.servesCreate {
                guard await connection.state.isConnected, await connection.server.scope == .control else { continue }
                hosts.insert(id)
            }
            return hosts
        }
    }

    /// A host's projects and workspaces, once it has a snapshot.
    public func catalog(for host: ServerID) async -> HostCatalog? {
        guard let snapshot = await connections[host]?.snapshot else { return nil }
        return HostCatalog(projects: snapshot.projects, spaces: snapshot.spaces)
    }

    /// Open a session in a project on one host.
    public func newSession(
        on host: ServerID, project: String, name: String, kind: SessionKind, cwd: String?, prompt: String?
    ) async -> NewSessionOutcome {
        guard let connection = connections[host] else { return .unanswered("no host") }
        let outcome = await connection.newSession(
            project: project, name: name, kind: kind, cwd: cwd, prompt: prompt)
        await rebuild()
        return outcome
    }

    /// The hosts that serve a ticket's files, events and close.
    public var evidenceHosts: Set<ServerID> {
        get async {
            var hosts: Set<ServerID> = []
            for (id, connection) in connections where await connection.servesEvidence {
                hosts.insert(id)
            }
            return hosts
        }
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
        var versions: [ServerID: String] = [:]
        for server in servers {
            guard let connection = connections[server.id] else { continue }
            parts.append(
                (
                    await connection.server, await connection.state, await connection.snapshot,
                    await connection.identityChanged, await connection.lastError
                ))
            versions[server.id] = await connection.info?.version
        }
        store = UnifiedStore.build(from: parts, versions: versions)
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
