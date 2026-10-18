import Foundation
import XCTest

@testable import SwitchboardClientCore
@testable import SwitchboardProtocol
@testable import SwitchboardTransport

/// Dispatch's tickets through the port (§4.7): listed from a host that
/// serves them and not asked of one that does not, refetched on
/// `changed tickets`, ordered with the waiting ones first, and a
/// decision answered, with a lost reply settled by reading back.
final class TicketTests: XCTestCase {
    let a = ServerID("11111111-1111-1111-1111-111111111111")
    let b = ServerID("22222222-2222-2222-2222-222222222222")
    let addrA = Address(host: "10.0.0.1", port: 43187)
    let addrB = Address(host: "10.0.0.2", port: 43187)

    func listing() -> TicketsBody {
        TicketsBody(
            connected: true,
            projects: [TicketProjectView(name: "Orchard", supervisorSession: "sup-1")],
            tickets: [
                TicketView(
                    id: "t1", project: "Orchard", number: 104, title: "One file per entry", stage: "plan",
                    stages: ["investigate", "plan"], standing: "plan running", createdMs: 1, updatedMs: 10),
                TicketView(
                    id: "t2", project: "Orchard", number: 9, title: "Roster import", stage: "plan",
                    stages: ["investigate", "plan"], standing: "1 waiting on you", waits: true, urgent: true,
                    decisions: [
                        TicketDecisionView(
                            id: "d1", name: "lanes", question: "Which lanes?", options: ["backend", "frontend"],
                            recommendation: "frontend", multiple: true)
                    ], createdMs: 2, updatedMs: 20),
                TicketView(
                    id: "t3", project: "Orchard", kind: "manual", title: "Parked one", state: "parked",
                    reason: "later", standing: "parked: later", urgent: true, createdMs: 3, updatedMs: 30),
            ])
    }

    /// Host A serves tickets, host B is older and does not.
    func twoHosts() async throws -> (ConnectionManager, FakeHost, FakeHost) {
        let network = FakeNetwork()
        let hostA = FakeHost(serverID: a, name: "Desk", spki: "ka")
        let hostB = FakeHost(serverID: b, name: "Laptop", spki: "kb")
        hostA.set {
            $0.capabilities = ["read", "control", "draft", "scope", "tickets", "evidence"]
            $0.tickets = listing()
            $0.ticketFiles = ["k-notes": TicketFileBody(key: "k-notes", text: "Found it.")]
            $0.ticketEvents = ["t1": TicketEventsBody(events: [
                TicketEventView(seq: 1, atMs: 3, stage: "investigate", kind: "stage", text: "queued → investigate"),
            ], last: 1)]
        }
        network.hosts = [addrA: hostA, addrB: hostB]
        let manager = ConnectionManager(
            registry: MemoryRegistry(), credentials: MemoryCredentials(), deviceName: "test",
            factory: network.factory)
        try await manager.load()
        try await manager.pair(
            invite: PairingInvite(serverID: a, spkiSha256: "ka", code: "ABCD2345", addresses: ["10.0.0.1:43187"]))
        try await manager.pair(
            invite: PairingInvite(serverID: b, spkiSha256: "kb", code: "ABCD2345", addresses: ["10.0.0.2:43187"]))
        await manager.connectAll()
        return (manager, hostA, hostB)
    }

    func testTicketsAreListedFromAHostThatServesThemWaitingFirst() async throws {
        let (manager, hostA, hostB) = try await twoHosts()
        let store = await manager.store
        XCTAssertEqual(store.tickets.map(\.ticket.id), ["t2", "t1", "t3"], "the waiting one first, then the host's order")
        XCTAssertEqual(store.tickets.map(\.serverName), ["Desk", "Desk", "Desk"])
        XCTAssertEqual(store.tickets[0].supervisor, SessionRef(serverID: a, recordID: "sup-1"))
        XCTAssertEqual(store.dispatchHosts, [a])
        XCTAssertTrue(hostA.calls.contains("tickets"))
        XCTAssertFalse(hostB.calls.contains("tickets"), "a host without the capability is not asked")
    }

    func testAChangedTicketsFrameRefetchesTheListing() async throws {
        let (manager, hostA, _) = try await twoHosts()
        let before = hostA.calls.filter { $0 == "tickets" }.count
        hostA.set {
            $0.revision = 5
            var next = listing()
            next.tickets[1].decisions[0] = TicketDecisionView(
                id: "d1", name: "lanes", question: "Which lanes?", options: ["backend", "frontend"],
                state: "answered", answer: "frontend")
            $0.tickets = TicketsBody(connected: true, projects: next.projects, tickets: [
                TicketView(id: "t2", project: "Orchard", number: 9, title: "Roster import", standing: "plan running",
                           decisions: next.tickets[1].decisions, createdMs: 2, updatedMs: 21),
            ])
        }
        hostA.emit(.changed, ResourceRef(kind: "tickets", id: ""), revision: 5)
        let settled = await settle(manager) { $0.tickets.count == 1 }
        XCTAssertTrue(settled, "the listing is refetched whole")
        XCTAssertEqual(hostA.calls.filter { $0 == "tickets" }.count, before + 1)
        let store = await manager.store
        XCTAssertEqual(store.tickets.first?.ticket.pendingDecisions, [])
    }

    func testADecisionIsAnsweredAndALostReplyIsSettledByReadingBack() async throws {
        let (manager, hostA, _) = try await twoHosts()
        let ref = TicketRef(serverID: a, ticketID: "t2")
        let outcome = await manager.decide(ref, decision: "d1", answer: "frontend", note: nil)
        XCTAssertEqual(outcome, .accepted)
        XCTAssertEqual(hostA.decides.map(\.1.answer), ["frontend"])
        let recorded = await settle(manager) {
            $0.tickets.first { $0.ticket.id == "t2" }?.ticket.decisions[0].state == "answered"
        }
        XCTAssertTrue(recorded, "recorded in place before the next frame")

        // Another decision, its reply lost: the ticket read back says it
        // was answered, so the device is not left to ask again.
        hostA.set {
            $0.tickets?.tickets[1].decisions.append(
                TicketDecisionView(id: "d2", name: "finalize", question: "Final?", options: ["finalize", "revise"]))
            $0.decideSilences = 1
        }
        let lost = await manager.decide(ref, decision: "d2", answer: "finalize", note: nil)
        XCTAssertEqual(lost, .accepted)
        XCTAssertEqual(hostA.decides.count, 2, "sent once")
        let maybe = await manager.readTicket(ref)
        let read = try XCTUnwrap(maybe)
        XCTAssertEqual(read.decisions.map(\.state), ["answered", "answered"])

        // A parked ticket resumes, and the listing moves at once.
        let parked = TicketRef(serverID: a, ticketID: "t3")
        let resumed = await manager.resumeTicket(parked)
        XCTAssertEqual(resumed, .accepted)
        let active = await settle(manager) { $0.tickets.first { $0.ticket.id == "t3" }?.ticket.state == "active" }
        XCTAssertTrue(active, "the listing moves at once")
    }

    func testTheListIsFilteredAndOrderedAsTheFiltersSay() async throws {
        let (manager, _, _) = try await twoHosts()
        let store = await manager.store
        var filters = TicketFilters()
        XCTAssertEqual(filters.apply(store.tickets).map(\.ticket.id), ["t3", "t2", "t1"], "last moved first")
        filters.sort = .waiting
        XCTAssertEqual(filters.apply(store.tickets).map(\.ticket.id), ["t2", "t3", "t1"])
        filters.sort = .created
        XCTAssertEqual(filters.apply(store.tickets).map(\.ticket.id), ["t3", "t2", "t1"])
        filters.sort = .recent
        filters.waitingOnly = true
        XCTAssertEqual(filters.apply(store.tickets).map(\.ticket.id), ["t2"])
        filters = TicketFilters()
        filters.states = ["parked"]
        XCTAssertEqual(filters.apply(store.tickets).map(\.ticket.id), ["t3"])

        // A closed ticket is hidden until closed is asked for.
        var closed = store.tickets
        closed.append(UnifiedTicket(
            ref: TicketRef(serverID: a, ticketID: "t4"),
            ticket: TicketView(id: "t4", project: "Orchard", title: "Done", state: "closed", updatedMs: 99),
            serverName: "Desk", stale: false, supervisor: nil))
        filters = TicketFilters()
        XCTAssertEqual(filters.apply(closed).map(\.ticket.id), ["t3", "t2", "t1"])
        filters.states = ["closed"]
        XCTAssertEqual(filters.apply(closed).map(\.ticket.id), ["t4"])
        let choices = TicketFilterChoices(store: store)
        XCTAssertEqual(choices.projects, ["Orchard"])
        XCTAssertEqual(choices.states, ["active", "parked"])
        XCTAssertEqual(choices.hosts.map(\.name), ["Desk"], "only hosts with tickets")

        // The filters survive a save by an older client's state.
        let data = try JSONEncoder().encode(filters)
        XCTAssertEqual(try JSONDecoder().decode(TicketFilters.self, from: data), filters)
        let older = try JSONDecoder().decode(TicketFilters.self, from: Data("{}".utf8))
        XCTAssertEqual(older, TicketFilters())
    }

    func testAFileAndTheEventsAreReadAndATicketIsClosed() async throws {
        let (manager, hostA, _) = try await twoHosts()
        let ref = TicketRef(serverID: a, ticketID: "t1")
        let hosts = await manager.evidenceHosts
        XCTAssertEqual(hosts, [a])
        let file = try await manager.readTicketFile(ref, key: "k-notes")
        XCTAssertEqual(file.text, "Found it.")
        do {
            _ = try await manager.readTicketFile(ref, key: "nope")
            XCTFail("a missing file is refused")
        } catch RemoteError.refused(let status, _, _) {
            XCTAssertEqual(status, 404)
        }
        let events = try await manager.readTicketEvents(ref)
        XCTAssertEqual(events.events.map(\.text), ["queued → investigate"])
        XCTAssertEqual(events.last, 1)

        // A close on a ticket the host marks closable; another is refused.
        hostA.set {
            $0.tickets?.tickets[2] = TicketView(
                id: "t3", project: "Orchard", kind: "manual", title: "Parked one", state: "parked",
                standing: "parked", closable: true, createdMs: 3, updatedMs: 30)
        }
        let refused = await manager.closeTicket(ref)
        XCTAssertEqual(refused, .refused("the ticket is active"))
        let closed = await manager.closeTicket(TicketRef(serverID: a, ticketID: "t3"))
        XCTAssertEqual(closed, .accepted)
        let closing = await settle(manager) { $0.tickets.first { $0.ticket.id == "t3" }?.ticket.state == "closing" }
        XCTAssertTrue(closing, "the listing moves at once")
        XCTAssertTrue(hostA.calls.contains("close_ticket"))
    }

    /// Pump the manager until `check` holds on its store, or a second passes.
    private func settle(_ manager: ConnectionManager, _ check: @escaping (UnifiedStore) -> Bool) async -> Bool {
        for _ in 0..<100 {
            if check(await manager.store) { return true }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        return false
    }
}
