import Foundation
import SwiftUI
import SwitchboardClientCore
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

    func testTheSlotOffersRunningSessionsThisProjectFirstAndLastActiveFirst() {
        let host = ServerID("host")
        func item(
            _ id: String, project: String = "p", running: Bool = true, at: UInt64 = 0, on server: ServerID? = nil
        ) -> UnifiedSession {
            UnifiedSession(
                ref: SessionRef(serverID: server ?? host, recordID: id),
                session: SessionView(
                    id: id, project: project, name: id, liveness: running ? .running : .missing, lastStopAtMs: at),
                serverName: "Mini", stale: false)
        }
        let owner = SessionRef(serverID: host, recordID: "me")
        let all = [
            item("me"), item("old", at: 1), item("new", at: 9), item("ended", running: false, at: 99),
            item("other", project: "q", at: 5), item("far", at: 7, on: ServerID("elsewhere")),
        ]
        var got = SlotChoices.sessions(all, owner: owner, project: "p", query: "", ended: false)
        XCTAssertEqual(got.here.map(\.session.name), ["new", "old"])
        // Another project, or the same project id on another host, is elsewhere.
        XCTAssertEqual(got.elsewhere.map(\.session.name), ["far", "other"])
        XCTAssertEqual(SlotChoices.endedCount(all, owner: owner), 1)

        got = SlotChoices.sessions(all, owner: owner, project: "p", query: "", ended: true)
        XCTAssertEqual(got.here.map(\.session.name), ["ended", "new", "old"])

        got = SlotChoices.sessions(all, owner: owner, project: "p", query: " OTH ", ended: false)
        XCTAssertEqual(got.here.map(\.session.name), [])
        XCTAssertEqual(got.elsewhere.map(\.session.name), ["other"])
        XCTAssertTrue(SlotChoices.matches("", "anything"))
        XCTAssertFalse(SlotChoices.matches("zzz", "notes.md"))
    }

    func testAFileInAnotherProjectDecodesLinksAndIsOfferedThere() throws {
        let json = """
            {"base":null,"files":[
              {"path":"decisions.md","project":"other","in_repo":false,"changed":false,"added":0,"removed":0,"binary":false},
              {"path":"src/a.rs","project":null,"in_repo":true,"changed":true,"added":1,"removed":0,"binary":false}]}
            """
        let files = try JSONDecoder().decode(EditedFilesBody.self, from: Data(json.utf8)).files
        XCTAssertEqual(files.map(\.project), ["other", nil])
        let owner = SessionRef(serverID: ServerID("host"), recordID: "me")
        XCTAssertEqual(
            SlotChoices.written(files, owner: owner, project: "mine", query: ""),
            [
                FileRef(serverID: ServerID("host"), projectID: "other", path: "decisions.md"),
                FileRef(serverID: ServerID("host"), projectID: "mine", path: "src/a.rs"),
            ])

        // A link into another project carries it, and one into the
        // session's own carries none.
        let url = try XCTUnwrap(FileLinks.url(path: "docs/open decisions.md", line: 3, project: "0a1b-2c"))
        let parsed = try XCTUnwrap(FileLinks.parse(try XCTUnwrap(URL(string: url))))
        XCTAssertEqual(parsed.path, "docs/open decisions.md")
        XCTAssertEqual(parsed.line, 3)
        XCTAssertEqual(parsed.project, "0a1b-2c")
        let own = try XCTUnwrap(FileLinks.url(path: "plan.md", line: nil))
        XCTAssertNil(FileLinks.parse(try XCTUnwrap(URL(string: own)))?.project)
        let link = try JSONDecoder().decode(
            LinkView.self, from: Data(#"{"start":0,"end":4,"path":"a.md","line":null,"project":"p2"}"#.utf8))
        XCTAssertEqual(link.project, "p2")

        let found = try JSONDecoder().decode(
            FindBody.self, from: Data(#"{"hits":[{"project":"p2","path":"a.md"}],"truncated":true}"#.utf8))
        XCTAssertEqual(found.hits, [FileHit(project: "p2", path: "a.md")])
        XCTAssertTrue(found.truncated)
    }

    func testTheChooserOffersTheNewestWrittenFilesAndTheOnesOpenedBefore() {
        let host = ServerID("host")
        let owner = SessionRef(serverID: host, recordID: "me")
        let files = (1...20).map { EditedFile(path: "f\($0).txt") }
        // The newest dozen until a filter is typed, then every match.
        XCTAssertEqual(SlotChoices.written(files, owner: owner, project: "p", query: "").count, SlotChoices.writtenShown)
        XCTAssertEqual(
            SlotChoices.written(files, owner: owner, project: "p", query: "f2").map(\.path), ["f2.txt", "f20.txt"])

        var state = PresentationState()
        let a = FileRef(serverID: host, projectID: "p", path: "a.md")
        let b = FileRef(serverID: host, projectID: "q", path: "b.md")
        let far = FileRef(serverID: ServerID("elsewhere"), projectID: "p", path: "c.md")
        for file in [a, b, far, a] { state.noteOpened(file) }
        XCTAssertEqual(state.recentFiles, [a, far, b])
        // Only the owner's host, and not what another list already shows.
        XCTAssertEqual(SlotChoices.recent(state.recentFiles, owner: owner, query: "", besides: [b]), [a])
        XCTAssertEqual(SlotChoices.recent(state.recentFiles, owner: owner, query: "b.m", besides: []), [b])
        for i in 0..<(PresentationState.recentFilesCap + 5) {
            state.noteOpened(FileRef(serverID: host, projectID: "p", path: "n\(i).md"))
        }
        XCTAssertEqual(state.recentFiles.count, PresentationState.recentFilesCap)
        XCTAssertFalse(state.recentFiles.contains(a))
    }

    func testEachSideKeepsWhatItHoldsAndAnEmptiedLeftPaneStaysEmpty() throws {
        // A card saved before the left pane could hold anything has the files there.
        let old = try JSONDecoder().decode(
            PaneSettings.self, from: Data(#"{"tilt":20,"leading":1,"trailing":1,"filesOpen":true,"slotOpen":false}"#.utf8))
        XCTAssertEqual(old.leadingPane, .files)

        var panes = PaneSettings()
        XCTAssertEqual(panes.leadingPane, .files)
        panes.leadingPane = nil
        panes.slot = .files
        var back = try JSONDecoder().decode(PaneSettings.self, from: JSONEncoder().encode(panes))
        XCTAssertNil(back.leadingPane)
        XCTAssertEqual(back.slot, .files)
        panes.leadingPane = .document(FileRef(serverID: ServerID("host"), projectID: "p", path: "decisions.md"))
        back = try JSONDecoder().decode(PaneSettings.self, from: JSONEncoder().encode(panes))
        XCTAssertEqual(back, panes)

        XCTAssertEqual(SidePaneLayout.drawn(.files).width, 360)
        XCTAssertGreaterThan(SidePaneLayout.drawn(panes.leadingPane).width, SidePaneLayout.drawn(nil).width)
    }

    func testClearingUnchangedLeavesOffOnlyFilesThatMatchTheBase() {
        let files = [
            EditedFile(path: "same.rs", changed: false),
            EditedFile(path: "edited.rs", changed: true, added: 2),
            EditedFile(path: "outside.md", inRepo: false, changed: false),
        ]
        XCTAssertEqual(EditedFilesPane.shown(files, hideUnchanged: false).map(\.path), ["same.rs", "edited.rs", "outside.md"])
        XCTAssertEqual(EditedFilesPane.shown(files, hideUnchanged: true).map(\.path), ["edited.rs", "outside.md"])
    }

    func testACardSavedBefore0_14_1KeepsItsPanesAndShowsEveryFile() throws {
        let json = #"{"tilt":30,"leading":1,"trailing":0.8,"filesOpen":true,"slotOpen":false}"#
        let panes = try JSONDecoder().decode(PaneSettings.self, from: Data(json.utf8))
        XCTAssertEqual(panes.tilt, 30)
        XCTAssertEqual(panes.trailing, 0.8)
        XCTAssertTrue(panes.filesOpen)
        XCTAssertFalse(panes.hideUnchanged)
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
        XCTAssertEqual(panes.tilt, 20)
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
