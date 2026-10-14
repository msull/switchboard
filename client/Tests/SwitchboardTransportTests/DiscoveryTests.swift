import Foundation
import Network
import XCTest

@testable import SwitchboardTransport

final class DiscoveryTests: XCTestCase {
    /// A connect to an address that never answers sits in the system's
    /// waiting state for as long as the path stays the same; the
    /// resolver must give it up on its own, or the retry loop awaiting
    /// it stops for good.
    func testAResolveThatGetsNoAnswerGivesUp() async {
        let started = Date()
        do {
            _ = try await Discovery.resolve(
                .hostPort(host: "10.255.255.1", port: 9), timeout: 1)
            XCTFail("resolved a black hole")
        } catch {
            XCTAssertLessThan(Date().timeIntervalSince(started), 5)
        }
    }
}
