import Foundation
import SwitchboardProtocol
import XCTest

@testable import SwitchboardUI

/// Web pages beside a card and on a board: the addresses a user types,
/// and what the presentation state keeps of them.
final class WebPageTests: XCTestCase {
    func testATypedAddressGetsTheSchemeItsHostCallsFor() {
        // A host on the local network is plain http, as a session serves it.
        XCTAssertEqual(WebAddress.parse("localhost:8000")?.absoluteString, "http://localhost:8000")
        XCTAssertEqual(WebAddress.parse("192.168.1.5:8000/report.html")?.absoluteString, "http://192.168.1.5:8000/report.html")
        XCTAssertEqual(WebAddress.parse("mini.local/a")?.absoluteString, "http://mini.local/a")
        XCTAssertEqual(WebAddress.parse("agentbox:3000")?.absoluteString, "http://agentbox:3000")
        XCTAssertEqual(WebAddress.parse(" example.com/docs ")?.absoluteString, "https://example.com/docs")
        // A scheme given is kept; any other is refused.
        XCTAssertEqual(WebAddress.parse("http://example.com")?.absoluteString, "http://example.com")
        XCTAssertEqual(WebAddress.parse("https://a.b/c?d=1#e")?.absoluteString, "https://a.b/c?d=1#e")
        XCTAssertNil(WebAddress.parse("file:///etc/hosts"))
        XCTAssertNil(WebAddress.parse("javascript://x"))
        // A word, or words, is a name to look for and not a page.
        XCTAssertNil(WebAddress.parse("decisions"))
        XCTAssertNil(WebAddress.parse("open decisions.md now"))
        XCTAssertNil(WebAddress.parse(""))
    }

    func testAPagesLabelTellsLocalPagesApart() throws {
        XCTAssertEqual(WebAddress.label(try XCTUnwrap(URL(string: "http://localhost:8000/report.html"))), "localhost:8000/report.html")
        XCTAssertEqual(WebAddress.label(try XCTUnwrap(URL(string: "https://example.com/"))), "example.com")
        XCTAssertTrue(WebAddress.isWeb(try XCTUnwrap(URL(string: "http://a"))))
        XCTAssertFalse(WebAddress.isWeb(try XCTUnwrap(URL(string: "switchboard-file:///a.md"))))
    }

    func testAPageStaysInItsPaneAndOnItsBoardAcrossASave() throws {
        let page = try XCTUnwrap(URL(string: "http://localhost:8000/report.html"))
        var panes = PaneSettings()
        panes.leadingPane = .web(page)
        panes.slot = .web(try XCTUnwrap(URL(string: "https://example.com/docs")))
        XCTAssertEqual(try JSONDecoder().decode(PaneSettings.self, from: JSONEncoder().encode(panes)), panes)
        XCTAssertGreaterThan(SidePaneLayout.drawn(.web(page)).width, SidePaneLayout.drawn(.files).width)

        let tile = BoardTile(item: .web(page))
        XCTAssertEqual(try JSONDecoder().decode(BoardTile.self, from: JSONEncoder().encode(tile)), tile)
    }

    func testOpenedPagesAreRememberedOnceNewestFirstAndOfferedByWhatIsTyped() throws {
        var state = PresentationState()
        let a = try XCTUnwrap(URL(string: "http://localhost:8000/report.html"))
        let b = try XCTUnwrap(URL(string: "https://example.com/docs"))
        for page in [a, b, a] { state.notePage(page) }
        XCTAssertEqual(state.recentPages, [a, b])
        XCTAssertEqual(SlotChoices.pages(state.recentPages, query: ""), [a, b])
        XCTAssertEqual(SlotChoices.pages(state.recentPages, query: "report"), [a])
        // The page the query itself names is offered once, as the one to open.
        XCTAssertEqual(SlotChoices.pages(state.recentPages, query: "localhost:8000/report.html"), [])
        for i in 0..<(PresentationState.recentPagesCap + 3) {
            state.notePage(try XCTUnwrap(URL(string: "https://example.com/\(i)")))
        }
        XCTAssertEqual(state.recentPages.count, PresentationState.recentPagesCap)

        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let back = try decoder.decode(PresentationState.self, from: encoder.encode(state))
        XCTAssertEqual(back.recentPages, state.recentPages)
    }
}
