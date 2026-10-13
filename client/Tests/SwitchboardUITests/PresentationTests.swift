import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardUI

/// The presentation model (§7.1): the modes per kind, the window
/// request, and where a window's session stands.
final class PresentationTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")

    func testTheModesOfferedFollowTheKindAndTheMonospacedOnesScale() {
        XCTAssertEqual(ViewMode.offered(for: .claude), [.compact, .screen, .lastResponse, .conversation])
        XCTAssertEqual(ViewMode.offered(for: .shell), [.compact, .screen])
        XCTAssertEqual(ViewMode.offered(for: .service), [.compact, .screen, .runOutput])
        XCTAssertTrue(ViewMode.screen.scalable)
        XCTAssertTrue(ViewMode.runOutput.scalable)
        XCTAssertFalse(ViewMode.conversation.scalable)
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

/// Browsing state (§8, §7.2): the filters and layout round-trip, focus
/// keeps its session across a reorder and wraps, and the saved windows
/// the system did not restore are the ones to reopen.
final class BrowsingTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")

    func item(_ id: String, card: String = "idle") -> UnifiedSession {
        UnifiedSession(
            ref: SessionRef(serverID: a, recordID: id),
            session: SessionView(id: id, project: "p", name: id, card: card),
            serverName: "Desk", stale: false)
    }

    func testFiltersLayoutAndFocusRoundTripThroughTheFile() throws {
        var state = PresentationState()
        state.browse = .grid
        state.filters.kinds = [.claude]
        state.filters.cards = ["waiting on you"]
        state.focused = SessionRef(serverID: a, recordID: "r2")
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let back = try decoder.decode(PresentationState.self, from: try encoder.encode(state))
        XCTAssertEqual(back, state)
        XCTAssertTrue(state.filters.admits(item("r1", card: "waiting on you")), "a waiting Claude session passes")
        XCTAssertFalse(state.filters.admits(item("r1", card: "idle")), "an idle one does not")
    }

    func testFocusStaysOnItsSessionAcrossAReorderAndWraps() {
        let items = [item("a"), item("b"), item("c")]
        XCTAssertEqual(FocusCursor.index(of: nil, in: items), 0, "nothing focused: the first")
        let b = items[1].ref
        XCTAssertEqual(FocusCursor.index(of: b, in: [item("c"), item("b"), item("a")]), 1)
        XCTAssertEqual(FocusCursor.moved(b, by: 1, in: items), items[2].ref)
        XCTAssertEqual(FocusCursor.moved(items[2].ref, by: 1, in: items), items[0].ref, "wraps")
        XCTAssertEqual(FocusCursor.moved(items[0].ref, by: -1, in: items), items[2].ref)
        let gone = SessionRef(serverID: a, recordID: "zz")
        XCTAssertEqual(FocusCursor.index(of: gone, in: items), 0, "a focused session that left: the first")
        XCTAssertNil(FocusCursor.index(of: gone, in: []))
    }

    func testTheWindowsToReopenAreTheSavedOnesNotRestoredWhoseSessionsAreListed() {
        var state = PresentationState()
        let restored = UUID()
        let missing = UUID()
        let unlisted = UUID()
        state.windows = [
            restored: SessionRef(serverID: a, recordID: "r1"),
            missing: SessionRef(serverID: a, recordID: "r1"),
            unlisted: SessionRef(serverID: a, recordID: "r9"),
        ]
        let listed: Set<SessionRef> = [SessionRef(serverID: a, recordID: "r1")]
        let reopen = state.windowsToReopen(appeared: [restored], listed: listed)
        XCTAssertEqual(reopen.map(\.id), [missing])
        XCTAssertEqual(reopen.first?.ref.recordID, "r1")
    }

    func testTheMainWindowIsBroughtBackOnceAndOnlyWhenItIsMissing() {
        var missing = MainWindowGuard()
        XCTAssertTrue(missing.claim(), "the first card window to ask opens it")
        XCTAssertFalse(missing.claim(), "the next does not")
        var shown = MainWindowGuard()
        shown.visible = true
        XCTAssertFalse(shown.claim(), "a restored main window needs no help")
        shown.visible = false
        XCTAssertTrue(shown.claim(), "the main window went away before the ask")
    }

    func testTheScreenScaleIsClampedAndReadAsAPercentage() {
        XCTAssertEqual(ScreenScale.initial, 1)
        XCTAssertEqual(ScreenScale.clamped(0.1), ScreenScale.range.lowerBound)
        XCTAssertEqual(ScreenScale.clamped(9), ScreenScale.range.upperBound)
        XCTAssertEqual(ScreenScale.clamped(1.5), 1.5)
        XCTAssertEqual(ScreenScale.label(1), "100%")
        XCTAssertEqual(ScreenScale.label(1.256), "126%")
        XCTAssertEqual(ScreenScale.presets.map(ScreenScale.label), ["50%", "75%", "100%", "150%", "200%"])
        XCTAssertTrue(ScreenScale.presets.allSatisfy { ScreenScale.range.contains($0) })
        XCTAssertTrue(ScreenScale.presets.contains(ScreenScale.initial))
        XCTAssertEqual(Set(ViewMode.allCases.map(\.symbol)).count, ViewMode.allCases.count, "every mode has its own icon")
    }

    func testAClickReachesTheOpenWindowOnItsSessionOrMakesANewOne() {
        var state = PresentationState()
        let r1 = SessionRef(serverID: a, recordID: "r1")
        let r2 = SessionRef(serverID: a, recordID: "r2")
        let low = UUID(uuidString: "00000000-0000-0000-0000-000000000001")!
        let high = UUID(uuidString: "00000000-0000-0000-0000-000000000002")!
        let gone = UUID()
        state.windows = [high: r1, low: r1, gone: r2]
        let appeared: Set<UUID> = [low, high]
        XCTAssertEqual(state.openWindow(on: r1, appeared: appeared)?.id, low, "one window, the same each time")
        XCTAssertNil(state.openWindow(on: r2, appeared: appeared), "a saved window not on screen this run is not it")
        XCTAssertNil(state.openWindow(on: SessionRef(serverID: a, recordID: "r3"), appeared: appeared))
    }
}
