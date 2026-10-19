import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// A session opened in a project on a host from the device (§4.5):
/// offered only by a connected host under the control scope that
/// serves `create`, its projects by workspace, the record named by the
/// reply and listed at once, a lost reply settled by the op's status,
/// and a refusal worded by the host.
final class NewSessionTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let b = ServerID("22222222-2222-2222-2222-222222222222")
    let addrA = Address(host: "10.0.0.1", port: 43187)
    let addrB = Address(host: "10.0.0.2", port: 43187)

    /// Host A serves `create` under the control scope; host B is older
    /// and read only.
    func twoHosts() async throws -> (ConnectionManager, FakeHost, FakeHost) {
        let network = FakeNetwork()
        let hostA = FakeHost(serverID: a, name: "Desk", spki: "ka")
        let hostB = FakeHost(serverID: b, name: "Laptop", spki: "kb")
        hostA.set {
            $0.capabilities = ["read", "control", "draft", "scope", "create"]
            $0.scope = "control"
            $0.spaces = [
                SpaceView(id: "all", name: "All", view: true),
                SpaceView(id: "s1", name: "Desk work"),
                SpaceView(id: "s2", name: "Side"),
            ]
            $0.projects = [
                ProjectView(id: "p1", name: "Orchard", root: "/code/orchard", space: "s1"),
                ProjectView(id: "p2", name: "Notes", root: "/code/notes", space: "s2"),
                ProjectView(id: "p3", name: "Lost", root: "/code/lost", space: "gone"),
            ]
            $0.sessions = ["p1": [SessionView(id: "r1", project: "p1", name: "one")]]
        }
        hostB.set {
            $0.projects = [ProjectView(id: "q1", name: "Elsewhere", space: "t1")]
            $0.spaces = [SpaceView(id: "t1", name: "T")]
        }
        network.hosts = [addrA: hostA, addrB: hostB]
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: MemoryCredentials(), deviceName: "test",
            factory: network.factory)
        try await manager.load()
        try await manager.pair(
            invite: PairingInvite(serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        try await manager.pair(
            invite: PairingInvite(serverID: b, spkiSha256: "kb", code: "ABCD2345", addresses: ["10.0.0.2:43187"]))
        await manager.connectAll()
        return (manager, hostA, hostB)
    }

    func testOnlyAControlHostThatServesCreateOffersItWithItsProjectsByWorkspace() async throws {
        let (manager, _, _) = try await twoHosts()
        let hosts = await manager.createHosts
        XCTAssertEqual(hosts, [a])
        let found = await manager.catalog(for: a)
        let catalog = try XCTUnwrap(found)
        XCTAssertEqual(catalog.byWorkspace.map(\.name), ["Desk work", "Side", "Other"], "the global view left out")
        XCTAssertEqual(catalog.byWorkspace.map { $0.projects.map(\.id) }, [["p1"], ["p2"], ["p3"]])
        let none = await manager.catalog(for: ServerID("33333333-3333-3333-3333-333333333333"))
        XCTAssertNil(none)
    }

    func testASessionIsOpenedAsAskedAndListedBeforeTheAnswerReturns() async throws {
        let (manager, hostA, _) = try await twoHosts()
        let outcome = await manager.newSession(
            on: a, project: "p1", name: "Claude 2", kind: .claude, cwd: nil, prompt: "Look around.")
        XCTAssertEqual(outcome, .opened("n1"))
        XCTAssertEqual(hostA.created.count, 1)
        let (project, request) = try XCTUnwrap(hostA.created.first)
        XCTAssertEqual(project, "p1")
        XCTAssertEqual(request.name, "Claude 2")
        XCTAssertEqual(request.kind, .claude)
        XCTAssertNil(request.cwd)
        XCTAssertEqual(request.prompt, "Look around.")
        let store = await manager.store
        XCTAssertEqual(
            store.sessions.filter { $0.ref.serverID == a }.map(\.session.id).sorted(), ["n1", "r1"],
            "read again on the answer, not left to the next change")
        let shell = await manager.newSession(
            on: a, project: "p2", name: "Shell 1", kind: .shell, cwd: "src", prompt: nil)
        XCTAssertEqual(shell, .opened("n2"))
        XCTAssertEqual(hostA.created.last?.1.cwd, "src")
        XCTAssertEqual(hostA.ops.count, 2)
        XCTAssertNotEqual(hostA.ops[0], hostA.ops[1], "every creation has its own op")
        let elsewhere = await manager.newSession(
            on: ServerID("33333333-3333-3333-3333-333333333333"), project: "p", name: "x", kind: .shell,
            cwd: nil, prompt: nil)
        XCTAssertEqual(elsewhere, .unanswered("no host"))
    }

    func testALostReplyIsSettledByTheOpsStatusAndARefusalIsTheHostsReason() async throws {
        let (manager, hostA, _) = try await twoHosts()
        hostA.set { $0.commandSilences = 1 }
        let lost = await manager.newSession(
            on: a, project: "p1", name: "Claude 2", kind: .claude, cwd: nil, prompt: nil)
        XCTAssertEqual(lost, .opened("n1"), "the status names what the host made")
        XCTAssertEqual(hostA.created.count, 1, "never sent twice")
        XCTAssertEqual(hostA.ops.count, 1)
        hostA.set {
            $0.commandRefusal = RemoteError.refused(
                status: 400, code: "bad_request", message: "cwd is a directory of the project, by a path under its root")
        }
        let refused = await manager.newSession(
            on: a, project: "p1", name: "x", kind: .shell, cwd: "/etc", prompt: nil)
        XCTAssertEqual(refused, .refused("cwd is a directory of the project, by a path under its root"))
        XCTAssertEqual(hostA.created.count, 1)
        hostA.set {
            $0.commandRefusal = RemoteError.refused(
                status: 403, code: "scope", message: "this device reads only; raise its scope on the host")
        }
        let scope = await manager.newSession(
            on: a, project: "p1", name: "x", kind: .shell, cwd: nil, prompt: nil)
        XCTAssertEqual(scope, .refused("this device reads only; raise its scope on the host"))
        let state = await manager.connection(a)?.state
        XCTAssertEqual(state, .connected, "a scope refusal never drops the pairing")
    }
}
