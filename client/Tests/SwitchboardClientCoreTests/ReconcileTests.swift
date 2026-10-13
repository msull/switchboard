import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// The client half of docs/remote-client.md §11: qualified ids, the
/// revision rule, a host offline, a key change, forget, and a changed
/// address.
final class ReconcileTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let b = ServerID("22222222-2222-2222-2222-222222222222")
    let addrA = Address(host: "10.0.0.1", port: 43187)
    let addrB = Address(host: "10.0.0.2", port: 43187)

    /// Two hosts, both with a session `r1`, paired and connected.
    func twoHosts() async throws -> (ConnectionManager, FakeNetwork, MemoryCredentials) {
        let network = FakeNetwork()
        let hostA = FakeHost(serverID: a, name: "Desk", spki: "ka")
        let hostB = FakeHost(serverID: b, name: "Laptop", spki: "kb")
        for host in [hostA, hostB] {
            host.set {
                $0.spaces = [SpaceView(id: "s", name: "Space")]
                $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
                $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "one")]]
                $0.sets = ["s": [SetView(id: "w1", name: "Desk set", space: "s")]]
            }
        }
        network.hosts = [addrA: hostA, addrB: hostB]
        let credentials = MemoryCredentials()
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: credentials, deviceName: "test",
            factory: network.factory)
        try await manager.load()
        try await manager.pair(
            invite: PairingInvite(
                serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        try await manager.pair(
            invite: PairingInvite(
                serverID: b, spkiSha256: "kb", code: "ABCD2345", addresses: ["10.0.0.2:43187"]))
        await manager.connectAll()
        return (manager, network, credentials)
    }

    func host(_ store: UnifiedStore, _ id: ServerID) -> HostStatus {
        store.hosts.first { $0.id == id }!
    }

    func testTwoHostsWithTheSameRecordIdNeverCollide() async throws {
        let (manager, _, credentials) = try await twoHosts()
        let store = await manager.store
        XCTAssertEqual(store.hosts.map(\.state), [.connected, .connected])
        XCTAssertEqual(store.sessions.count, 2)
        XCTAssertEqual(Set(store.sessions.map(\.ref)).count, 2, "qualified by server")
        XCTAssertEqual(Set(store.sessions.map(\.serverName)), ["Desk", "Laptop"])
        XCTAssertEqual(store.sets.map(\.serverName), ["Desk", "Laptop"])
        XCTAssertEqual(try credentials.token(for: a), "tok-1")
        XCTAssertEqual(try credentials.pin(for: b), "kb")
        XCTAssertFalse(store.sessions.contains(where: \.stale))
    }

    func testAnOfflineHostKeepsItsItemsVisiblyStaleAndTheOtherWorks() async throws {
        let (manager, network, _) = try await twoHosts()
        network.hosts[addrB]!.set { $0.failure = .transport("connection refused") }
        await manager.refreshAll()
        var store = await manager.store
        let desk = store.sessions.first { $0.serverName == "Desk" }!
        let laptop = store.sessions.first { $0.serverName == "Laptop" }!
        XCTAssertFalse(desk.stale)
        XCTAssertTrue(laptop.stale, "the last snapshot stays, marked")
        XCTAssertEqual(host(store, a).state, .connected)
        XCTAssertEqual(host(store, b).state, .reconnecting(attempt: 1))
        network.hosts[addrB]!.set { $0.failure = nil }
        await manager.refreshAll()
        store = await manager.store
        XCTAssertEqual(store.hosts.map(\.state), [.connected, .connected])
        XCTAssertFalse(store.sessions.contains(where: \.stale))
    }

    func testAnOlderReplyArrivingLateIsDroppedByRevision() async throws {
        let connection = ServerConnection(
            server: KnownServer(id: a, name: "Desk", addresses: [addrA]),
            credentials: MemoryCredentials(), deviceName: "t", factory: FakeNetwork().factory)
        let newer = Envelope(
            epoch: "e1", revision: 5,
            body: SessionsBody(sessions: [SessionView(id: "r1", project: "p", name: "new")]))
        let older = Envelope(epoch: "e1", revision: 3, body: SessionsBody(sessions: []))
        await connection.applySessions(newer)
        await connection.applySessions(older)
        var names = await connection.snapshot?.sessions.map { $0.name }
        XCTAssertEqual(names, ["new"], "the late one is dropped")
        let same = Envelope(epoch: "e1", revision: 5, body: SessionsBody(sessions: []))
        await connection.applySessions(same)
        names = await connection.snapshot?.sessions.map { $0.name }
        XCTAssertEqual(names, [], "at the held revision applies")
        // A new epoch: the host restarted, and its counter starts over.
        let restarted = Envelope(
            epoch: "e2", revision: 1,
            body: SessionsBody(sessions: [SessionView(id: "r1", project: "p", name: "again")]))
        await connection.applySessions(restarted)
        names = await connection.snapshot?.sessions.map { $0.name }
        XCTAssertEqual(names, ["again"])
    }

    func testAKeyChangeUnderAKnownIdDropsTheTokenAndKeepsTheServer() async throws {
        let (manager, network, credentials) = try await twoHosts()
        // The host's key rotated: the pinned handshake fails from now on.
        network.hosts[addrA]!.set { $0.spki = "ka2" }
        await manager.refreshAll()
        var store = await manager.store
        XCTAssertEqual(host(store, a).state, .unpaired)
        XCTAssertTrue(host(store, a).identityChanged)
        XCTAssertTrue(host(store, a).server.needsPairing)
        XCTAssertNil(try credentials.token(for: a), "nothing is sent with the old token")
        XCTAssertNil(try credentials.pin(for: a))
        let known = await manager.known.map(\.id)
        XCTAssertEqual(known, [a, b], "the entry stays for a new pairing")
        XCTAssertFalse(
            store.sessions.contains { $0.ref.serverID == a }, "nothing cached is believed")
        XCTAssertEqual(host(store, b).state, .connected)
        // Pairing again under the new key restores it.
        try await manager.pair(
            invite: PairingInvite(
                serverID: a, spkiSha256: "ka2", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        store = await manager.store
        XCTAssertEqual(host(store, a).state, .connected)
        XCTAssertFalse(host(store, a).identityChanged)
        XCTAssertTrue(store.sessions.contains { $0.ref.serverID == a && !$0.stale })
    }

    func testAnInviteWithAnotherKeyNeverSendsTheCode() async throws {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        network.hosts = [addrA: fake]
        let credentials = MemoryCredentials()
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: credentials, deviceName: "t",
            factory: network.factory)
        do {
            try await manager.pair(
                invite: PairingInvite(
                    serverID: a, spkiSha256: "substituted", code: "ABCD2345",
                    addresses: ["10.0.0.1:43187"]))
            XCTFail("paired through a substituted certificate")
        } catch let error as RemoteError {
            guard case .pinMismatch = error else { return XCTFail("\(error)") }
        }
        XCTAssertEqual(fake.calls, [], "no request reached the host")
        XCTAssertTrue(credentials.isEmpty)
        await manager.rebuild()
        let store = await manager.store
        XCTAssertEqual(store.hosts.first?.state, .unpaired)
    }

    func testARevokedTokenAsksToPairAgain() async throws {
        let (manager, network, credentials) = try await twoHosts()
        network.hosts[addrA]!.set {
            $0.failure = .refused(status: 401, code: "unauthorized", message: "revoked")
        }
        await manager.refreshAll()
        let store = await manager.store
        XCTAssertEqual(host(store, a).state, .unpaired)
        XCTAssertFalse(host(store, a).identityChanged, "the key is the same; only the token is gone")
        XCTAssertNil(try credentials.token(for: a))
        XCTAssertEqual(try credentials.pin(for: a), "ka", "the pin stays")
        XCTAssertTrue(
            store.sessions.contains { $0.ref.serverID == a && $0.stale }, "cached, marked")
    }

    func testForgetRemovesTokenPinMemoryAndViews() async throws {
        let (manager, _, credentials) = try await twoHosts()
        await manager.forget(a)
        let known = await manager.known.map(\.id)
        XCTAssertEqual(known, [b])
        XCTAssertNil(try credentials.token(for: a))
        XCTAssertNil(try credentials.pin(for: a))
        let store = await manager.store
        XCTAssertEqual(store.hosts.map(\.id), [b])
        XCTAssertFalse(store.sessions.contains { $0.ref.serverID == a })
        XCTAssertNotNil(try credentials.token(for: b))
    }

    func testAChangedAddressReconnectsUnderTheSamePin() async throws {
        let (manager, network, _) = try await twoHosts()
        let moved = Address(host: "10.0.0.9", port: 43187)
        network.hosts[moved] = network.hosts.removeValue(forKey: addrA)
        await manager.refreshAll()
        var store = await manager.store
        XCTAssertEqual(host(store, a).state, .reconnecting(attempt: 1))
        await manager.connection(a)!.setAddresses([moved])
        await manager.refreshAll()
        store = await manager.store
        XCTAssertEqual(host(store, a).state, .connected)
        let last = network.opened.last { $0.0 == moved }!
        XCTAssertEqual(last.1, .expect(spkiSha256: "ka"), "the same pin at the new address")
        XCTAssertEqual(last.2, "tok-1", "the same token")
    }

    func testAnotherInstallationAtTheAddressIsRefused() async throws {
        let (manager, network, _) = try await twoHosts()
        // A's address now answers with A's key but another id, and so
        // another epoch.
        network.hosts[addrA]!.set {
            $0.serverID = self.b
            $0.epoch = "e9"
        }
        await manager.refreshAll()
        let store = await manager.store
        guard case .offline(let reason) = host(store, a).state else {
            return XCTFail("\(host(store, a).state)")
        }
        XCTAssertTrue(reason.contains("another installation"), reason)
    }

    func testTheUnifiedOrderIsRankThenActivityThenId() {
        func item(_ id: String, _ card: String, _ at: UInt64?, server: ServerID) -> UnifiedSession {
            UnifiedSession(
                ref: SessionRef(serverID: server, recordID: id),
                session: SessionView(id: id, project: "p", name: id, card: card, lastStopAtMs: at),
                serverName: server.short, stale: false)
        }
        let sorted = UnifiedOrder.sort([
            item("z", "idle", 50, server: a),
            item("b", "waiting on you", 10, server: b),
            item("a", "waiting on you", 10, server: a),
            item("c", "working", 99, server: b),
            item("d", "waiting on you", 20, server: a),
        ])
        XCTAssertEqual(sorted.map(\.ref.recordID), ["d", "a", "b", "c", "z"])
        var filters = Filters()
        filters.servers = [b]
        XCTAssertEqual(sorted.filter(filters.admits).map(\.ref.recordID), ["b", "c"])
        filters.cards = ["working"]
        XCTAssertEqual(sorted.filter(filters.admits).map(\.ref.recordID), ["c"])
    }

    func testTheFileStoresRoundTripAndKeepCredentialsOwnerOnly() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let registry = FileRegistry(url: dir.appendingPathComponent("servers.json"))
        let server = KnownServer(
            id: a, name: "Desk", addresses: [addrA], pairedAt: Date(), scope: .read,
            needsPairing: false)
        try registry.save([server])
        XCTAssertEqual(try registry.load().map(\.id), [a])
        XCTAssertEqual(try registry.load()[0].scope, .read)
        let credentials = FileCredentials(url: dir.appendingPathComponent("credentials.json"))
        try credentials.setToken("t", for: a)
        try credentials.setPin("k", for: a)
        XCTAssertEqual(try credentials.token(for: a), "t")
        let attrs = try FileManager.default.attributesOfItem(atPath: credentials.url.path)
        XCTAssertEqual(attrs[.posixPermissions] as? Int, 0o600)
        try credentials.forget(a)
        XCTAssertNil(try credentials.pin(for: a))
        try? FileManager.default.removeItem(at: dir)
    }
}
