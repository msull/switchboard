import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// Watched content on the client (§4.4): a watch is sent at the first
/// holder and `unwatch` at the last, the screen follows `changed` past
/// the held revision, and a reconnect resends every watch before it
/// snapshots.
final class WatchTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let addrA = Address(host: "10.0.0.1", port: 43187)

    func host() async throws -> (ConnectionManager, FakeHost, ServerConnection) {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        fake.set {
            $0.spaces = [SpaceView(id: "s", name: "Space")]
            $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
            $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "one")]]
            $0.screens = ["r1": ScreenBody(text: "$ cargo test")]
        }
        network.hosts = [addrA: fake]
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: MemoryCredentials(), deviceName: "t",
            factory: network.factory)
        try await manager.pair(
            invite: PairingInvite(
                serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        await manager.connectAll()
        let found = await manager.connection(a)
        let connection = try XCTUnwrap(found)
        return (manager, fake, connection)
    }

    func calls(_ fake: FakeHost, _ name: String) -> Int {
        fake.calls.filter { $0 == name }.count
    }

    /// Wait until `check` holds on the connection.
    func settle(_ connection: ServerConnection, _ check: (ServerConnection) async -> Bool) async {
        for _ in 0..<200 {
            if await check(connection) { return }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    func testTheFirstHolderSendsWatchReadsOnceAndTheScreenFollowsChanged() async throws {
        let (manager, fake, connection) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        await manager.watch(screen: ref)
        XCTAssertEqual(calls(fake, "watch:r1"), 1)
        XCTAssertEqual(calls(fake, "screen"), 1, "read once after the watch")
        let first = await manager.screen(for: ref)
        XCTAssertEqual(first?.text, "$ cargo test")

        await manager.watch(screen: ref)
        XCTAssertEqual(calls(fake, "watch:r1"), 1, "a second holder sends nothing")
        XCTAssertEqual(calls(fake, "screen"), 1)

        fake.set {
            $0.revision = 2
            $0.screens = ["r1": ScreenBody(text: "$ cargo test\ntest result: ok")]
        }
        fake.emit(.changed, ResourceRef(kind: "screen", id: "r1"))
        await settle(connection) { await $0.screens["r1"]?.text.contains("ok") == true }
        let next = await manager.screen(for: ref)
        XCTAssertEqual(next?.text, "$ cargo test\ntest result: ok")
        XCTAssertEqual(calls(fake, "screen"), 2)

        // An event at the held revision is already in what was read.
        fake.emit(.changed, ResourceRef(kind: "screen", id: "r1"), revision: 2)
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(calls(fake, "screen"), 2, "nothing refetched")
    }

    func testUnwatchGoesOutAtTheLastHolderOnly() async throws {
        let (manager, fake, connection) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        await manager.watch(screen: ref)
        await manager.watch(screen: ref)
        await manager.unwatch(screen: ref)
        XCTAssertEqual(calls(fake, "unwatch:r1"), 0, "one of two holders closed")
        let still = await connection.watchedScreens
        XCTAssertEqual(still, ["r1"])
        await manager.unwatch(screen: ref)
        XCTAssertEqual(calls(fake, "unwatch:r1"), 1)
        let none = await connection.watchedScreens
        XCTAssertEqual(none, [])
        let kept = await manager.screen(for: ref)
        XCTAssertNotNil(kept, "the last text stays until a removal or a new epoch")
    }

    func testAReconnectResendsTheWatchBeforeItSnapshots() async throws {
        let (manager, fake, connection) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        await manager.watch(screen: ref)
        fake.closeSockets()
        await settle(connection) { await !$0.state.isConnected }
        let before = fake.calls.count
        // The backoff for one failure is one to one and a half seconds.
        try await Task.sleep(nanoseconds: 1_600_000_000)
        await manager.retryOffline()
        await settle(connection) { await $0.state.isConnected }
        let after = Array(fake.calls[before...])
        let watch = try XCTUnwrap(after.firstIndex(of: "watch:r1"), "\(after)")
        let projects = try XCTUnwrap(after.firstIndex(of: "projects"), "\(after)")
        XCTAssertLessThan(watch, projects, "the watch went before the first snapshot: \(after)")
        XCTAssertTrue(after.contains("screen"), "and the screen was read again: \(after)")
    }

    func testARefusedScreenClearsWhatWasHeldAndKeepsTheConnection() async throws {
        let (manager, fake, connection) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        await manager.watch(screen: ref)
        fake.set {
            $0.revision = 2
            $0.screens = [:]
        }
        fake.emit(.changed, ResourceRef(kind: "screen", id: "r1"))
        await settle(connection) { await $0.screens["r1"] == nil }
        let gone = await manager.screen(for: ref)
        XCTAssertNil(gone)
        let state = await connection.state
        XCTAssertEqual(state, .connected)
    }
}
