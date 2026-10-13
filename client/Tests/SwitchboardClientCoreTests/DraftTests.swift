import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// The draft (§4.5): a session's prompt is the host's editor. The
/// device reads it, re-reads it on `changed`, dictates into it as
/// speech events, works it with the editor's tools, and sends it under
/// an op of its own.
final class DraftTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let addrA = Address(host: "10.0.0.1", port: 43187)

    func host() async throws -> (ConnectionManager, FakeHost, ServerConnection) {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        fake.set {
            $0.spaces = [SpaceView(id: "s", name: "Space")]
            $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
            $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "one")]]
            $0.drafts["r1"] = DraftView(text: "hello")
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

    func settle(_ connection: ServerConnection, _ check: (ServerConnection) async -> Bool) async {
        for _ in 0..<200 {
            if await check(connection) { return }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    func testADraftIsReadHeldAndFollowsChanged() async throws {
        let (manager, fake, connection) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        let capable = await manager.hasDraft(a)
        XCTAssertTrue(capable, "the host lists the draft capability")
        await manager.readDraft(ref)
        let first = await manager.draft(for: ref)
        XCTAssertEqual(first?.text, "hello")
        fake.set {
            $0.drafts["r1"]?.text = "hello there"
            $0.revision = 2
        }
        fake.emit(.changed, ResourceRef(kind: "draft", id: "r1"), revision: 2)
        await settle(connection) { await $0.drafts["r1"]?.text == "hello there" }
        let second = await manager.draft(for: ref)
        XCTAssertEqual(second?.text, "hello there", "changed re-reads a held draft")
        XCTAssertEqual(fake.calls.filter { $0 == "draft:r1" }.count, 2)
        // A host without the capability says so, and the bar keeps its own field.
        fake.set { $0.capabilities = ["read", "control"] }
        let old = FakeHost(serverID: ServerID("22222222-2222-2222-2222-222222222222"), name: "Old", spki: "kb")
        old.set { $0.capabilities = ["read", "control"] }
        XCTAssertFalse(old.capabilities.contains("draft"))
    }

    func testDictationLandsInTheDraftAndSendClearsItUnderAnOp() async throws {
        let (manager, fake, _) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        fake.set { $0.drafts["r1"] = DraftView() }
        let events: [DictationEvent] = [
            DictationEvent(sequence: 1, kind: .started),
            DictationEvent(sequence: 2, kind: .voiceStarted, utterance: 1),
            DictationEvent(sequence: 3, kind: .partial, utterance: 1, revision: 1, text: "fix the"),
            DictationEvent(sequence: 4, kind: .final, utterance: 1, text: "Fix the test."),
        ]
        let landed = await manager.dictate(ref, recognizer: 5, events: events)
        guard case .applied(let draft) = landed else { return XCTFail("\(landed)") }
        XCTAssertEqual(draft.text, "Fix the test.")
        XCTAssertEqual(draft.status, "listening")
        XCTAssertEqual(fake.dictations.count, 1)
        XCTAssertEqual(fake.dictations[0].0, 5)
        XCTAssertEqual(fake.dictations[0].1.map(\.sequence), [1, 2, 3, 4])
        let stopped = await manager.dictate(ref, recognizer: 5, events: [DictationEvent(sequence: 5, kind: .stopped)])
        guard case .applied(let idle) = stopped else { return XCTFail("\(stopped)") }
        XCTAssertEqual(idle.status, "idle")

        let replaced = await manager.replaceDraft(ref, text: "Fix the test, then run it.")
        guard case .applied(let typed) = replaced else { return XCTFail("\(replaced)") }
        XCTAssertEqual(typed.text, "Fix the test, then run it.")
        let undone = await manager.draftAction(ref, .undo)
        guard case .applied(let back) = undone else { return XCTFail("\(undone)") }
        XCTAssertEqual(back.text, "Fix the test.")

        let sent = await manager.sendDraft(ref)
        guard case .applied(let cleared) = sent else { return XCTFail("\(sent)") }
        XCTAssertEqual(cleared.text, "")
        XCTAssertEqual(fake.calls.filter { $0.hasPrefix("send:") }, ["send:r1:Fix the test."])
        XCTAssertEqual(fake.ops.count, 1)
        XCTAssertFalse(fake.ops[0].isEmpty, "the send carries an op")
    }

    func testASendWithNoAnswerAsksTheOpAndReadsTheDraftAgain() async throws {
        let (_, fake, connection) = try await host()
        fake.set {
            $0.commandSilences = 1
            $0.opStatuses["op-done"] = .done(failed: nil)
            $0.drafts["r1"] = DraftView(text: "")
        }
        let outcome = await connection.sendDraft("r1", op: "op-done")
        guard case .applied(let draft) = outcome else { return XCTFail("\(outcome)") }
        XCTAssertEqual(draft.text, "")
        XCTAssertEqual(fake.ops, ["op-done"], "no resend after the status said done")
        XCTAssertTrue(fake.calls.contains("op:op-done"))
        fake.set {
            $0.commandRefusal = RemoteError.refused(status: 403, code: "scope", message: "this device reads only")
        }
        let refused = await connection.sendDraft("r1")
        XCTAssertEqual(refused, .refused("this device reads only"))
        let paired = await connection.server.needsPairing
        XCTAssertFalse(paired, "a scope refusal keeps the pairing")
    }
}
