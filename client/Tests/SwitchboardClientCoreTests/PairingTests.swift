import Foundation
import Network
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// Pairing without a camera (§5): a discovered host is looked up, the
/// key it presents is what the owner compares with the host's dialog,
/// and the typed code goes out pinned to that key.
final class PairingTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let addrA = Address(host: "10.0.0.1", port: 43187)

    func record(id: String) -> DiscoveredServer {
        DiscoveredServer(
            instance: "Desk (111111)", txt: ["id": id, "name": "Desk", "v": "1"],
            endpoint: .hostPort(host: "10.0.0.1", port: 43187))
    }

    func manager(_ network: FakeNetwork, _ credentials: MemoryCredentials) async throws -> ConnectionManager {
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: credentials, deviceName: "test",
            factory: network.factory)
        try await manager.load()
        let address = addrA
        await manager.setResolver { _ in address }
        return manager
    }

    func testADiscoveredHostPairsByItsCodeAgainstTheKeyItPresents() async throws {
        let network = FakeNetwork()
        let host = FakeHost(serverID: a, name: "Desk", spki: "ka")
        network.hosts = [addrA: host]
        let credentials = MemoryCredentials()
        let manager = try await manager(network, credentials)

        let lookup = try await manager.lookup(record(id: a.raw))
        XCTAssertEqual(lookup.address, addrA)
        XCTAssertEqual(lookup.info.spkiSha256, "ka", "the key the owner compares")
        XCTAssertEqual(network.opened.last?.1, .observe, "no trust before the owner looked")

        try await manager.pair(lookup, code: " abcd 2345 ")
        XCTAssertEqual(try credentials.pin(for: a), "ka")
        XCTAssertEqual(try credentials.token(for: a), "tok-1")
        XCTAssertEqual(network.opened.last?.1, .expect(spkiSha256: "ka"), "the code went out pinned")
        let known = await manager.known
        XCTAssertEqual(known.first?.needsPairing, false)
    }

    func testAWrongCodePinsNothing() async throws {
        let network = FakeNetwork()
        network.hosts = [addrA: FakeHost(serverID: a, name: "Desk", spki: "ka")]
        let credentials = MemoryCredentials()
        let manager = try await manager(network, credentials)
        let lookup = try await manager.lookup(record(id: a.raw))
        do {
            try await manager.pair(lookup, code: "WRONG123")
            XCTFail("paired with a wrong code")
        } catch {}
        XCTAssertNil(try credentials.token(for: a))
        let known = await manager.known
        XCTAssertEqual(known.first?.needsPairing, true)
    }

    func testARecordNamingAnotherHostIsRefusedAtLookup() async throws {
        let network = FakeNetwork()
        network.hosts = [addrA: FakeHost(serverID: a, name: "Desk", spki: "ka")]
        let manager = try await manager(network, MemoryCredentials())
        do {
            _ = try await manager.lookup(record(id: "22222222-2222-2222-2222-222222222222"))
            XCTFail("looked up a host under another's record")
        } catch {}
    }

    func testTheShortKeyReadsAsTheHostPrintsIt() {
        XCTAssertEqual(
            KeyFingerprint.short("0123456789abcdef0000ffff"), "0123 4567 89ab cdef")
        XCTAssertEqual(KeyFingerprint.short("abcdef"), "abcd ef")
    }
}
