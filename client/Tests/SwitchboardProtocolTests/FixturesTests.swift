import Foundation
import XCTest

@testable import SwitchboardProtocol

/// `client/Fixtures/*.json`, written by the Rust port's fixture test
/// (`tests/remote.rs`), must decode here; the Rust side checks the same
/// files, so a shape change on either side fails a test.
final class FixturesTests: XCTestCase {
    func testEveryFixtureDecodes() throws {
        let info: Envelope<ServerInfo> = try Fixtures.load("info")
        XCTAssertEqual(info.body.protocolVersion, RemoteProtocol.version)
        XCTAssertEqual(info.body.capabilities, ["read", "control", "draft", "scope", "tickets", "evidence", "create"])
        XCTAssertEqual(info.body.spkiSha256.count, 64)
        XCTAssertEqual(info.body.limits.screenLines, 200)
        XCTAssertFalse(info.body.pairingOpen)

        let pair: Envelope<PairReply> = try Fixtures.load("pair")
        XCTAssertEqual(pair.body.scope, .read)
        XCTAssertEqual(pair.body.serverID, info.body.serverID)

        let projects: Envelope<ProjectsBody> = try Fixtures.load("projects")
        XCTAssertEqual(projects.body.projects.count, 1)
        let spaces: Envelope<SpacesBody> = try Fixtures.load("spaces")
        XCTAssertTrue(spaces.body.spaces.contains { $0.id == projects.body.projects[0].space })
        let sets: Envelope<SetsBody> = try Fixtures.load("sets")
        XCTAssertEqual(sets.body.sets.map(\.name), ["Desk"])

        let sessions: Envelope<SessionsBody> = try Fixtures.load("sessions")
        let first = try XCTUnwrap(sessions.body.sessions.first)
        XCTAssertEqual(first.name, "investigator")
        XCTAssertEqual(first.kind, .claude)
        XCTAssertEqual(first.liveness, .running)
        XCTAssertEqual(first.truncated, [])
        let session: Envelope<SessionBody> = try Fixtures.load("session")
        XCTAssertEqual(session.body.session.id, first.id)
        XCTAssertEqual(session.body.session.notes, "ticket 1\nnotes with two lines")
        let waiting: Envelope<SessionsBody> = try Fixtures.load("waiting")
        XCTAssertTrue(waiting.body.sessions.allSatisfy(\.waiting))

        let tickets: Envelope<TicketsBody> = try Fixtures.load("tickets")
        XCTAssertTrue(tickets.body.connected)
        XCTAssertEqual(tickets.body.tickets.map(\.id), ["t1", "t2", "t3"])
        XCTAssertEqual(tickets.body.projects.first?.supervisorSession, "sup-session-id")
        let listed = try XCTUnwrap(tickets.body.tickets.first)
        XCTAssertEqual(listed.standing, "1 waiting on you")
        XCTAssertEqual(listed.stage, "plan")
        XCTAssertNil(listed.body, "the listing leaves the body out")
        XCTAssertEqual(listed.pendingDecisions.map(\.name), ["lanes"])
        XCTAssertTrue(listed.pendingDecisions[0].multiple)
        let ticket: Envelope<TicketView> = try Fixtures.load("ticket")
        XCTAssertEqual(ticket.body.body, "Each entry in its own file.")
        XCTAssertEqual(ticket.body.attempts.first?.session, "worker-session-id")
        XCTAssertEqual(ticket.body.lanes.map(\.branch), ["t1-frontend"])
        let decided: Envelope<DecidedBody> = try Fixtures.load("decided")
        XCTAssertEqual(decided.body.decision.id, "d1")
        let attempt = try XCTUnwrap(ticket.body.attempts.first)
        XCTAssertEqual(attempt.files.map(\.name), ["notes", "token", "checks", "probe.log", "shot.png"])
        XCTAssertEqual(attempt.files.map(\.readable), [true, false, true, true, true])
        XCTAssertEqual(attempt.checks?.exit, 0)
        let file: Envelope<TicketFileBody> = try Fixtures.load("ticket_file")
        XCTAssertEqual(file.body.key, attempt.files[0].key)
        XCTAssertEqual(file.body.text, "Found it: one file each.\n")
        XCTAssertNil(file.body.imageData)
        let events: Envelope<TicketEventsBody> = try Fixtures.load("ticket_events")
        XCTAssertEqual(events.body.events.map(\.kind), ["stage", "attempt-started"])
        XCTAssertEqual(events.body.events[1].attempt, "investigate/1")
        XCTAssertFalse(events.body.unavailable)
        let closed: Envelope<TicketResumedBody> = try Fixtures.load("closed")
        XCTAssertEqual(closed.body.ticket.state, "closing")
        let created: Envelope<CommandBody> = try Fixtures.load("created")
        XCTAssertEqual(created.body.made.map(\.kind), ["session"])
        XCTAssertEqual(created.body.made.first?.id.count, 36, "a record id")

        let screen: Envelope<ScreenBody> = try Fixtures.load("screen")
        XCTAssertTrue(screen.body.text.hasPrefix("$ claude"))
        XCTAssertFalse(screen.body.truncated)
        XCTAssertEqual(screen.epoch, sessions.epoch)

        let page: Envelope<ConversationBody> = try Fixtures.load("conversation")
        XCTAssertEqual(page.body.total, 2)
        XCTAssertEqual(page.body.from, 1)
        XCTAssertEqual(page.body.next, 2)
        XCTAssertEqual(page.body.turns.map(\.n), [1])
        XCTAssertEqual(page.body.turns[0].user, "prompt 1")
        XCTAssertEqual(page.body.title, "explain-repo")
        let cut = try XCTUnwrap(page.body.turns[0].activity.first)
        XCTAssertTrue(cut.more, "the page carries the head of the result")
        let whole: Envelope<TurnBody> = try Fixtures.load("turn")
        XCTAssertEqual(whole.body.turn?.n, 1)
        XCTAssertEqual(whole.body.resumeID, page.body.resumeID)
        let full = try XCTUnwrap(whole.body.turn?.activity.first)
        XCTAssertFalse(full.more)
        XCTAssertEqual(full.detail?.name, "Read")
        XCTAssertGreaterThan(full.detail?.result.count ?? 0, cut.detail?.result.count ?? 0)
        let last: Envelope<LastResponseBody> = try Fixtures.load("last_response")
        XCTAssertEqual(last.body.n, 2)
        XCTAssertEqual(last.body.text, "pong")
        XCTAssertEqual(last.body.resumeID, page.body.resumeID)
        let runs: Envelope<RunsBody> = try Fixtures.load("runs")
        XCTAssertEqual(runs.body.runs.map(\.n), [1])
        XCTAssertFalse(runs.body.runs[0].open)
        let output: Envelope<RunOutputBody> = try Fixtures.load("run_output")
        XCTAssertEqual(output.body.n, 1)
        XCTAssertTrue(output.body.text.hasPrefix("$ make"))

        let command: Envelope<CommandBody> = try Fixtures.load("command")
        XCTAssertEqual(command.body.made, [])
        let status: Envelope<OpStatusBody> = try Fixtures.load("op_status")
        XCTAssertEqual(status.body.status, .done(failed: nil))
        let failed = try Wire.decode(
            OpStatusBody.self,
            from: Data(#"{"status":{"status":"done","reply":{"reply":"failed","reason":"not running"}}}"#.utf8))
        XCTAssertEqual(failed.status, .done(failed: "not running"))
        XCTAssertEqual(
            try Wire.decode(OpStatusBody.self, from: Data(#"{"status":{"status":"interrupted"}}"#.utf8)).status,
            .interrupted)
        XCTAssertTrue(info.body.capabilities.contains("control"))

        let error = try Fixtures.decode(ErrorEnvelope.self, "error")
        XCTAssertEqual(error.error.code, "unauthorized")
    }

    func testLivenessAndPinTargetTagging() throws {
        let exited = try Wire.decode(Liveness.self, from: Data(#"{"exited":{"code":1}}"#.utf8))
        XCTAssertEqual(exited, .exited(code: 1))
        let signal = try Wire.decode(Liveness.self, from: Data(#"{"exited":{"code":null}}"#.utf8))
        XCTAssertEqual(signal, .exited(code: nil))
        XCTAssertEqual(try Wire.decode(Liveness.self, from: Data(#""missing""#.utf8)), .missing)
        XCTAssertThrowsError(try Wire.decode(Liveness.self, from: Data(#""asleep""#.utf8)))

        let pin = try Wire.decode(
            Pin.self,
            from: Data(
                #"{"target":{"session":{"session":"s1"}},"rect":{"x":0,"y":1,"w":2,"h":3}}"#.utf8))
        XCTAssertEqual(pin.target, .session("s1"))
        let file = try Wire.decode(
            PinTarget.self, from: Data(#"{"file":{"project":"p","path":"src/a.rs"}}"#.utf8))
        XCTAssertEqual(file, .file(project: "p", path: "src/a.rs"))
        let rule = try Wire.decode(SetRule.self, from: Data(#"{"kind":"recent","hours":24}"#.utf8))
        XCTAssertEqual(rule, .recent(hours: 24))
    }

    func testAnOlderHostsSessionDecodesWithDefaults() throws {
        // The fields a later host added are absent here.
        let text = """
            {"id":"a","project":"p","name":"n","kind":"shell","cwd":"/","notes":"",
             "liveness":"missing","card":"not running","last_exit":null,"last_stop_at_ms":null,
             "quiet_secs":null,"waiting":false,"waiting_reason":null,"resume_id":null,"op":null}
            """
        let view = try Wire.decode(SessionView.self, from: Data(text.utf8))
        XCTAssertFalse(view.trustQuestion)
        XCTAssertEqual(view.pending, [])
        XCTAssertNil(view.heldUntilMs)
    }

    func testInviteParsesTheQRText() throws {
        let text = """
            {"server_id":"17A9B824-AC51-47DC-A96A-C2B9E69C85B4","spki_sha256":"ab","code":"ABCD2345","addresses":["192.168.1.9:43187"]}
            """
        let invite = try PairingInvite.parse(text)
        XCTAssertEqual(invite.serverID.raw, "17a9b824-ac51-47dc-a96a-c2b9e69c85b4")
        XCTAssertEqual(invite.serverID.short, "17a9b8")
        XCTAssertEqual(invite.addresses, ["192.168.1.9:43187"])
    }

    func testInviteParsesThePageURL() throws {
        let url = try PairingInvite.parse(
            "https://192.168.1.9:43187/#code=ABCD2345&key=ab&id=17A9B824-AC51-47DC-A96A-C2B9E69C85B4")
        let json = try PairingInvite.parse(
            #"{"server_id":"17A9B824-AC51-47DC-A96A-C2B9E69C85B4","spki_sha256":"ab","code":"ABCD2345","addresses":["192.168.1.9:43187"]}"#
        )
        XCTAssertEqual(url, json)
        XCTAssertThrowsError(
            try PairingInvite.parse("https://192.168.1.9:43187/#key=ab&id=17a9b824-ac51-47dc-a96a-c2b9e69c85b4"))
    }
}

enum Fixtures {
    static var directory: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Fixtures")
    }

    static func data(_ name: String) throws -> Data {
        try Data(contentsOf: directory.appendingPathComponent("\(name).json"))
    }

    static func load<T: Decodable & Sendable>(_ name: String) throws -> Envelope<T> {
        try Wire.decode(Envelope<T>.self, from: data(name))
    }

    static func decode<T: Decodable>(_ type: T.Type, _ name: String) throws -> T {
        try Wire.decode(type, from: data(name))
    }

    func testAFileFixtureDecodes() throws {
        let file: Envelope<FileBody> = try Fixtures.load("file")
        XCTAssertEqual(file.body.path, "docs/plan.md")
        XCTAssertEqual(file.body.size, UInt64(file.body.text.utf8.count))
        XCTAssertNotNil(file.body.modifiedMs)
    }
}
