import Foundation
import SwitchboardProtocol
import XCTest

@testable import SwitchboardUI

/// The pin board: where tiles go, how they move, and that a board
/// survives a save.
final class BoardTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")

    func testANewTileTakesTheFirstFreeSlotAndStepsAsideWhenTheBoardIsFull() {
        let first = UnitRect(x: 0, y: 0, w: 1 / 3, h: 0.5)
        XCTAssertEqual(BoardLayout.place(avoiding: []), first)
        var tiles = [first]
        XCTAssertEqual(BoardLayout.place(avoiding: tiles), UnitRect(x: 1 / 3, y: 0, w: 1 / 3, h: 0.5))
        // Three across, then the gap left by a tile taken off is filled.
        tiles = [first, UnitRect(x: 2 / 3, y: 0, w: 1 / 3, h: 0.5)]
        XCTAssertEqual(BoardLayout.place(avoiding: tiles), UnitRect(x: 1 / 3, y: 0, w: 1 / 3, h: 0.5))
        // A tile nudged a little still counts as covering its slot; one
        // moved mostly out of it does not.
        tiles = [UnitRect(x: 0.05, y: 0.05, w: 1 / 3, h: 0.5)]
        XCTAssertEqual(BoardLayout.place(avoiding: tiles), UnitRect(x: 1 / 3, y: 0, w: 1 / 3, h: 0.5))
        tiles = [UnitRect(x: 0.3, y: 0.4, w: 1 / 3, h: 0.5)]
        XCTAssertEqual(BoardLayout.place(avoiding: tiles), first)
        // A full board: the next goes down and right of a slot, on top.
        tiles = (0..<6).map { BoardLayout.slot($0) }
        let seventh = BoardLayout.place(avoiding: tiles)
        XCTAssertEqual(seventh.x, 0.04, accuracy: 1e-9)
        XCTAssertEqual(seventh.y, 0.04, accuracy: 1e-9)
    }

    func testAMoveAndAResizeAreFractionsOfTheBoardAndStayOnIt() {
        let size = CGSize(width: 1000, height: 500)
        let tile = UnitRect(x: 0.1, y: 0.1, w: 0.3, h: 0.4)
        let moved = BoardLayout.moved(tile, by: CGSize(width: 100, height: 50), in: size)
        XCTAssertEqual(moved.x, 0.2, accuracy: 1e-9)
        XCTAssertEqual(moved.y, 0.2, accuracy: 1e-9)
        // Dragged off the far edge, it stops at it.
        let far = BoardLayout.moved(tile, by: CGSize(width: 5000, height: 5000), in: size)
        XCTAssertEqual(far.x, 0.7, accuracy: 1e-9)
        XCTAssertEqual(far.y, 0.6, accuracy: 1e-9)
        let grown = BoardLayout.resized(tile, by: CGSize(width: 100, height: -500), in: size)
        XCTAssertEqual(grown.w, 0.4, accuracy: 1e-9)
        XCTAssertEqual(grown.h, UnitRect.minimum, accuracy: 1e-9, "never below the minimum")
        let huge = BoardLayout.resized(tile, by: CGSize(width: 5000, height: 5000), in: size)
        XCTAssertEqual(huge.w, 0.9, accuracy: 1e-9, "no wider than the board allows from its place")
        XCTAssertEqual(huge.h, 0.9, accuracy: 1e-9)
        // A board with no size yet changes nothing.
        XCTAssertEqual(BoardLayout.moved(tile, by: CGSize(width: 9, height: 9), in: .zero), tile)
    }

    func testABoardRoundTripsThroughThePresentationStateAndAnOldStateHasNone() throws {
        var state = PresentationState()
        let ref = SessionRef(serverID: a, recordID: "r1")
        let file = FileRef(serverID: a, projectID: "p", path: "docs/plan.md")
        state.boards = [Board(name: "Client work", tiles: [
            BoardTile(item: .session(ref), frame: BoardLayout.slot(0)),
            BoardTile(item: .file(file), frame: BoardLayout.slot(1)),
        ])]
        let data = try JSONEncoder().encode(state)
        let back = try JSONDecoder().decode(PresentationState.self, from: data)
        XCTAssertEqual(back.boards, state.boards)
        let old = try JSONDecoder().decode(PresentationState.self, from: Data(#"{"browse":"list"}"#.utf8))
        XCTAssertEqual(old.boards, [])
    }

    func testAFileWindowRequestReachesTheOpenWindowOnItsFile() {
        let file = FileRef(serverID: a, projectID: "p", path: "a.rs")
        let other = FileRef(serverID: a, projectID: "p", path: "b.rs")
        let id = UUID()
        XCTAssertEqual(FileWindowRequest.open(on: file, among: [id: file, UUID(): other])?.id, id)
        XCTAssertNil(FileWindowRequest.open(on: other, among: [id: file]))
    }
}
