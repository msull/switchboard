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

    func testAConversationWatchReadsWhatIsHeldAndFollowsChanged() async throws {
        let (_, fake, connection) = try await host()
        fake.set {
            $0.lastResponses = ["r1": LastResponseBody(resumeID: "h1", n: 1, total: 1, text: "pong")]
            $0.conversations = [
                "r1": ConversationBody(
                    resumeID: "h1", total: 1, from: 1, next: nil,
                    turns: [TurnView(n: 1, user: "ping", finalText: "pong")])
            ]
        }
        await connection.watch(.conversation(session: "r1"))
        await connection.readLastResponse("r1")
        await connection.readConversation("r1", from: nil, limit: 20)
        XCTAssertEqual(fake.calls.filter { $0 == "watch:r1" }.count, 1, "\(fake.calls)")
        let held = await connection.lastResponses["r1"]
        XCTAssertEqual(held?.text, "pong")
        let turns = await connection.conversations["r1"]?.turns.map(\.n)
        XCTAssertEqual(turns, [1])

        fake.set {
            $0.revision = 9
            $0.lastResponses = ["r1": LastResponseBody(resumeID: "h1", n: 2, total: 2, text: "pong again")]
            $0.conversations["r1"] = ConversationBody(
                resumeID: "h1", total: 2, from: 1, next: nil,
                turns: [
                    TurnView(n: 1, user: "ping", finalText: "pong"),
                    TurnView(n: 2, user: "again", finalText: "pong again"),
                ])
        }
        fake.emit(.changed, ResourceRef(kind: "conversation", id: "r1"))
        await settle(connection) { await $0.lastResponses["r1"]?.n == 2 }
        let after = await connection.lastResponses["r1"]
        XCTAssertEqual(after?.text, "pong again")
        await settle(connection) { await $0.conversations["r1"]?.turns.count == 2 }
        let merged = await connection.conversations["r1"]?.turns.map(\.n)
        XCTAssertEqual(merged, [1, 2], "the page from the last held turn replaced it and added the next")
        XCTAssertEqual(fake.calls.filter { $0 == "conversation" }.count, 2, "one page per change")

        await connection.unwatch(.conversation(session: "r1"))
        XCTAssertEqual(fake.calls.filter { $0 == "unwatch:r1" }.count, 1)
    }

    func testAChangeIsFollowedToTheEndAcrossSeveralPages() async throws {
        let (_, fake, connection) = try await host()
        fake.set {
            $0.conversations = [
                "r1": ConversationBody(
                    resumeID: "h1", total: 1, from: 1, next: nil,
                    turns: [TurnView(n: 1, user: "p1", finalText: "a1")])
            ]
        }
        await connection.watch(.conversation(session: "r1"))
        await connection.readConversation("r1", from: nil, limit: 20)
        // Forty-four turns arrive while the window was away: the fake
        // pages twenty at a time, so following the change takes three.
        fake.set {
            $0.revision = 9
            $0.conversations["r1"] = ConversationBody(
                resumeID: "h1", total: 45, from: 1, next: nil,
                turns: (1...45).map { TurnView(n: $0, user: "p\($0)", finalText: "a\($0)") })
        }
        fake.emit(.changed, ResourceRef(kind: "conversation", id: "r1"))
        await settle(connection) { await $0.conversations["r1"]?.lastHeld == 45 }
        let held = await connection.conversations["r1"]?.turns.map(\.n)
        XCTAssertEqual(held, Array(1...45), "no gap and the end reached")
        XCTAssertEqual(fake.calls.filter { $0 == "conversation" }.count, 4, "\(fake.calls)")
    }

    func testOpeningATurnReadsItWholeInPlaceOfThePagesCopy() async throws {
        let (manager, fake, _) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        let head = ActivityView(
            kind: .tool, line: "Read x", detail: ToolDetailView(name: "Read", input: "{}", result: "# Swi"),
            more: true)
        let whole = ActivityView(
            kind: .tool, line: "Read x", detail: ToolDetailView(name: "Read", input: "{}", result: "# Switchboard"))
        fake.set {
            $0.conversations = [
                "r1": ConversationBody(
                    resumeID: "h1", total: 1, from: 1, next: nil,
                    turns: [TurnView(n: 1, user: "p1", finalText: "a1", activity: [head])])
            ]
            $0.turns = ["r1": [TurnView(n: 1, user: "p1", finalText: "a1", activity: [whole])]]
        }
        await manager.readConversation(ref, from: nil, limit: 20)
        let page = await manager.conversation(for: ref)
        XCTAssertEqual(page?.turns[0].activity[0].more, true)
        await manager.readTurn(ref, n: 1)
        let filled = await manager.conversation(for: ref)
        XCTAssertEqual(filled?.turns[0].activity[0].more, false)
        XCTAssertEqual(filled?.turns[0].activity[0].detail?.result, "# Switchboard")
        XCTAssertEqual(fake.calls.filter { $0 == "turn:1" }.count, 1)
        await manager.readTurn(ref, n: 9)
        let same = await manager.conversation(for: ref)
        XCTAssertEqual(same, filled, "a turn past the end changes nothing")
    }

    func testEarlierTurnsArriveAsThePageEndingBeforeTheFirstHeld() async throws {
        let (manager, fake, _) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        fake.set {
            $0.conversations = [
                "r1": ConversationBody(
                    resumeID: "h1", total: 5, from: 1, next: nil,
                    turns: (1...5).map { TurnView(n: $0, user: "p\($0)", finalText: "a\($0)") })
            ]
        }
        await manager.readConversation(ref, from: nil, limit: 2)
        let tail = await manager.conversation(for: ref)
        XCTAssertEqual(tail?.turns.map(\.n), [4, 5], "the tail first")
        await manager.readConversation(ref, before: 4, limit: 2)
        let more = await manager.conversation(for: ref)
        XCTAssertEqual(more?.turns.map(\.n), [2, 3, 4, 5], "the page before joined in front")
        XCTAssertTrue(more?.hasEarlier == true)
        await manager.readConversation(ref, before: 2, limit: 2)
        let all = await manager.conversation(for: ref)
        XCTAssertEqual(all?.turns.map(\.n), [1, 2, 3, 4, 5])
        XCTAssertFalse(all?.hasEarlier == true)
    }

    func testARunOutputWatchNamesTheRunAndFollowsChanged() async throws {
        let (_, fake, connection) = try await host()
        fake.set {
            $0.runs = ["r1": [RunView(n: 1, startedMs: 1), RunView(n: 2, startedMs: 2)]]
            $0.runOutputs = [RunKey(session: "r1", n: 2): RunOutputBody(n: 2, text: "building")]
        }
        await connection.readRuns("r1")
        let runs = await connection.runs["r1"]?.map(\.n)
        XCTAssertEqual(runs, [1, 2])
        await connection.watch(.runOutput(session: "r1", run: 2))
        await connection.readRunOutput("r1", run: 2, lines: nil)
        XCTAssertEqual(fake.calls.filter { $0 == "watch:r1/2" }.count, 1, "\(fake.calls)")
        let text = await connection.runOutputs[RunKey(session: "r1", n: 2)]?.text
        XCTAssertEqual(text, "building")
        fake.set {
            $0.revision = 9
            $0.runOutputs[RunKey(session: "r1", n: 2)] = RunOutputBody(n: 2, text: "building\ndone")
            $0.runs["r1"]?[1] = RunView(n: 2, startedMs: 2, endedMs: 3, exit: 0)
        }
        fake.emit(.changed, ResourceRef(kind: "run_output", id: "r1/2"))
        fake.emit(.changed, ResourceRef(kind: "runs", id: "r1"))
        await settle(connection) { await $0.runOutputs[RunKey(session: "r1", n: 2)]?.text == "building\ndone" }
        await settle(connection) { await $0.runs["r1"]?[1].exit == 0 }
        let exit = await connection.runs["r1"]?[1].exit
        XCTAssertEqual(exit, 0, "the runs collection refetched on its event")
    }

    /// A pinned file is content like a screen: the first holder watches
    /// and reads, `changed {file}` re-reads, and a refusal clears it.
    func testAPinnedFileIsWatchedReadAndFollowsChanged() async throws {
        let (manager, fake, connection) = try await host()
        fake.set { $0.files["p/docs/plan.md"] = FileBody(path: "docs/plan.md", text: "# Plan", size: 6) }
        let ref = FileRef(serverID: a, projectID: "p", path: "docs/plan.md")
        await manager.watch(file: ref)
        let first = await manager.file(for: ref)
        XCTAssertEqual(first?.text, "# Plan")
        let keys = await connection.watchedKeys
        XCTAssertEqual(keys.map { $0.wire.kind + ":" + $0.wire.id }, ["file:p/docs/plan.md"])
        fake.set {
            $0.files["p/docs/plan.md"] = FileBody(path: "docs/plan.md", text: "# Plan\nmore", size: 11)
            $0.revision = 2
        }
        fake.emit(.changed, ResourceRef(kind: "file", id: "p/docs/plan.md"), revision: 2)
        await settle(connection) { await $0.files["p/docs/plan.md"]?.text == "# Plan\nmore" }
        let second = await manager.file(for: ref)
        XCTAssertEqual(second?.text, "# Plan\nmore")
        fake.set { $0.files = [:] }
        await connection.readFile(project: "p", path: "docs/plan.md")
        let gone = await manager.file(for: ref)
        XCTAssertNil(gone, "an unpinned file is cleared")
        await manager.unwatch(file: ref)
        let after = await connection.watchedKeys
        XCTAssertTrue(after.isEmpty)
    }
}
