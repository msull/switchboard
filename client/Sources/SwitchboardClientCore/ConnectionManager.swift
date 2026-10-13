import Foundation
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
public struct Filters: Sendable, Equatable {
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
            if let address = try? await Discovery.resolve(record.endpoint) {
                await connection.setAddresses([address])
            }
        }
    }

    /// A host that is not connected is looked up again in what Bonjour
    /// shows before each try: a host that moved sends no goodbye for
    /// its old record, so the change is seen by resolving, not browsing.
    private func reresolveOffline() async {
        for (id, connection) in connections {
            let state = await connection.state
            guard !state.isConnected, state != .unpaired,
                let record = discovered.first(where: { $0.serverID == id }),
                let address = try? await Discovery.resolve(record.endpoint)
            else { continue }
            await connection.setAddresses([address])
        }
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
        await reresolveOffline()
        await withTaskGroup(of: Void.self) { group in
            for connection in connections.values {
                group.addTask { await connection.refresh() }
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
    /// is not connected and whose backoff has elapsed.
    public func retryOffline() async {
        await reresolveOffline()
        await withTaskGroup(of: Void.self) { group in
            for connection in connections.values {
                group.addTask {
                    let state = await connection.state
                    guard !state.isConnected, state != .unpaired, state != .syncing,
                        await connection.dueForTry
                    else { return }
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
    }
}
