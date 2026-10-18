import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// Dispatch's tickets from every connected host, filtered and ordered
/// as the toolbar's menu says (last moved first by default); a ticket
/// opens on its decisions, body, attempts with their files, lanes and
/// timeline.
public struct TicketsView: View {
    @Bindable var model: BrowserModel

    public init(model: BrowserModel) {
        self.model = model
    }

    public var body: some View {
        Group {
            if model.tickets.isEmpty {
                VStack {
                    Spacer()
                    Text(empty).foregroundStyle(.secondary).multilineTextAlignment(.center)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            } else {
                List {
                    ForEach(model.tickets) { item in
                        NavigationLink(value: item.ref) {
                            TicketRow(item: item, showHost: model.ticketFilterChoices.hosts.count > 1)
                        }
                    }
                }
            }
        }
        .toolbar {
            ToolbarItemGroup {
                TicketFilterMenu(model: model).equatable()
            }
        }
        .navigationDestination(for: TicketRef.self) { ref in
            TicketDetailView(model: model, ref: ref)
        }
        .navigationDestination(for: TicketFileRef.self) { ref in
            TicketFilePage(model: model, ref: ref)
        }
    }

    private var empty: String {
        if model.store.dispatchHosts.isEmpty {
            return "No connected host runs Dispatch."
        }
        if !model.store.tickets.isEmpty {
            return "No ticket matches the filters."
        }
        return "No tickets."
    }
}

/// A file of a ticket, as a navigation value.
public struct TicketFileRef: Hashable, Sendable {
    public let ticket: TicketRef
    public let key: String

    public init(ticket: TicketRef, key: String) {
        self.ticket = ticket
        self.key = key
    }
}

/// The order and filters of the ticket list. It reads only the choices
/// and the filters, never the store, as `FilterMenu` does: a menu
/// rebuilt while open scrolls back to its top.
struct TicketFilterMenu: View, Equatable {
    @Bindable var model: BrowserModel

    static func == (lhs: TicketFilterMenu, rhs: TicketFilterMenu) -> Bool {
        lhs.model.ticketFilterChoices == rhs.model.ticketFilterChoices
            && lhs.model.ticketFilters == rhs.model.ticketFilters
    }

    var body: some View {
        Menu {
            Picker("Order", selection: Binding(
                get: { model.ticketFilters.sort },
                set: { model.ticketFilters.sort = $0 })
            ) {
                ForEach(TicketSort.allCases, id: \.self) { sort in
                    Text(sort.label).tag(sort)
                }
            }
            .pickerStyle(.inline)
            Toggle("Waiting on me", isOn: Binding(
                get: { model.ticketFilters.waitingOnly },
                set: { model.ticketFilters.waitingOnly = $0 }))
            if model.ticketFilterChoices.hosts.count > 1 {
                Section("Hosts") {
                    ForEach(model.ticketFilterChoices.hosts) { host in
                        Toggle(host.name, isOn: binding(\.servers, host.id, all: model.ticketFilterChoices.hosts.map(\.id)))
                    }
                }
            }
            if model.ticketFilterChoices.projects.count > 1 {
                Section("Projects") {
                    let projects = model.ticketFilterChoices.projects
                    ForEach(projects, id: \.self) { project in
                        Toggle(project, isOn: binding(\.projects, project, all: projects))
                    }
                }
            }
            Section("States") {
                let states = model.ticketFilterChoices.states
                ForEach(states, id: \.self) { state in
                    Toggle(state, isOn: stateBinding(state, all: states))
                }
            }
            if model.ticketFilters != TicketFilters() {
                Button("Clear filters") { model.ticketFilters = TicketFilters() }
            }
        } label: {
            Label("Filter", systemImage: model.ticketFilters == TicketFilters() ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
        }
    }

    /// An empty set admits everything, so turning one member off from
    /// there selects all the others first.
    private func binding<T: Hashable>(_ path: WritableKeyPath<TicketFilters, Set<T>>, _ member: T, all: [T]) -> Binding<Bool> {
        Binding(
            get: { model.ticketFilters[keyPath: path].isEmpty || model.ticketFilters[keyPath: path].contains(member) },
            set: { on in
                var filters = model.ticketFilters
                if on {
                    filters[keyPath: path].insert(member)
                    if filters[keyPath: path].count == all.count { filters[keyPath: path] = [] }
                } else {
                    if filters[keyPath: path].isEmpty { filters[keyPath: path] = Set(all) }
                    filters[keyPath: path].remove(member)
                }
                model.ticketFilters = filters
            })
    }

    /// States: the empty set means every state but closed, so closed
    /// starts off and the others on.
    private func stateBinding(_ state: String, all: [String]) -> Binding<Bool> {
        Binding(
            get: {
                let states = model.ticketFilters.states
                return states.isEmpty ? state != "closed" : states.contains(state)
            },
            set: { on in
                var filters = model.ticketFilters
                if filters.states.isEmpty { filters.states = Set(all.filter { $0 != "closed" }) }
                if on { filters.states.insert(state) } else { filters.states.remove(state) }
                if filters.states == Set(all.filter { $0 != "closed" }) { filters.states = [] }
                model.ticketFilters = filters
            })
    }
}

/// One ticket as the list shows it: source and title, the project,
/// where it stands, and how long since it moved.
struct TicketRow: View {
    let item: UnifiedTicket
    var showHost = true

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                if !item.ticket.source.isEmpty {
                    Text(item.ticket.source).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                Text(item.ticket.title).font(.headline).lineLimit(2)
                Spacer()
                if showHost {
                    SourceLabel(name: item.serverName, stale: item.stale, size: .small)
                }
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
                Spacer()
                Text(Ago.text(item.ticket.updatedMs)).font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
            }
        }
        .opacity(item.stale ? 0.6 : 1)
    }
}

/// One ticket: its decisions to answer, its body, attempts with their
/// files, lanes, timeline, and the project's supervisor session to
/// open. The listing's copy is shown at once; the ticket read alone
/// fills in the rest.
struct TicketDetailView: View {
    @Bindable var model: BrowserModel
    let ref: TicketRef
    @Environment(\.openWindow) private var openWindow
    @State private var busy = false
    @State private var outcome: String?
    @State private var confirmResume = false
    @State private var confirmClose = false
    @State private var timelineOpen = false

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
    /// The host serves files, events and close.
    private var evidence: Bool { model.evidenceHosts.contains(ref.serverID) }

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
                        if evidence {
                            timeline
                        }
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
        .task(id: listed?.ticket.updatedMs) {
            await model.loadTicket(ref)
            if timelineOpen { await model.loadTicketEvents(ref) }
        }
        .onChange(of: timelineOpen) { _, open in
            if open { Task { await model.loadTicketEvents(ref) } }
        }
        .confirmationDialog("Resume this ticket?", isPresented: $confirmResume) {
            Button("Resume") { run("Resumed.") { await model.resumeTicket(ref) } }
        } message: {
            Text("Dispatch starts its stages again where it was parked.")
        }
        .confirmationDialog("Close this ticket?", isPresented: $confirmClose) {
            Button("Close", role: .destructive) { run("Closing.") { await model.closeTicket(ref) } }
        } message: {
            Text("Its worktrees are removed; its branches and record are kept.")
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
                if let notes = latestNotes(ticket) {
                    NavigationLink(value: TicketFileRef(ticket: ref, key: notes.key)) {
                        Label("Notes", systemImage: "note.text")
                    }
                    .buttonStyle(.bordered).controlSize(.small)
                    .help("The newest notes: \(notes.stage) \(notes.n)")
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
                if ticket.closable && evidence {
                    Button("Close…", role: .destructive) { confirmClose = true }
                        .buttonStyle(.bordered).controlSize(.small).disabled(!canControl)
                }
            }
        }
    }

    /// The notes of the newest attempt that wrote any.
    private func latestNotes(_ ticket: TicketView) -> TicketFileView? {
        guard evidence else { return nil }
        for attempt in ticket.attempts.reversed() {
            if let notes = attempt.files.first(where: { $0.name == "notes" && $0.inline }) {
                return notes
            }
        }
        return nil
    }

    private func attempts(_ ticket: TicketView) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Attempts").font(.caption).foregroundStyle(.secondary)
            ForEach(ticket.attempts) { attempt in
                AttemptRow(attempt: attempt, ticket: ref, files: evidence)
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
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(d.name).font(.caption)
                        Text(d.answer ?? d.state).font(.caption).foregroundStyle(.secondary)
                        if let note = d.note, !note.isEmpty {
                            Text(note).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
                        }
                    }
                }
            }
        }
    }

    /// The runner's events about the ticket, newest last; read when
    /// opened and again whenever the ticket moves.
    private var timeline: some View {
        DisclosureGroup(isExpanded: $timelineOpen) {
            if let events = model.ticketEvents[ref] {
                if events.unavailable {
                    Text("The host's runner does not serve events.").font(.caption).foregroundStyle(.secondary)
                } else if events.events.isEmpty {
                    Text("Nothing yet.").font(.caption).foregroundStyle(.secondary)
                } else {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(events.events) { event in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text(Ago.text(event.atMs)).font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
                                    .frame(width: 32, alignment: .trailing)
                                Text(event.stage).font(.caption2).foregroundStyle(.secondary)
                                Text(event.text).font(.caption).lineLimit(3)
                            }
                        }
                    }
                    .padding(.top, 4)
                }
            } else {
                ProgressView().controlSize(.small)
            }
        } label: {
            Text("Timeline").font(.caption).foregroundStyle(.secondary)
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

/// One attempt: stage, context and state on a line, then its checks,
/// its review rounds, and a button per file the port reads.
struct AttemptRow: View {
    let attempt: TicketAttemptView
    let ticket: TicketRef
    /// The host serves the files; without that they are only named.
    let files: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text("\(attempt.stage) \(attempt.n)").font(.caption.monospacedDigit())
                Text(attempt.context).font(.caption).foregroundStyle(.secondary)
                Text(attempt.state).font(.caption)
                    .foregroundStyle(attempt.state == "failed" ? Color.red : Color.secondary)
                if let reason = attempt.reason {
                    Text(reason).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
                }
                if let pr = attempt.pr, let url = URL(string: pr.url) {
                    Link("PR #\(pr.number) \(pr.checks)", destination: url).font(.caption)
                }
                Spacer()
                Text(Ago.text(attempt.endedMs ?? attempt.startedMs)).font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
            }
            if let checks = attempt.checks {
                Text(checksText(checks)).font(.caption2).foregroundStyle(checks.exit == 0 || checks.exit == nil ? Color.secondary : Color.red)
            }
            ForEach(attempt.rounds) { round in
                Text("round \(round.n): \(round.state), \(round.openPoints) open").font(.caption2).foregroundStyle(.secondary)
            }
            if let swept = attempt.evidenceSweptMs {
                Text("evidence swept \(Ago.text(swept)) ago").font(.caption2).foregroundStyle(.secondary)
            }
            if !attempt.files.isEmpty {
                FlowStack(spacing: 6) {
                    ForEach(attempt.files) { file in
                        if files && file.readable {
                            NavigationLink(value: TicketFileRef(ticket: ticket, key: file.key)) {
                                Label(file.name, systemImage: icon(file))
                                    .font(.caption)
                                    .lineLimit(1)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.mini)
                        } else {
                            Label(file.name, systemImage: icon(file))
                                .font(.caption)
                                .foregroundStyle(.tertiary)
                                .lineLimit(1)
                                .help(file.kind == "secret" ? "secret; Dispatch never reads it" : "open it on the host")
                        }
                    }
                }
            }
        }
        .padding(.vertical, 2)
    }

    private func checksText(_ checks: TicketChecksView) -> String {
        let head = String(checks.head.prefix(8))
        switch checks.exit {
        case nil: return "checks running at \(head)"
        case 0: return "checks passed at \(head)"
        case let code?: return "checks failed at \(head), exit \(code)"
        }
    }

    private func icon(_ file: TicketFileView) -> String {
        if file.image { return "photo" }
        switch file.kind {
        case "secret": return "lock"
        case "checks": return "checkmark.circle"
        case "findings": return "magnifyingglass"
        case "response": return "arrowshape.turn.up.left"
        case "evidence": return file.inline ? "doc.plaintext" : "doc"
        default: return "doc.text"
        }
    }
}

/// Views laid out in rows, wrapping at the width given.
struct FlowStack: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 400
        return place(width: width, subviews: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let placed = place(width: bounds.width, subviews: subviews)
        for (index, origin) in placed.origins.enumerated() {
            subviews[index].place(
                at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y),
                proposal: .unspecified)
        }
    }

    private func place(width: CGFloat, subviews: Subviews) -> (size: CGSize, origins: [CGPoint]) {
        var origins: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var widest: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            widest = max(widest, x - spacing)
        }
        return (CGSize(width: widest, height: y + rowHeight), origins)
    }
}

/// One file of a ticket, read when shown: notes and other markdown as
/// the agent's text, any other text in a monospaced block, an image
/// as a picture.
struct TicketFilePage: View {
    @Bindable var model: BrowserModel
    let ref: TicketFileRef
    @State private var file: TicketFileBody?
    @State private var problem: String?

    private var named: TicketFileMeta? {
        let ticket = model.ticketDetails[ref.ticket] ?? model.store.tickets.first { $0.ref == ref.ticket }?.ticket
        for attempt in ticket?.attempts ?? [] {
            if let f = attempt.files.first(where: { $0.key == ref.key }) {
                return TicketFileMeta(name: f.name, stage: attempt.stage, n: attempt.n, image: f.image)
            }
        }
        return nil
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let named {
                    Text("\(named.stage) \(named.n)").font(.caption).foregroundStyle(.secondary)
                }
                if let problem {
                    Text(problem).foregroundStyle(.secondary)
                } else if let file {
                    content(file)
                } else {
                    ProgressView()
                }
            }
            .padding()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(named?.name ?? "File")
        .task(id: ref) {
            file = nil
            problem = nil
            do {
                file = try await model.readTicketFile(ref.ticket, key: ref.key)
            } catch {
                problem = "\(error)"
            }
        }
        .toolbar {
            Button {
                Task {
                    file = nil
                    problem = nil
                    do { file = try await model.readTicketFile(ref.ticket, key: ref.key) } catch { problem = "\(error)" }
                }
            } label: {
                Label("Read again", systemImage: "arrow.clockwise")
            }
        }
    }

    /// An artifact (named without an extension: notes, plan, summary, a
    /// round's findings) and any `.md` file are the agent's markdown.
    private var markdown: Bool {
        guard let name = named?.name.lowercased() else { return true }
        return !name.contains(".") || name.hasSuffix(".md")
    }

    @ViewBuilder
    private func content(_ file: TicketFileBody) -> some View {
        if let text = file.text {
            if text.isEmpty {
                Text("Empty.").foregroundStyle(.secondary)
            } else if markdown {
                AgentText(text: text)
            } else {
                Text(text).font(.body.monospaced()).textSelection(.enabled)
            }
        } else if let data = file.imageData, let image = PlatformImage.make(data) {
            image.resizable().aspectRatio(contentMode: .fit)
        } else {
            Text("Nothing to show.").foregroundStyle(.secondary)
        }
    }
}

/// What the file view knows of its file from the ticket's listing.
struct TicketFileMeta {
    let name: String
    let stage: String
    let n: UInt32
    let image: Bool
}

/// An image from bytes, on whichever UI framework the build has.
enum PlatformImage {
    static func make(_ data: Data) -> Image? {
        #if canImport(UIKit)
        guard let image = UIImage(data: data) else { return nil }
        return Image(uiImage: image)
        #elseif canImport(AppKit)
        guard let image = NSImage(data: data) else { return nil }
        return Image(nsImage: image)
        #else
        return nil
        #endif
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
