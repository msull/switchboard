import Foundation
import SwiftUI
import SwitchboardProtocol
import XCTest

@testable import SwitchboardUI

/// A session's edited files and one file's diff, as the host's
/// `sessions/{id}/files` and `sessions/{id}/diff/...` bodies carry them
/// (the shapes `tests/remote.rs` asserts on the Rust side).
final class EditsTests: XCTestCase {
    func testTheFilesListDecodesWithItsBaseAndCounts() throws {
        let json = """
            {"base":{"sha":"abc123","kind":"branch","onto":"origin/main"},
             "files":[{"path":"notes.md","in_repo":true,"changed":false,"added":0,"removed":0,"binary":false},
                      {"path":"src/a.rs","in_repo":true,"changed":true,"added":3,"removed":1,"binary":false}]}
            """
        let body = try JSONDecoder().decode(EditedFilesBody.self, from: Data(json.utf8))
        XCTAssertEqual(body.base, DiffBase(sha: "abc123", kind: "branch", onto: "origin/main"))
        XCTAssertEqual(body.base?.label, "Since branching from origin/main")
        XCTAssertEqual(body.files.map(\.path), ["notes.md", "src/a.rs"])
        XCTAssertEqual(body.files[1].added, 3)
        XCTAssertFalse(body.files[0].changed)
    }

    func testAFilesListWithNoBaseIsTheSessionStartOrNone() throws {
        let json = #"{"base":null,"files":[{"path":"a","in_repo":false,"changed":false,"added":0,"removed":0,"binary":false}]}"#
        let body = try JSONDecoder().decode(EditedFilesBody.self, from: Data(json.utf8))
        XCTAssertNil(body.base)
        XCTAssertFalse(body.files[0].inRepo)
        XCTAssertEqual(DiffBase(sha: "s", kind: "session").label, "Since the session started")
    }

    func testADiffDecodesAsHunksOrWhyNot() throws {
        let json = """
            {"status":"modified","body":{"kind":"hunks","hunks":[{"old_start":1,"new_start":1,"lines":[
              {"kind":"removed","old":1,"new":null,"text":"let t = 1;","marks":[[0,3]]},
              {"kind":"added","old":null,"new":1,"text":"let t = 2;","marks":[]}]}]}}
            """
        let diff = try JSONDecoder().decode(FileDiffBody.self, from: Data(json.utf8))
        XCTAssertEqual(diff.status, "modified")
        let hunks = try XCTUnwrap(diff.body.hunks)
        XCTAssertEqual(hunks[0].lines.map(\.kind), ["removed", "added"])
        XCTAssertEqual(hunks[0].lines[0].marks, [[0, 3]])
        XCTAssertTrue(DiffWindowView.counts(hunks) == (1, 1))

        let big = try JSONDecoder().decode(
            FileDiffBody.self, from: Data(#"{"status":"added","body":{"kind":"too_large","lines":9000}}"#.utf8))
        XCTAssertEqual(big.body.lines, 9000)
        XCTAssertNil(big.body.hunks)
    }

    func testASidePaneScalesWithTheWindowsHeightAndKeepsItsShape() {
        XCTAssertEqual(SidePaneLayout.size(width: 360, height: 640, windowHeight: 900), CGSize(width: 360, height: 640))
        XCTAssertEqual(SidePaneLayout.size(width: 360, height: 640, windowHeight: 450), CGSize(width: 180, height: 320))
        // The user's size for the side, on top of the window's height.
        XCTAssertEqual(
            SidePaneLayout.size(width: 360, height: 640, windowHeight: 450, factor: 1.5), CGSize(width: 270, height: 480))
        // Before the window is measured, and past the limits.
        XCTAssertEqual(SidePaneLayout.size(width: 360, height: 640, windowHeight: 0), CGSize(width: 360, height: 640))
        XCTAssertEqual(SidePaneLayout.size(width: 400, height: 600, windowHeight: 90), CGSize(width: 140, height: 210))
        XCTAssertEqual(SidePaneLayout.size(width: 400, height: 600, windowHeight: 9000), CGSize(width: 600, height: 900))
    }

    func testACardsPanesAreKeptForItsSessionAndAnOlderStateHasNone() throws {
        let ref = SessionRef(serverID: ServerID("host"), recordID: "rec")
        var panes = PaneSettings()
        panes.tilt = 20
        panes.trailing = 0.75
        panes.filesOpen = true
        panes.slotOpen = true
        panes.slot = .document(FileRef(serverID: ServerID("host"), projectID: "p", path: "notes.md"))
        var state = PresentationState()
        state.setPanes(panes, for: ref)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let back = try decoder.decode(PresentationState.self, from: encoder.encode(state))
        XCTAssertEqual(back.panes(for: ref), panes)
        XCTAssertNil(back.panes(for: SessionRef(serverID: ServerID("host"), recordID: "other")))

        let old = try decoder.decode(PresentationState.self, from: Data(#"{"browse":"grid"}"#.utf8))
        XCTAssertTrue(old.panes.isEmpty)

        // Reset puts the angle and sizes back and leaves the panes open.
        panes.resetLayout()
        XCTAssertEqual(panes.tilt, 30)
        XCTAssertEqual(panes.trailing, 1)
        XCTAssertTrue(panes.slotOpen)
    }

    func testMarksOffACharacterBoundaryOrPastTheEndAreLeftUnmarked() {
        // "é" is two bytes, so byte 1 is inside it.
        let text = "é ab"
        let marked = DiffMarks.attributed(text, marks: [[3, 5], [1, 3], [3, 9]], tint: .green)
        XCTAssertEqual(String(marked.characters), text)
        let runs = marked.runs.filter { $0.backgroundColor != nil }
        XCTAssertEqual(runs.count, 1)
        XCTAssertEqual(runs.first.map { String(marked[$0.range].characters) }, "ab")
        // A context line is never marked.
        XCTAssertTrue(DiffMarks.attributed(text, marks: [[3, 5]], tint: .clear).runs.allSatisfy { $0.backgroundColor == nil })
    }
}
