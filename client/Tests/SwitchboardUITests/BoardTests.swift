import Foundation
import SwitchboardClientCore
import SwitchboardProtocol
import XCTest

@testable import SwitchboardUI

/// The pin board: where tiles go, how they move, and that a board
/// survives a save.
final class BoardTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")

    func testTheGridSharesTheBoardAmongTheColumnsAndAtMostTwoRows() {
        XCTAssertEqual(BoardLayout.rows(count: 0, columns: 3), 1)
        XCTAssertEqual(BoardLayout.rows(count: 3, columns: 3), 1)
        XCTAssertEqual(BoardLayout.rows(count: 4, columns: 3), 2)
        XCTAssertEqual(BoardLayout.rows(count: 7, columns: 3), 3)
        let size = CGSize(width: 1024, height: 500)
        let one = BoardLayout.tileSize(count: 2, columns: 3, in: size, spacing: 12)
        XCTAssertEqual(one.width, (1024 - 24) / 3, accuracy: 1e-9)
        XCTAssertEqual(one.height, 500, accuracy: 1e-9)
        let two = BoardLayout.tileSize(count: 7, columns: 3, in: size, spacing: 12)
        XCTAssertEqual(two.height, (500 - 12) / 2, accuracy: 1e-9, "a third row scrolls rather than shrinking the tiles")
        let origin = BoardLayout.origin(of: 4, columns: 3, tile: CGSize(width: 100, height: 50), spacing: 10)
        XCTAssertEqual(origin, CGPoint(x: 110, y: 60))
    }

    func testADraggedTileLandsInTheSlotUnderItsCentreAndTheOrderMoves() {
        let tile = CGSize(width: 100, height: 50)
        XCTAssertEqual(BoardLayout.slot(at: CGPoint(x: 150, y: 20), count: 6, columns: 3, tile: tile, spacing: 10), 1)
        XCTAssertEqual(BoardLayout.slot(at: CGPoint(x: 230, y: 80), count: 6, columns: 3, tile: tile, spacing: 10), 5)
        XCTAssertEqual(BoardLayout.slot(at: CGPoint(x: -40, y: -40), count: 6, columns: 3, tile: tile, spacing: 10), 0)
        XCTAssertEqual(BoardLayout.slot(at: CGPoint(x: 900, y: 900), count: 4, columns: 3, tile: tile, spacing: 10), 3, "never past the last tile")
        XCTAssertEqual(BoardLayout.reordered(["a", "b", "c", "d"], from: 0, to: 2), ["b", "c", "a", "d"])
        XCTAssertEqual(BoardLayout.reordered(["a", "b", "c", "d"], from: 3, to: 1), ["a", "d", "b", "c"])
        XCTAssertEqual(BoardLayout.reordered(["a", "b"], from: 1, to: 1), ["a", "b"])
        XCTAssertEqual(BoardLayout.reordered(["a", "b"], from: 5, to: 0), ["a", "b"])
    }

    func testABoardRoundTripsThroughThePresentationStateAndAnOldStateHasNone() throws {
        var state = PresentationState()
        let ref = SessionRef(serverID: a, recordID: "r1")
        let file = FileRef(serverID: a, projectID: "p", path: "docs/plan.md")
        state.boards = [Board(name: "Client work", tiles: [
            BoardTile(item: .session(ref)),
            BoardTile(item: .file(file)),
        ], columns: 4)]
        let data = try JSONEncoder().encode(state)
        let back = try JSONDecoder().decode(PresentationState.self, from: data)
        XCTAssertEqual(back.boards, state.boards)
        // A board saved by the first build, with a frame per tile and no columns.
        let early = #"{"boards":[{"id":"22222222-2222-2222-2222-222222222222","name":"Old","created":0,"tiles":[{"id":"33333333-3333-3333-3333-333333333333","item":{"session":{"_0":{"serverID":"11111111-1111-1111-1111-111111111111","recordID":"r1"}}},"frame":{"x":0,"y":0,"w":0.3,"h":0.5}}]}]}"#
        let old = try JSONDecoder().decode(PresentationState.self, from: Data(early.utf8))
        XCTAssertEqual(old.boards.first?.columns, 3)
        XCTAssertEqual(old.boards.first?.tiles.first?.item, .session(ref))
        let none = try JSONDecoder().decode(PresentationState.self, from: Data(#"{"browse":"list"}"#.utf8))
        XCTAssertEqual(none.boards, [Board]())
    }

    func testTheAddMenusChoicesIgnoreWhatAPullChangesOnACard() {
        let one = UnifiedSession(
            ref: SessionRef(serverID: a, recordID: "r1"),
            session: SessionView(id: "r1", project: "p", name: "one", kind: .claude, card: "working"),
            serverName: "Mac", stale: false)
        let same = UnifiedSession(
            ref: one.ref,
            session: SessionView(id: "r1", project: "p", name: "one", kind: .claude, card: "waiting on you"),
            serverName: "Mac", stale: true)
        XCTAssertEqual(SessionChoiceGroup.groups([one]), SessionChoiceGroup.groups([same]))
        let renamed = UnifiedSession(
            ref: one.ref, session: SessionView(id: "r1", project: "p", name: "two", kind: .claude),
            serverName: "Mac", stale: false)
        XCTAssertNotEqual(SessionChoiceGroup.groups([one]), SessionChoiceGroup.groups([renamed]))
        XCTAssertEqual(SessionChoiceGroup.groups([one]).map(\.host), ["Mac"])
    }

    func testAFileWindowRequestReachesTheOpenWindowOnItsFile() {
        let file = FileRef(serverID: a, projectID: "p", path: "a.rs")
        let other = FileRef(serverID: a, projectID: "p", path: "b.rs")
        let id = UUID()
        XCTAssertEqual(FileWindowRequest.open(on: file, among: [id: file, UUID(): other])?.id, id)
        XCTAssertNil(FileWindowRequest.open(on: other, among: [id: file]))
    }
}
