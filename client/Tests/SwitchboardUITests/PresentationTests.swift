import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardUI

/// The presentation model (§7.1): the modes per kind, the window
/// request, and where a window's session stands.
final class PresentationTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")

    func testTheModesOfferedFollowTheKindAndAllButTheCardScale() {
        XCTAssertEqual(ViewMode.offered(for: .claude), [.compact, .screen, .lastResponse, .conversation])
        XCTAssertEqual(ViewMode.offered(for: .shell), [.compact, .screen])
        XCTAssertEqual(ViewMode.offered(for: .service), [.compact, .screen, .runOutput])
        XCTAssertTrue(ViewMode.screen.scalable)
        XCTAssertTrue(ViewMode.runOutput.scalable)
        XCTAssertTrue(ViewMode.conversation.scalable)
        XCTAssertTrue(ViewMode.lastResponse.scalable)
        XCTAssertFalse(ViewMode.compact.scalable)
        XCTAssertGreaterThan(ScreenScale.prosePoints, ScreenScale.calloutPoints)
        XCTAssertGreaterThan(ScreenScale.calloutPoints, ScreenScale.captionPoints)
        XCTAssertGreaterThan(ScreenScale.captionPoints, ScreenScale.caption2Points)
    }

    func testADefaultNameCountsTheProjectsSessionsOfTheKind() {
        let listed = [
            SessionView(id: "r1", project: "p", name: "one", kind: .claude),
            SessionView(id: "r2", project: "p", name: "two", kind: .shell),
        ]
        XCTAssertEqual(NewSessionNaming.defaultName(kind: .claude, existing: listed), "Claude 2")
        XCTAssertEqual(NewSessionNaming.defaultName(kind: .shell, existing: listed), "Shell 2")
        XCTAssertEqual(NewSessionNaming.defaultName(kind: .codex, existing: listed), "Codex 1")
        XCTAssertEqual(NewSessionNaming.defaultName(kind: .claude, existing: []), "Claude 1")
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
    func testAWindowsPlaceInItsConversationFollowsTheTranscriptAndSurvivesTheFile() throws {
        func conversation(_ id: String, _ turns: [Int]) -> ConversationState {
            ConversationState(page: ConversationBody(
                resumeID: id, total: turns.last ?? 0, from: turns.first ?? 1, next: nil,
                turns: turns.map { TurnView(n: $0, user: "p", finalText: "a") }))
        }
        let held = conversation("h", [4, 5, 6])
        var place = ConversationPlace(resumeID: "h", turn: 5, following: false)
        place.openTurns = [5]
        place.openActivities = [ActivityKey(turn: 5, index: 2)]
        XCTAssertEqual(place.fitted(to: held), place, "the turn is held: nothing changes")
        XCTAssertEqual(place.target(in: held), 5)
        // The turn went with a replaced page: the view follows the end.
        let later = conversation("h", [7, 8])
        let refitted = place.fitted(to: later)
        XCTAssertNil(refitted.turn)
        XCTAssertTrue(refitted.following)
        XCTAssertEqual(refitted.openTurns, [5], "folds are kept for when the turn comes back")
        XCTAssertNil(refitted.target(in: later))
        // Another transcript starts over.
        let cleared = place.fitted(to: conversation("h2", [1]))
        XCTAssertEqual(cleared, ConversationPlace(resumeID: "h2"))
        XCTAssertTrue(cleared.openTurns.isEmpty)
        // Following the end: no target, whatever the turn says.
        var following = place
        following.following = true
        XCTAssertNil(following.target(in: held))

        // Through the file, beside the window's mode, and an entry
        // written before places existed still reads.
        var state = PresentationState()
        let window = UUID()
        state.setMode(.conversation, for: window, at: Date(timeIntervalSince1970: 1))
        state.setPlace(place, for: window, at: Date(timeIntervalSince1970: 2))
        XCTAssertEqual(state.mode(for: window), .conversation, "the mode stays")
        state.setMode(.screen, for: window, at: Date(timeIntervalSince1970: 3))
        XCTAssertEqual(state.place(for: window), place, "and so does the place")
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let back = try decoder.decode(PresentationState.self, from: try encoder.encode(state))
        XCTAssertEqual(back, state)
        // An entry with no place encodes without the key, which is what
        // a file written before places existed holds.
        var before = PresentationState()
        before.setMode(.screen, for: window, at: Date(timeIntervalSince1970: 1))
        let old = try encoder.encode(before)
        XCTAssertFalse(String(decoding: old, as: UTF8.self).contains("place"))
        let older = try decoder.decode(PresentationState.self, from: old)
        XCTAssertEqual(older.mode(for: window), .screen)
        XCTAssertNil(older.place(for: window))
    }

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

    @MainActor func testTheLaunchLogAppendsToItsFileAndStartsOverWhenLarge() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("launch-\(UUID()).log")
        defer { try? FileManager.default.removeItem(at: file) }
        LaunchLog.file = file
        defer { LaunchLog.file = nil }
        LaunchLog.note("launch")
        LaunchLog.note("main appeared")
        let text = try String(contentsOf: file, encoding: .utf8)
        let lines = text.split(separator: "\n")
        XCTAssertEqual(lines.count, 2)
        XCTAssertTrue(lines[1].hasSuffix(" main appeared"))
        try Data(repeating: UInt8(ascii: "x"), count: 70_000).write(to: file)
        LaunchLog.note("again")
        let after = try String(contentsOf: file, encoding: .utf8)
        XCTAssertTrue(after.hasSuffix(" again\n"))
        XCTAssertLessThan(after.count, 100)
    }

    func testTheLaunchGraceIsTheFirstSecondsOfTheProcess() {
        let launched = Date()
        XCTAssertTrue(LaunchGrace.within(at: launched.addingTimeInterval(1), since: launched))
        XCTAssertTrue(LaunchGrace.within(at: launched.addingTimeInterval(7.9), since: launched))
        XCTAssertFalse(LaunchGrace.within(at: launched.addingTimeInterval(8), since: launched))
        XCTAssertFalse(LaunchGrace.within(at: launched.addingTimeInterval(60), since: launched))
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

    func testTablesAreCutOutOfProseWithTheirCellsAndAlignments() {
        let text = """
            Two columns:

            | Name | Count |
            |:-----|------:|
            | a \\| b | **1** |
            | c |
            | d | 2 | extra |

            and after.
            """
        let segments = MarkdownTable.split(text)
        XCTAssertEqual(segments.count, 3)
        XCTAssertEqual(segments.first, .prose("Two columns:"))
        XCTAssertEqual(segments.last, .prose("and after."))
        guard case .table(let table) = segments[1] else { return XCTFail("no table") }
        XCTAssertEqual(table.header, ["Name", "Count"])
        XCTAssertEqual(table.alignments, [.leading, .trailing])
        XCTAssertEqual(table.rows, [["a | b", "**1**"], ["c"], ["d", "2", "extra"]])
        XCTAssertEqual(table.columns, 3)
        XCTAssertEqual(table.cell(1, 1), "", "a short row reads empty")
        XCTAssertEqual(MarkdownTable.delimiter("| :-: | --- |"), [.center, .leading])
        XCTAssertNil(MarkdownTable.delimiter("| a | b |"))
    }

    func testPipesWithoutADelimiterRowOrInsideAFenceStayProse() {
        XCTAssertEqual(MarkdownTable.split("a | b\nc | d"), [.prose("a | b\nc | d")])
        let fenced = "```\n| a | b |\n|---|---|\n| 1 | 2 |\n```"
        XCTAssertEqual(MarkdownTable.split(fenced), [.prose(fenced)])
        XCTAssertEqual(MarkdownTable.split(""), [.prose("")])
        XCTAssertEqual(MarkdownTable.split("| a | b |\n|---|---|"), [.table(MarkdownTable(header: ["a", "b"], alignments: [.leading, .leading], rows: []))], "a header alone is a table")
    }

    func testDictationJoinsWhatWasHeardAfterWhatWasTyped() {
        XCTAssertEqual(Dictation.joined("", "fix the test"), "fix the test")
        XCTAssertEqual(Dictation.joined("please", "fix the test "), "please fix the test")
        XCTAssertEqual(Dictation.joined("please ", "fix"), "please fix")
        XCTAssertEqual(Dictation.joined("please", ""), "please")
        XCTAssertEqual(Dictation.joined("line\n", "next"), "line\nnext")
    }

    func testAClickReachesTheOpenWindowOnItsSessionOrMakesANewOne() {
        let r1 = SessionRef(serverID: a, recordID: "r1")
        let low = UUID(uuidString: "00000000-0000-0000-0000-000000000001")!
        let high = UUID(uuidString: "00000000-0000-0000-0000-000000000002")!
        let open = [high: r1, low: r1]
        XCTAssertEqual(CardWindowRequest.open(on: r1, among: open)?.id, low, "one window, the same each time")
        XCTAssertNil(CardWindowRequest.open(on: SessionRef(serverID: a, recordID: "r3"), among: open))
    }

    func testUtterancesCommitTheLiveOneWhenTheNextStartsLater() {
        var u = Utterances()
        _ = u.take("fix the", start: 0.3, finished: false)
        _ = u.take("fix the test", start: 0.4, finished: false)
        XCTAssertEqual(u.heard, "fix the test")
        XCTAssertEqual(u.live, "fix the test")
        // After a pause the recognizer covers only the new phrase, from later in the audio.
        u.take("and run it", start: 3.1, finished: false)
        XCTAssertEqual(u.committed, "fix the test")
        XCTAssertEqual(u.heard, "fix the test and run it")
        u.take("and run it twice", start: 3.1, finished: true)
        XCTAssertEqual(u.committed, "fix the test and run it twice")
        XCTAssertEqual(u.live, "")
        // The next task's audio starts over at zero; its first result is a new utterance.
        u.take("then push", start: 0.2, finished: false)
        XCTAssertEqual(u.heard, "fix the test and run it twice then push")
    }

    func testUtterancesProduceTheEditorsEventsInOrder() {
        var u = Utterances()
        var events = u.begin()
        events += u.take("fix the", start: 0.3, finished: false)
        events += u.take("fix the test", start: 0.4, finished: false)
        events += u.take("Fix the test.", start: 3.0, finished: false)
        events += u.take("and run it", start: 3.2, finished: false)
        events += u.end()
        XCTAssertEqual(events.map(\.sequence), Array(1...UInt64(events.count)), "numbered in order")
        XCTAssertEqual(
            events.map { "\($0.kind.rawValue):\($0.utterance):\($0.revision):\($0.text)" },
            [
                "started:0:0:",
                "voice_started:1:0:",
                "partial:1:1:fix the",
                "partial:1:2:fix the test",
                "final:1:2:Fix the test.",
                "voice_started:2:0:",
                "partial:2:1:and run it",
                "final:2:1:and run it",
                "stopped:0:0:",
            ])
        XCTAssertEqual(u.heard, "Fix the test. and run it")
    }

    /// The prompt window marks the live utterance in amber by the
    /// draft's byte range, which the host counts in UTF-8.
    func testThePromptWindowMarksTheLiveUtterance() {
        let draft = DraftView(text: "Fix the café. and run it", live: [15, 25], status: "listening")
        XCTAssertEqual(draft.liveText, "and run it")
        let marked = PromptWindowView.marked(draft)
        let runs = marked.runs.map { (String(marked[$0.range].characters), $0.foregroundColor != nil) }
        XCTAssertEqual(runs.map(\.0), ["Fix the café. ", "and run it"])
        XCTAssertEqual(runs.map(\.1), [false, true])
        let plain = PromptWindowView.marked(DraftView(text: "done"))
        XCTAssertEqual(plain.runs.count, 1)
        XCTAssertNil(DraftView(text: "x", live: [0, 9]).liveText, "a range past the text marks nothing")
        XCTAssertNotEqual(PromptWindow.id, MainWindow.id)
    }

    func testTheRecognizerClosesAnUtteranceBySendingItsWordsAgainLater() {
        var u = Utterances()
        u.take("fix the test", start: 0.4, finished: false)
        u.take("Fix the test.", start: 3.0, finished: false)
        XCTAssertEqual(u.committed, "Fix the test.")
        XCTAssertEqual(u.live, "")
        u.take("and run it", start: 3.2, finished: false)
        XCTAssertEqual(u.heard, "Fix the test. and run it")
        XCTAssertTrue(Utterances.sameWords("fix the test", "Fix, the test!"))
        XCTAssertFalse(Utterances.sameWords("fix the test", "fix the tests"))
    }

    func testAnEditInsideTheLiveUtteranceDropsItsRevisions() {
        var u = Utterances()
        u.take("hello", start: 0.1, finished: false)
        u.take("world", start: 2.0, finished: false)
        u.absorb(keepingLive: false)
        XCTAssertEqual(u.heard, "")
        u.take("world again", start: 2.0, finished: false)
        XCTAssertEqual(u.heard, "", "a revision of the edited utterance is dropped")
        u.take("next", start: 4.0, finished: false)
        XCTAssertEqual(u.heard, "next")
        var kept = Utterances()
        kept.take("hello", start: 0.1, finished: false)
        kept.take("world", start: 2.0, finished: false)
        kept.absorb(keepingLive: true)
        XCTAssertEqual(kept.heard, "world")
        kept.take("world again", start: 2.0, finished: false)
        XCTAssertEqual(kept.heard, "world again")
    }
}
