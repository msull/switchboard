import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// Dispatch's tickets from every connected host, the ones waiting on
/// the owner first; a ticket opens on its decisions, body, attempts
/// and lanes.
public struct TicketsView: View {
    @Bindable var model: BrowserModel

    public init(model: BrowserModel) {
        self.model = model
    }

    public var body: some View {
        Group {
            if model.store.tickets.isEmpty {
                VStack {
                    Spacer()
                    Text(empty).foregroundStyle(.secondary).multilineTextAlignment(.center)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            } else {
                List {
                    ForEach(hosts, id: \.self) { host in
                        Section(host) {
                            ForEach(model.store.tickets.filter { $0.serverName == host }) { item in
                                NavigationLink(value: item.ref) {
                                    TicketRow(item: item)
                                }
                            }
                        }
                    }
                }
            }
        }
        .navigationDestination(for: TicketRef.self) { ref in
            TicketDetailView(model: model, ref: ref)
        }
    }

    private var hosts: [String] {
        var seen: [String] = []
        for item in model.store.tickets where !seen.contains(item.serverName) {
            seen.append(item.serverName)
        }
        return seen
    }

    private var empty: String {
        if model.store.dispatchHosts.isEmpty {
            return "No connected host runs Dispatch."
        }
        return "No tickets."
    }
}

/// One ticket as the list shows it: source and title, the project,
/// and where it stands.
struct TicketRow: View {
    let item: UnifiedTicket

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                if !item.ticket.source.isEmpty {
                    Text(item.ticket.source).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                Text(item.ticket.title).font(.headline).lineLimit(2)
                Spacer()
                SourceLabel(name: item.serverName, stale: item.stale, size: .small)
            }
            HStack(spacing: 8) {
                Text(item.ticket.project).font(.caption).foregroundStyle(.secondary)
                if let stage = item.ticket.stage {
                    Text(stage).font(.caption).foregroundStyle(.secondary)
                }
                Text(item.ticket.standing)
                    .font(.caption)
                    .foregroundStyle(item.ticket.waits ? Color.orange : item.ticket.urgent ? Color.primary : Color.secondary)
                    .lineLimit(1)
            }
        }
        .opacity(item.stale ? 0.6 : 1)
    }
}

/// One ticket: its decisions to answer, its body, attempts and lanes,
/// and the project's supervisor session to open. The listing's copy
/// is shown at once; the ticket read alone fills in the rest.
struct TicketDetailView: View {
    @Bindable var model: BrowserModel
    let ref: TicketRef
    @Environment(\.openWindow) private var openWindow
    @State private var busy = false
    @State private var outcome: String?
    @State private var confirmResume = false

    private var listed: UnifiedTicket? { model.store.tickets.first { $0.ref == ref } }
    /// The ticket read alone when it is as new as the listing's copy,
    /// else the listing's, which moves first.
    private var ticket: TicketView? {
        let read = model.ticketDetails[ref]
        guard let summary = listed?.ticket else { return read }
        guard let read, read.updatedMs >= summary.updatedMs else { return summary }
        return read
    }
    private var scope: Scope? { model.store.hosts.first { $0.id == ref.serverID }?.server.scope }
    private var canControl: Bool { scope == .control && !busy && !(listed?.stale ?? true) }

    var body: some View {
        Group {
            if let ticket {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        header(ticket)
                        if scope != .control {
                            Text("This device reads only. Raise its scope on the host.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        if let outcome {
                            Text(outcome).font(.caption).foregroundStyle(.secondary)
                        }
                        ForEach(ticket.pendingDecisions) { decision in
                            DecisionCard(decision: decision, enabled: canControl) { answer, note in
                                run("Answered \(decision.name).") {
                                    await model.decide(ref, decision: decision.id, answer: answer, note: note)
                                }
                            }
                        }
                        if let body = ticket.body, !body.isEmpty {
                            AgentText(text: body)
                        }
                        if !ticket.attempts.isEmpty {
                            attempts(ticket)
                        }
                        if !ticket.lanes.isEmpty {
                            lanes(ticket)
                        }
                        answered(ticket)
                    }
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else {
                Text("The ticket is no longer listed.").foregroundStyle(.secondary)
            }
        }
        .navigationTitle(ticket.map { "\($0.source) \($0.title)".trimmingCharacters(in: .whitespaces) } ?? "Ticket")
        .task(id: ref) { await model.loadTicket(ref) }
        .onChange(of: listed?.ticket.updatedMs) { _, _ in Task { await model.loadTicket(ref) } }
        .confirmationDialog("Resume this ticket?", isPresented: $confirmResume) {
            Button("Resume") { run("Resumed.") { await model.resumeTicket(ref) } }
        } message: {
            Text("Dispatch starts its stages again where it was parked.")
        }
    }

    private func header(_ ticket: TicketView) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(ticket.project).font(.subheadline)
                if let stage = ticket.stage {
                    Text("· \(stage)").font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer()
                if let item = listed {
                    SourceLabel(name: item.serverName, stale: item.stale, size: .small)
                }
            }
            Text(ticket.standing)
                .font(.subheadline)
                .foregroundStyle(ticket.waits ? Color.orange : Color.secondary)
            if !ticket.stages.isEmpty {
                Text(ticket.stages.joined(separator: " › ")).font(.caption2).foregroundStyle(.secondary)
            }
            HStack(spacing: 8) {
                if ticket.state == "parked" {
                    Button("Resume…") { confirmResume = true }
                        .buttonStyle(.borderedProminent).controlSize(.small).disabled(!canControl)
                }
                if let supervisor = listed?.supervisor {
                    Button("Supervisor") { openWindow(value: model.windowRequest(for: supervisor)) }
                        .buttonStyle(.bordered).controlSize(.small)
                        .disabled(!model.store.sessions.contains { $0.ref == supervisor })
                        .help("The project's supervisor session, when the host lists it")
                }
                if let url = ticket.url.flatMap(URL.init) {
                    Link("Issue", destination: url).buttonStyle(.bordered).controlSize(.small)
                }
            }
        }
    }

    private func attempts(_ ticket: TicketView) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Attempts").font(.caption).foregroundStyle(.secondary)
            ForEach(ticket.attempts) { attempt in
                HStack(spacing: 8) {
                    Text("\(attempt.stage) \(attempt.n)").font(.caption.monospacedDigit())
                    Text(attempt.context).font(.caption).foregroundStyle(.secondary)
                    Text(attempt.state).font(.caption)
                        .foregroundStyle(attempt.state == "failed" ? Color.red : Color.secondary)
                    if let reason = attempt.reason {
                        Text(reason).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                    }
                    if let pr = attempt.pr, let url = URL(string: pr.url) {
                        Link("PR #\(pr.number) \(pr.checks)", destination: url).font(.caption)
                    }
                    Spacer()
                    if let session = attempt.session {
                        Text(String(session.prefix(8))).font(.caption2.monospaced()).foregroundStyle(.tertiary)
                    }
                }
            }
        }
    }

    private func lanes(_ ticket: TicketView) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Lanes").font(.caption).foregroundStyle(.secondary)
            ForEach(ticket.lanes) { lane in
                HStack(spacing: 8) {
                    Text(lane.name).font(.caption)
                    Text(lane.branch).font(.caption.monospaced()).foregroundStyle(.secondary)
                    if let head = lane.head {
                        Text(String(head.prefix(8))).font(.caption2.monospaced()).foregroundStyle(.tertiary)
                    }
                    if !lane.chosen {
                        Text("not chosen").font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func answered(_ ticket: TicketView) -> some View {
        let done = ticket.decisions.filter { !$0.pending }
        if !done.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("Decided").font(.caption).foregroundStyle(.secondary)
                ForEach(done) { d in
                    HStack(spacing: 8) {
                        Text(d.name).font(.caption)
                        Text(d.answer ?? d.state).font(.caption).foregroundStyle(.secondary)
                        if let note = d.note, !note.isEmpty {
                            Text(note).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
            }
        }
    }

    private func run(_ done: String, _ action: @escaping () async -> CommandOutcome) {
        busy = true
        outcome = nil
        Task {
            let result = await action()
            switch result {
            case .accepted: outcome = done
            case .refused(let why): outcome = "Refused: \(why)"
            case .interrupted: outcome = "Sent; the ticket will say what happened."
            case .unanswered(let why): outcome = "No answer: \(why)"
            }
            busy = false
        }
    }
}

/// A pending decision: its question, one button per option with the
/// recommended one filled, a note field, and for a decision that takes
/// several options a toggle each and one Answer button.
struct DecisionCard: View {
    let decision: TicketDecisionView
    let enabled: Bool
    let answer: (String, String?) -> Void
    @State private var note = ""
    @State private var chosen: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("\(decision.stage) · \(decision.name)").font(.caption).foregroundStyle(.secondary)
            Text(decision.question).font(.body)
            if decision.multiple {
                ForEach(decision.options, id: \.self) { option in
                    Toggle(option, isOn: toggle(option))
                }
                Button("Answer") { answer(decision.options.filter(chosen.contains).joined(separator: ","), trimmed) }
                    .buttonStyle(.borderedProminent).controlSize(.small)
                    .disabled(!enabled || chosen.isEmpty)
            } else {
                HStack(spacing: 8) {
                    ForEach(decision.options, id: \.self) { option in
                        let recommended = decision.recommendation == option
                        Button(option) { answer(option, trimmed) }
                            .buttonStyle(.bordered)
                            .tint(recommended ? .accentColor : nil)
                            .controlSize(.small)
                            .disabled(!enabled || needsNote(option))
                    }
                }
            }
            TextField("Note (needed for some answers)", text: $note, axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(1...4)
        }
        .padding(12)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
        .onAppear {
            if decision.multiple, let r = decision.recommendation {
                chosen = Set(r.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) })
            }
        }
    }

    private var trimmed: String? {
        let text = note.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    private func needsNote(_ option: String) -> Bool {
        decision.needsNote.contains(option) && trimmed == nil
    }

    private func toggle(_ option: String) -> Binding<Bool> {
        Binding(
            get: { chosen.contains(option) },
            set: { on in if on { chosen.insert(option) } else { chosen.remove(option) } })
    }
}
