import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// Subscribe-then-snapshot (§4.4) on the client: buffered events are
/// reconciled against each snapshot's revision, a live event refetches
/// only past the held revision, `resync_required` reconnects, and a
/// new epoch starts over.
final class EventTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let addrA = Address(host: "10.0.0.1", port: 43187)

    func host() async throws -> (ConnectionManager, FakeHost) {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        fake.set {
            $0.spaces = [SpaceView(id: "s", name: "Space")]
            $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
            $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "one")]]
            $0.sets = ["s": [SetView(id: "w1", name: "Desk set", space: "s")]]
        }
        network.hosts = [addrA: fake]
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: MemoryCredentials(), deviceName: "t",
            factory: network.factory)
        try await manager.pair(
            invite: PairingInvite(
                serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        await manager.connectAll()
        return (manager, fake)
    }

    /// Wait for the manager's store to satisfy `check`, rebuilding from
    /// the connections meanwhile.
    func settle(_ manager: ConnectionManager, _ check: (UnifiedStore) -> Bool) async -> UnifiedStore {
        for _ in 0..<200 {
            await manager.rebuild()
            let store = await manager.store
            if check(store) { return store }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        return await manager.store
    }

    func calls(_ fake: FakeHost, _ name: String) -> Int {
        fake.calls.filter { $0 == name }.count
    }

    func testAConnectionSubscribesBeforeItSnapshotsAndIsConnected() async throws {
        let (manager, fake) = try await host()
        let store = await manager.store
        XCTAssertEqual(store.hosts.first?.state, .connected)
        XCTAssertEqual(store.sessions.map(\.session.name), ["one"])
        XCTAssertEqual(fake.subscriberCount, 1)
        let order = fake.calls
        let events = try XCTUnwrap(order.firstIndex(of: "events"))
        let projects = try XCTUnwrap(order.firstIndex(of: "projects"))
        XCTAssertLessThan(events, projects, "subscribed before the first snapshot: \(order)")
    }

    func testALiveChangePastTheHeldRevisionRefetchesOnlyThatResource() async throws {
        let (manager, fake) = try await host()
        let sessionsBefore = calls(fake, "sessions")
        fake.set {
            $0.revision = 2
            $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "renamed")]]
        }
        fake.emit(.changed, ResourceRef(kind: "session", id: "r1"))
        let store = await settle(manager) { $0.sessions.first?.session.name == "renamed" }
        XCTAssertEqual(store.sessions.map(\.session.name), ["renamed"])
        XCTAssertEqual(calls(fake, "session"), 1, "the one session was read")
        XCTAssertEqual(calls(fake, "sessions"), sessionsBefore, "no collection refetched")
        XCTAssertEqual(store.hosts.first?.state, .connected)
    }

    func testAnEventAtOrBelowTheSnapshotRevisionCausesNoRefetch() async throws {
        let (manager, fake) = try await host()
        let before = fake.calls.count
        fake.emit(.changed, ResourceRef(kind: "session", id: "r1"), revision: 1)
        fake.emit(.changed, ResourceRef(kind: "sessions", id: "p"), revision: 0)
        try await Task.sleep(nanoseconds: 50_000_000)
        await manager.rebuild()
        XCTAssertEqual(fake.calls.count, before, "nothing refetched: \(fake.calls)")
    }

    func testAMutationBetweenHelloAndSnapshotEndsOnTheHostsState() async throws {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        fake.set {
            $0.spaces = [SpaceView(id: "s", name: "Space")]
            $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
            $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "stale")]]
            // The sessions snapshot is built at revision 1, then the host
            // moves to revision 2 and tells every subscriber before the
            // snapshot reaches the client.
            $0.afterSessionsBuilt = { [unowned fake] in
                fake.set {
                    $0.revision = 2
                    $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "fresh")]]
                }
                fake.emit(.changed, ResourceRef(kind: "session", id: "r1"))
            }
        }
        network.hosts = [addrA: fake]
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: MemoryCredentials(), deviceName: "t",
            factory: network.factory)
        try await manager.pair(
            invite: PairingInvite(
                serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        await manager.connectAll()
        let store = await settle(manager) { $0.sessions.first?.session.name == "fresh" }
        XCTAssertEqual(store.sessions.map(\.session.name), ["fresh"], "the buffered event won")
        XCTAssertEqual(store.hosts.first?.state, .connected)
    }

    func testARemovedSessionIsDroppedAndACollectionChangeRefetchesIt() async throws {
        let (manager, fake) = try await host()
        fake.set {
            $0.revision = 2
            $0.sessions = ["p": [
                SessionView(id: "r1", project: "p", name: "one"),
                SessionView(id: "r2", project: "p", name: "two"),
            ]]
        }
        fake.emit(.changed, ResourceRef(kind: "sessions", id: "p"))
        var store = await settle(manager) { $0.sessions.count == 2 }
        XCTAssertEqual(store.sessions.map(\.session.name).sorted(), ["one", "two"])
        fake.set {
            $0.revision = 3
            $0.sessions = ["p": [SessionView(id: "r2", project: "p", name: "two")]]
        }
        fake.emit(.removed, ResourceRef(kind: "session", id: "r1"))
        store = await settle(manager) { $0.sessions.count == 1 }
        XCTAssertEqual(store.sessions.map(\.session.name), ["two"])
    }

    func testResyncRequiredReconnectsAndANewEpochRefetchesEverything() async throws {
        let (manager, fake) = try await host()
        let infos = calls(fake, "info")
        fake.set {
            $0.epoch = "e2"
            $0.revision = 1
            $0.sessions = ["p": [SessionView(id: "r9", project: "p", name: "after restart")]]
        }
        fake.emit(.resyncRequired)
        fake.closeSockets()
        var store = await settle(manager) { !$0.hosts[0].state.isConnected }
        XCTAssertNotEqual(store.hosts.first?.state, .connected)
        await manager.retryOffline()
        store = await settle(manager) { $0.hosts.first?.state == .connected }
        XCTAssertEqual(store.hosts.first?.state, .connected)
        XCTAssertEqual(calls(fake, "info"), infos + 1, "reconnected through /info")
        XCTAssertEqual(store.sessions.map(\.session.name), ["after restart"], "the old epoch's items are gone")
        let resyncs = await manager.connection(a)!.resyncs
        XCTAssertEqual(resyncs, 1)
    }

    /// With no snapshot (a host restart dropped it and the refetch
    /// failed), a frame for one session must not seed the list with
    /// that session alone.
    func testAFrameWithNoSnapshotFetchesEverythingInstead() async throws {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        fake.set {
            $0.spaces = [SpaceView(id: "s", name: "Space")]
            $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
            $0.sessions = [
                "p": [
                    SessionView(id: "r1", project: "p", name: "one"),
                    SessionView(id: "r2", project: "p", name: "two"),
                ]
            ]
        }
        network.hosts = [addrA: fake]
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: MemoryCredentials(), deviceName: "t",
            factory: network.factory)
        try await manager.pair(
            invite: PairingInvite(
                serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        let found = await manager.connection(a)
        let connection = try XCTUnwrap(found)
        // The host restarted and its refetch failed: no snapshot stands.
        fake.set {
            $0.epoch = "e2"
            $0.failure = .transport("timed out")
        }
        fake.emit(.changed, ResourceRef(kind: "session", id: "r1"), revision: 2)
        _ = await settle(manager) { !$0.hosts[0].state.isConnected }
        fake.set { $0.failure = nil }
        let before = await connection.snapshot
        XCTAssertNil(before, "the failed refetch left nothing")
        let singles = calls(fake, "session")
        try await connection.apply(
            EventFrame(
                kind: .changed, serverID: a, epoch: "e2", revision: 3,
                resource: ResourceRef(kind: "session", id: "r1")),
            api: fake)
        let after = await connection.snapshot
        let snapshot = try XCTUnwrap(after)
        XCTAssertEqual(snapshot.epoch, "e2", "the host's epoch, never a placeholder")
        XCTAssertEqual(snapshot.sessions.map(\.name).sorted(), ["one", "two"])
        XCTAssertEqual(calls(fake, "session"), singles, "no single fetch stood in for the list")
    }

    /// A connect while connected replaces the subscription. The old
    /// pump, still inside a refetch when it was replaced, finishes
    /// later and finds its socket gone; that must not take the new
    /// subscription down.
    func testAReplacedSubscriptionsEndIsIgnored() async throws {
        let (manager, fake) = try await host()
        let found = await manager.connection(a)
        let connection = try XCTUnwrap(found)
        fake.set { $0.sessionDelay = 0.3 }
        fake.emit(.changed, ResourceRef(kind: "session", id: "r1"), revision: 2)
        try await Task.sleep(nanoseconds: 50_000_000)
        await connection.connect()
        // The old pump's refetch returns and its loop ends after this.
        for _ in 0..<40 {
            try await Task.sleep(nanoseconds: 10_000_000)
            await manager.rebuild()
        }
        let store = await manager.store
        XCTAssertEqual(store.hosts.first?.state, .connected)
        XCTAssertEqual(fake.subscriberCount, 1)
        let error = await connection.lastError
        XCTAssertNil(error)
    }

    func testAClosedSocketIsReconnectedByTheLoop() async throws {
        let (manager, fake) = try await host()
        fake.closeSockets()
        let store = await settle(manager) { !$0.hosts[0].state.isConnected }
        guard case .reconnecting(let attempt) = store.hosts[0].state else {
            return XCTFail("\(store.hosts[0].state)")
        }
        XCTAssertEqual(attempt, 1)
        XCTAssertEqual(fake.subscriberCount, 0)
        // The backoff for one failure is one to one and a half seconds.
        try await Task.sleep(nanoseconds: 1_600_000_000)
        await manager.retryOffline()
        let back = await settle(manager) { $0.hosts.first?.state == .connected }
        XCTAssertEqual(back.hosts.first?.state, .connected)
        XCTAssertEqual(fake.subscriberCount, 1)
    }
}
