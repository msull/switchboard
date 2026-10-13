import Foundation
import XCTest

@testable import SwitchboardTransport

/// The pin fixture holds a host certificate and the hash the Rust side
/// computed over its SPKI; the client must compute the same, and
/// nothing weaker is accepted.
final class PinningTests: XCTestCase {
    struct PinFixture: Decodable {
        let certificate: String
        let spki_sha256: String
    }

    func testTheClientHashesTheSPKIAsTheHostDoes() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Fixtures/pin.json")
        let fixture = try JSONDecoder().decode(PinFixture.self, from: Data(contentsOf: url))
        let der = try XCTUnwrap(Data(base64Encoded: fixture.certificate))
        XCTAssertEqual(SPKI.sha256Hex(ofCertificateDER: der), fixture.spki_sha256)
        XCTAssertNil(SPKI.sha256Hex(ofCertificateDER: Data([0x30, 0x00])), "not a certificate")
    }

    func testAddressesParse() {
        XCTAssertEqual(Address("192.168.1.9:43187"), Address(host: "192.168.1.9", port: 43187))
        XCTAssertEqual(Address("[fe80::1]:1"), Address(host: "fe80::1", port: 1))
        XCTAssertEqual(Address(host: "fe80::1", port: 1).description, "[fe80::1]:1")
        XCTAssertNil(Address("nohost"))
        XCTAssertNil(Address(":43187"))
        XCTAssertEqual(
            Address(host: "10.0.0.2", port: 43187).baseURL.absoluteString,
            "https://10.0.0.2:43187/api/v1/")
    }

    func testAPairingNeedsAPin() async {
        let client = HostClient(address: Address(host: "127.0.0.1", port: 1), pin: .observe)
        do {
            _ = try await client.pair(.init(code: "X", deviceName: "t", spkiSha256: "y"))
            XCTFail("sent a code without a pin")
        } catch {
            XCTAssertTrue("\(error)".contains("pinned"))
        }
    }

    /// A request cut short is not a key change: only a certificate the
    /// delegate refused under another key drops the pairing.
    func testOnlyACertificateUnderAnotherKeyIsAMismatch() {
        let cancelled = NSError(domain: NSURLErrorDomain, code: NSURLErrorCancelled)
        let cut = HostClient.classify(cancelled, policy: .expect(spkiSha256: "ka"), mismatch: nil)
        guard case .transport = cut else { return XCTFail("\(cut)") }
        let untrusted = NSError(domain: NSURLErrorDomain, code: NSURLErrorServerCertificateUntrusted)
        let refused = HostClient.classify(untrusted, policy: .expect(spkiSha256: "ka"), mismatch: "kb")
        XCTAssertEqual("\(refused)", "the host's key kb is not the pinned ka")
        let observed = HostClient.classify(cancelled, policy: .observe, mismatch: "kb")
        guard case .transport = observed else { return XCTFail("\(observed)") }
    }
}
