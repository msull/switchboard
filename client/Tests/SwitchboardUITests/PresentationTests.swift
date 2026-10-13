import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardUI

/// The presentation model (§7.1): the modes per kind, the window
/// request, and where a window's session stands.
final class PresentationTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")

    func testTheModesOfferedFollowTheKindAndTheUndrawnOnesNameTheirMilestone() {
        XCTAssertEqual(ViewMode.offered(for: .claude), [.compact, .screen, .lastResponse, .conversation])
        XCTAssertEqual(ViewMode.offered(for: .shell), [.compact, .screen])
        XCTAssertEqual(ViewMode.offered(for: .service), [.compact, .screen, .runOutput])
        XCTAssertTrue(ViewMode.compact.drawn)
        XCTAssertTrue(ViewMode.screen.drawn)
        XCTAssertFalse(ViewMode.conversation.drawn)
        XCTAssertEqual(ViewMode.arrivesWith, "Milestone C")
    }

    func testTwoRequestsOnOneSessionAreTwoWindowsAndARequestRoundTrips() throws {
        let ref = SessionRef(serverID: a, recordID: "r1")
        let one = CardWindowRequest(ref: ref)
        let two = CardWindowRequest(ref: ref)
        XCTAssertNotEqual(one, two)
        let data = try JSONEncoder().encode(one)
        let back = try JSONDecoder().decode(CardWindowRequest.self, from: data)
        XCTAssertEqual(back, one)
        XCTAssertEqual(ViewMode(rawValue: "last_response"), .lastResponse)
    }

    func testStandingFollowsTheStoreAndTheRemovedSet() {
        let ref = SessionRef(serverID: a, recordID: "r1")
        let server = KnownServer(id: a, name: "Desk", addresses: [])
        var snapshot = HostSnapshot(epoch: "e", revision: 1)
        snapshot.sessions = [SessionView(id: "r1", project: "p", name: "one")]
        let live = UnifiedStore.build(from: [(server, .connected, snapshot, false, nil)])
        XCTAssertEqual(CardStanding.of(ref, in: live, removed: []), .live)
        let offline = UnifiedStore.build(from: [(server, .reconnecting(attempt: 1), snapshot, false, nil)])
        XCTAssertEqual(CardStanding.of(ref, in: offline, removed: []), .offline)
        XCTAssertFalse(CardStanding.offline.controlsEnabled)
        let empty = UnifiedStore.build(from: [(server, .connected, HostSnapshot(epoch: "e", revision: 2), false, nil)])
        XCTAssertEqual(CardStanding.of(ref, in: empty, removed: [ref]), .removed)
        XCTAssertEqual(CardStanding.of(ref, in: empty, removed: []), .unknown)
        XCTAssertNotNil(CardStanding.removed.banner)
    }
}

/// The presentation state on disk: a window's mode survives a relaunch
/// (visionOS restores the windows), the oldest entries go past the cap,
/// and nothing from a host is in the file.
final class PresentationStoreTests: XCTestCase {
    func testAModeRoundTripsThroughTheFileAndTheCapDropsTheOldest() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let store = FilePresentationStore(url: dir.appendingPathComponent("presentation.json"))
        XCTAssertEqual(try store.load(), PresentationState(), "an absent file is the empty state")
        var state = PresentationState()
        let window = UUID()
        state.setMode(.screen, for: window, at: Date(timeIntervalSince1970: 1))
        try store.save(state)
        let back = try store.load()
        XCTAssertEqual(back.mode(for: window), .screen)
        XCTAssertEqual(back, state)
        for i in 0..<PresentationState.modeCap {
            state.setMode(.compact, for: UUID(), at: Date(timeIntervalSince1970: Double(10 + i)))
        }
        XCTAssertEqual(state.modes.count, PresentationState.modeCap)
        XCTAssertNil(state.mode(for: window), "the oldest went")
        let text = String(decoding: try Data(contentsOf: store.url), as: UTF8.self)
        XCTAssertTrue(text.contains("\"screen\"") || text.contains("\"compact\""))
    }
}
