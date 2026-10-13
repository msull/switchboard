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
    /// How many views hold each watched session's screen; `watch` goes
    /// out at the first and `unwatch` at the last (§4.4), and every
    /// watch is resent after `hello` on a reconnect.
    private var watches: [String: Int] = [:]
    /// The text last read per session, watched or not, at the revision
    /// its key holds.
    public private(set) var screens: [String: ScreenBody] = [:]
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
            for session in watches.keys.sorted() {
                try await send(watch: true, session: session, on: subscription)
            }
            state = .syncing
            try await fetchAll(api: api)
            buffering = false
            let pending = buffer
            buffer = []
            for frame in pending {
                try await apply(frame, api: api)
            }
            for session in watches.keys.sorted() {
                await readScreen(session)
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
            screens = [:]
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
            screens = [:]
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
        if case .screen(let session) = key {
            await readScreen(session)
            return
        }
        try await refetch(key, api: api)
        changed()
    }

    // MARK: Watched content

    /// Hold a watch on a session's screen: the host polls the pane and
    /// says `changed` when it moves, and the text is read then. The
    /// first holder sends `watch` and reads once; a second changes
    /// nothing on the host.
    public func watch(screen session: String) async {
        let holders = (watches[session] ?? 0) + 1
        watches[session] = holders
        guard holders == 1 else { return }
        if let subscription {
            try? await send(watch: true, session: session, on: subscription)
        }
        await readScreen(session)
    }

    /// Release one hold; the last sends `unwatch`.
    public func unwatch(screen session: String) async {
        guard let holders = watches[session] else { return }
        if holders > 1 {
            watches[session] = holders - 1
            return
        }
        watches[session] = nil
        if let subscription {
            try? await send(watch: false, session: session, on: subscription)
        }
    }

    /// The sessions watched now, for tests.
    public var watchedScreens: [String] { watches.keys.sorted() }

    private func send(watch: Bool, session: String, on subscription: EventSubscription)
        async throws
    {
        let frame: [String: Any] = [
            "type": watch ? "watch" : "unwatch",
            "resource": ["kind": "screen", "id": session],
        ]
        let data = try JSONSerialization.data(withJSONObject: frame)
        try await subscription.send(String(decoding: data, as: UTF8.self))
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
        case .screen:
            // Content: read through `readScreen`, never as a view.
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
            screens = [:]
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
        screens = [:]
        watches = [:]
        info = nil
        server.needsPairing = true
        disconnect(to: .unpaired)
    }
}
