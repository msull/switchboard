import Foundation
import SwitchboardProtocol
import XCTest

@testable import SwitchboardUI

/// The file links the host finds, put into the prose and read back
/// from a tap.
final class FileLinksTests: XCTestCase {
    func testRangesBecomeMarkdownLinksFromTheEndBack() {
        let text = "See plan.md and `deep/a.rs:12` now."
        let links = [
            LinkView(start: 4, end: 11, path: "plan.md"),
            LinkView(start: 16, end: 30, path: "sub/deep/a.rs", line: 12),
        ]
        XCTAssertEqual(
            FileLinks.linked(text, links),
            "See [plan.md](switchboard-file:///plan.md) and [`deep/a.rs:12`](switchboard-file:///sub/deep/a.rs#L12) now.")
    }

    func testARangeOffACharacterBoundaryOrPastTheEndIsLeftAlone() {
        let text = "é plan.md"
        // "é" is two bytes: the token starts at byte 3.
        XCTAssertEqual(
            FileLinks.linked(text, [LinkView(start: 3, end: 10, path: "plan.md")]),
            "é [plan.md](switchboard-file:///plan.md)")
        XCTAssertEqual(FileLinks.linked(text, [LinkView(start: 1, end: 10, path: "plan.md")]), text)
        XCTAssertEqual(FileLinks.linked(text, [LinkView(start: 3, end: 11, path: "plan.md")]), text)
        XCTAssertEqual(FileLinks.linked(text, []), text)
    }

    func testAPathWithSpacesAndParenthesesSurvivesTheRoundTrip() {
        let url = FileLinks.url(path: "docs/my notes (draft).md", line: 7)!
        XCTAssertEqual(url, "switchboard-file:///docs/my%20notes%20%28draft%29.md#L7")
        let parsed = FileLinks.parse(URL(string: url)!)
        XCTAssertEqual(parsed?.path, "docs/my notes (draft).md")
        XCTAssertEqual(parsed?.line, 7)
        XCTAssertNil(FileLinks.parse(URL(string: "https://example.com/plan.md")!))
        XCTAssertNil(FileLinks.parse(URL(string: "switchboard-file:///")!)?.path)
    }

    func testATurnWithoutLinksOnTheWireDecodesToNone() throws {
        let json = #"{"n":1,"user":"p","final_text":"a","activity":[{"kind":"text","line":"x"}]}"#
        let turn = try JSONDecoder().decode(TurnView.self, from: Data(json.utf8))
        XCTAssertEqual(turn.links, [])
        XCTAssertEqual(turn.activity[0].links, [])
        let linked = #"{"n":1,"user":"p","final_text":"a.rs","links":[{"start":0,"end":4,"path":"a.rs","line":null}]}"#
        let withLinks = try JSONDecoder().decode(TurnView.self, from: Data(linked.utf8))
        XCTAssertEqual(withLinks.links, [LinkView(start: 0, end: 4, path: "a.rs")])
    }
}
