import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// Commands under the control scope (§4.5): the op is the key, a
/// timeout is followed by a status query and never a blind resend,
/// and a refusal names the host's reason without dropping the pairing.
final class CommandTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let addrA = Address(host: "10.0.0.1", port: 43187)

    func host() async throws -> (ConnectionManager, FakeHost, ServerConnection) {
        let network = FakeNetwork()
        let fake = FakeHost(serverID: a, name: "Desk", spki: "ka")
        fake.set {
            $0.spaces = [SpaceView(id: "s", name: "Space")]
            $0.projects = [ProjectView(id: "p", name: "P", space: "s")]
            $0.sessions = ["p": [SessionView(id: "r1", project: "p", name: "one")]]
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

    func sends(_ fake: FakeHost) -> [String] { fake.calls.filter { $0.hasPrefix("send:") } }

    func testASendCarriesTheTextAndAnOpOfItsOwn() async throws {
        let (manager, fake, _) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        let first = await manager.send(ref, text: "hello")
        XCTAssertEqual(first, .accepted)
        XCTAssertEqual(sends(fake), ["send:r1:hello"])
        let second = await manager.send(ref, text: "again")
        XCTAssertEqual(second, .accepted)
        XCTAssertEqual(fake.ops.count, 2)
        XCTAssertNotEqual(fake.ops[0], fake.ops[1], "every command has its own op")
        let resumed = await manager.resume(ref)
        XCTAssertEqual(resumed, .accepted)
        let trusted = await manager.trust(ref)
        XCTAssertEqual(trusted, .accepted)
        XCTAssertEqual(fake.calls.filter { $0 == "resume:r1" || $0 == "trust:r1" }.count, 2)
    }

    func testNoAnswerIsFollowedByTheOpsStatusNotAResend() async throws {
        let (_, fake, connection) = try await host()
        // Done on the host: the reply is taken from the status.
        fake.set {
            $0.commandSilences = 1
            $0.opStatuses["op-done"] = .done(failed: nil)
        }
        let done = await connection.command("r1", .send, text: "x", op: "op-done")
        XCTAssertEqual(done, .accepted)
        XCTAssertEqual(sends(fake).count, 1, "no resend: \(fake.calls)")
        XCTAssertTrue(fake.calls.contains("op:op-done"))

        // Interrupted: never resubmitted.
        fake.set {
            $0.commandSilences = 1
            $0.opStatuses["op-int"] = .interrupted
        }
        let interrupted = await connection.command("r1", .send, text: "y", op: "op-int")
        XCTAssertEqual(interrupted, .interrupted)
        XCTAssertEqual(sends(fake).count, 2)

        // Unknown: the host never saw it, so it goes once more with the same op.
        fake.set { $0.commandSilences = 1 }
        let unknown = await connection.command("r1", .send, text: "z", op: "op-new")
        XCTAssertEqual(unknown, .accepted)
        XCTAssertEqual(sends(fake).count, 4, "sent twice: \(fake.calls)")
        XCTAssertEqual(fake.ops.suffix(2).map { $0 }, ["op-new", "op-new"])

        // Unknown twice: given up, and still not a third send.
        fake.set { $0.commandSilences = 2 }
        let gone = await connection.command("r1", .send, text: "w", op: "op-gone")
        guard case .unanswered = gone else { return XCTFail("\(gone)") }
        XCTAssertEqual(sends(fake).count, 6)

        // Done but failed on the host: the host's reason.
        fake.set {
            $0.commandSilences = 1
            $0.opStatuses["op-fail"] = .done(failed: "not running")
        }
        let failed = await connection.command("r1", .send, text: "v", op: "op-fail")
        XCTAssertEqual(failed, .refused("not running"))
        let state = await connection.state
        XCTAssertEqual(state, .connected, "a timeout on a command does not drop the connection")
    }

    func testARefusalNamesTheReasonAndAScopeRefusalKeepsThePairing() async throws {
        let (manager, fake, connection) = try await host()
        let ref = SessionRef(serverID: a, recordID: "r1")
        fake.set { $0.commandRefusal = .refused(status: 403, code: "scope", message: "this device reads only") }
        let refused = await manager.send(ref, text: "hello")
        XCTAssertEqual(refused, .refused("this device reads only"))
        let state = await connection.state
        XCTAssertEqual(state, .connected)
        let server = await connection.server
        XCTAssertFalse(server.needsPairing)
        fake.set { $0.commandRefusal = .refused(status: 400, code: "failed", message: "not running") }
        let notRunning = await manager.send(ref, text: "hello")
        XCTAssertEqual(notRunning, .refused("not running"))
        fake.set { $0.commandRefusal = .refused(status: 409, code: "interrupted", message: "interrupted: started") }
        let interrupted = await manager.send(ref, text: "hello")
        XCTAssertEqual(interrupted, .interrupted)
        XCTAssertEqual(sends(fake).count, 3, "a refusal is never retried")
    }
}
