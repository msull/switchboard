import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol

/// The conversation as the client holds it across pages (§4.3).
final class ConversationTests: XCTestCase {
    func turn(_ n: Int, _ answer: String = "") -> TurnView {
        TurnView(n: n, user: "p\(n)", finalText: answer.isEmpty ? "a\(n)" : answer)
    }

    func testPagesMergeByTurnAndTheEndReplacesWhatFollows() {
        var held = ConversationState(page: ConversationBody(
            resumeID: "h", total: 5, from: 4, next: nil, turns: [turn(4), turn(5, "draft")]))
        XCTAssertTrue(held.hasEarlier)
        XCTAssertEqual(held.firstHeld, 4)
        // An earlier page goes in front.
        held.merge(ConversationBody(resumeID: "h", total: 5, from: 1, next: 4, turns: [turn(1), turn(2), turn(3)]))
        XCTAssertEqual(held.turns.map(\.n), [1, 2, 3, 4, 5])
        XCTAssertFalse(held.hasEarlier)
        // The in-progress last turn refetched from itself: replaced, and a new one after it.
        held.merge(ConversationBody(resumeID: "h", total: 6, from: 5, next: nil, turns: [turn(5, "final"), turn(6)]))
        XCTAssertEqual(held.turns.map(\.n), [1, 2, 3, 4, 5, 6])
        XCTAssertEqual(held.turns[4].finalText, "final")
        XCTAssertEqual(held.total, 6)
        // A page that reaches the end drops anything after it (a rewind).
        held.merge(ConversationBody(resumeID: "h", total: 4, from: 4, next: nil, turns: [turn(4)]))
        XCTAssertEqual(held.turns.map(\.n), [1, 2, 3, 4])
    }

    func testAnotherResumeHandleDropsEverythingHeld() {
        var held = ConversationState(page: ConversationBody(
            resumeID: "h", total: 2, from: 1, next: nil, turns: [turn(1), turn(2)]))
        held.merge(ConversationBody(resumeID: "h2", total: 1, from: 1, next: nil, turns: [turn(1, "fresh")]))
        XCTAssertEqual(held.resumeID, "h2")
        XCTAssertEqual(held.turns.map(\.finalText), ["fresh"])
    }
}
