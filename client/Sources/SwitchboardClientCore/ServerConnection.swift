import Foundation
import SwitchboardProtocol
import SwitchboardTransport

/// Where one host stands (§6.2).
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

/// The resource keys the client holds a revision for. One per
/// collection; a session's metadata, conversation and screen will be
/// separate keys when the content endpoints arrive.
public enum ResourceKey: Hashable, Sendable {
    case projects
    case spaces
    case sets
    case sessions
}

/// Makes the network side for an address, so tests pass a fake.
public typealias HostAPIFactory = @Sendable (Address, PinPolicy, String?) -> HostAPI

public let liveHostAPI: HostAPIFactory = { address, pin, token in
    HostClient(address: address, pin: pin, token: token)
}

/// One known host: its state machine, its credentials, its snapshot,
/// and the revision rule that drops an older reply arriving late. The
/// manager owns one per known server; a stall here never touches
/// another host.
public actor ServerConnection {
    public private(set) var server: KnownServer
    public private(set) var state: ConnectionState
    public private(set) var snapshot: HostSnapshot?
    public private(set) var info: ServerInfo?
    /// The host's key changed under its id: nothing is trusted until a
    /// new pairing, and the cached snapshot is dropped.
    public private(set) var identityChanged = false
    public private(set) var lastError: String?

    private let credentials: CredentialStore
    private let factory: HostAPIFactory
    private let deviceName: String
    private var api: HostAPI?
    private var held: [ResourceKey: UInt64] = [:]
    private var failures = 0

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

    /// Where it will be reached next.
    public var address: Address? { server.addresses.first }

    public func setAddresses(_ addresses: [Address]) {
        guard !addresses.isEmpty, addresses != server.addresses else { return }
        server.addresses = addresses
        api = nil
    }

    public func setEnabled(_ enabled: Bool) {
        server.enabled = enabled
        if !enabled {
            state = .offline(reason: "disabled")
            api = nil
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
            state = .connecting
            lastError = nil
        } catch {
            state = .unpaired
            lastError = "\(error)"
            throw error
        }
    }

    /// Verify the pin, present the token, check `/info`, then fetch the
    /// collections. Each failure is this host's alone.
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
        if case .connected = state {} else { state = .connecting }
        do {
            let reply = try await api.info()
            let info = reply.body
            guard info.serverID == server.id else {
                state = .offline(
                    reason: "the host at \(address) is another installation (\(info.serverName))")
                self.api = nil
                return
            }
            guard info.protocolVersion == RemoteProtocol.version else {
                state = .offline(
                    reason: "protocol \(info.protocolVersion) is not the \(RemoteProtocol.version) this client speaks")
                return
            }
            self.info = info
            server.name = info.serverName
            if let snapshot, snapshot.epoch != reply.epoch {
                // The host restarted: everything is refetched from zero.
                held = [:]
                self.snapshot = nil
            }
            state = .syncing
            _ = try await refresh(api: api, epoch: reply.epoch)
            failures = 0
            lastError = nil
            state = .connected
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// Refetch the collections; a reply older than the held revision
    /// for its key is dropped.
    public func refresh() async {
        guard server.enabled, let api, state.isConnected || state == .syncing else {
            await connect()
            return
        }
        do {
            if try await refresh(api: api, epoch: snapshot?.epoch) {
                // The host restarted, or another one answers here: back
                // through `/info` and a refetch from zero.
                await connect()
                return
            }
            failures = 0
            lastError = nil
            state = .connected
        } catch let error as RemoteError {
            handle(error)
        } catch {
            handle(.transport("\(error)"))
        }
    }

    /// Fetch the collections and apply them; `true` when the host's
    /// epoch is not the one expected, in which case nothing is applied.
    private func refresh(api: HostAPI, epoch: String?) async throws -> Bool {
        let projects = try await api.projects()
        if let epoch, projects.epoch != epoch {
            held = [:]
            snapshot = nil
            return true
        }
        let spaces = try await api.spaces()
        var sessions: [(UInt64, [SessionView])] = []
        for project in projects.body.projects {
            let reply = try await api.sessions(project: project.id)
            sessions.append((reply.revision, reply.body.sessions))
        }
        var sets: [(UInt64, [SetView])] = []
        for space in spaces.body.spaces {
            let reply = try await api.sets(space: space.id)
            sets.append((reply.revision, reply.body.sets))
        }
        let now = Date()
        var next = snapshot ?? HostSnapshot(epoch: projects.epoch, revision: 0, fetchedAt: now)
        next.epoch = projects.epoch
        next.fetchedAt = now
        apply(.projects, at: projects.revision) { next.projects = projects.body.projects }
        apply(.spaces, at: spaces.revision) { next.spaces = spaces.body.spaces }
        let sessionRevision = sessions.map(\.0).min() ?? projects.revision
        apply(.sessions, at: sessionRevision) { next.sessions = sessions.flatMap(\.1) }
        let setRevision = sets.map(\.0).min() ?? spaces.revision
        apply(.sets, at: setRevision) { next.sets = sets.flatMap(\.1) }
        next.revision = held.values.max() ?? 0
        snapshot = next
        return false
    }

    /// The revision rule: apply only at or past the held revision.
    private func apply(_ key: ResourceKey, at revision: UInt64, _ body: () -> Void) {
        if let heldRevision = held[key], revision < heldRevision { return }
        held[key] = revision
        body()
    }

    /// Replace a collection from a reply, for the manager and tests:
    /// the same rule as a fetch.
    public func applySessions(_ reply: Envelope<SessionsBody>) {
        var next = snapshot ?? HostSnapshot(epoch: reply.epoch, revision: reply.revision)
        if next.epoch != reply.epoch {
            held = [:]
            next = HostSnapshot(epoch: reply.epoch, revision: reply.revision)
        }
        apply(.sessions, at: reply.revision) { next.sessions = reply.body.sessions }
        next.revision = held.values.max() ?? 0
        snapshot = next
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
            server.needsPairing = true
            api = nil
            state = .unpaired
        case .refused(let status, _, _) where status == 401 || status == 403:
            try? credentials.setToken(nil, for: server.id)
            server.needsPairing = true
            api = nil
            state = .unpaired
        default:
            failures += 1
            state = .reconnecting(attempt: failures)
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
        info = nil
        api = nil
        state = .unpaired
        server.needsPairing = true
    }
}
