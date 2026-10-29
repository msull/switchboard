import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol
import SwitchboardTransport

/// The shared read model the views draw from, refreshed from the
/// manager's store on the main actor.
@MainActor
@Observable
public final class BrowserModel {
    public let manager: ConnectionManager
    public private(set) var store = UnifiedStore()
    public private(set) var discovered: [DiscoveredServerRow] = []
    /// The filter menu's choices, replaced only when they differ, so
    /// the menu's body is not re-evaluated by the two-second pull.
    public private(set) var filterChoices = FilterChoices()
    /// The sessions a board's add menu offers, by host: replaced only
    /// when a name, a host or the set of sessions changes, never on a
    /// pull that moved only a card's text, so an open menu keeps its
    /// place.
    public private(set) var sessionChoices: [SessionChoiceGroup] = []
    public private(set) var ticketFilterChoices = TicketFilterChoices()
    /// The hosts that serve a ticket's files, events and close.
    public private(set) var evidenceHosts: Set<ServerID> = []
    /// The hosts a session can be opened on from here.
    public private(set) var createHosts: Set<ServerID> = []
    /// Each ticket's events as last read, by ref.
    public private(set) var ticketEvents: [TicketRef: TicketEventsBody] = [:]
    /// The card windows on screen, by window id, so a click on a listed
    /// session reaches its window. Nothing here outlives the process:
    /// a launch brings back the main window and no other.
    public private(set) var open: [UUID: SessionRef] = [:]
    /// The screen last read per watched session, mirrored from its
    /// connection; the holders per session are the open screen views.
    public private(set) var screens: [SessionRef: ScreenBody] = [:]
    private var holders: [SessionRef: Int] = [:]
    /// The conversation content per watched session: the last response
    /// and the turns held; the holders are the open response and
    /// conversation views.
    public private(set) var lastResponses: [SessionRef: LastResponseBody] = [:]
    public private(set) var conversations: [SessionRef: ConversationState] = [:]
    private var conversationHolders: [SessionRef: Int] = [:]
    /// The files open in file windows, as the host last served them.
    public private(set) var files: [FileRef: FileBody] = [:]
    private var fileHolders: [FileRef: Int] = [:]
    /// The file windows on screen, by window id.
    public private(set) var openFiles: [UUID: FileRef] = [:]
    /// The line a file window was last asked to show, numbered so a
    /// second ask for the same line scrolls again.
    public private(set) var lineRequests: [FileRef: LineRequest] = [:]
    /// The runs read per session, and the output per run watched.
    public private(set) var runs: [SessionRef: [RunView]] = [:]
    public private(set) var runOutputs: [RunRef: RunOutputBody] = [:]
    private var runHolders: [RunRef: Int] = [:]
    /// The draft per session a control bar or the prompt window holds:
    /// the host's editor as last read (§4.5), and the hosts that keep
    /// one; a host without the capability leaves the bar its own field.
    public private(set) var drafts: [SessionRef: DraftView] = [:]
    private var draftHolders: [SessionRef: Int] = [:]
    public private(set) var draftHosts: Set<ServerID> = []
    /// The device's own field per session, shared by the card's control
    /// bar and the keys panel; made on first use and kept while the app
    /// runs, so what a closed card held is there when it reopens.
    public private(set) var fields: [SessionRef: EditField] = [:]
    /// Tickets read alone, by ref: the body, attempts and lanes the
    /// listing leaves out.
    public private(set) var ticketDetails: [TicketRef: TicketView] = [:]
    /// The session whose input is active: the last one a control bar
    /// typed or dictated into, which the prompt window follows.
    public private(set) var activeInput: SessionRef?
    /// How many turns a conversation page asks for.
    public static let conversationPage = 20
    /// Every session seen from a host, and the ones a connected host
    /// no longer lists: a window on one says it was removed.
    private var seen: Set<SessionRef> = []
    public private(set) var removed: Set<SessionRef> = []
    /// The client's own presentation state, kept in its container.
    public private(set) var presentation = PresentationState()
    private let presentationStore: PresentationStore
    private var pump: Task<Void, Never>?
    private var booting: Task<Void, Never>?

    public init(manager: ConnectionManager, presentationStore: PresentationStore = MemoryPresentationStore()) {
        self.manager = manager
        self.presentationStore = presentationStore
        presentation = (try? presentationStore.load()) ?? PresentationState()
    }

    /// Mirror the manager's store every `interval` seconds.
    public func start(interval: TimeInterval = 2) {
        pump?.cancel()
        pump = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.pull()
                try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
            }
        }
    }

    public func stop() {
        pump?.cancel()
    }

    /// Load the registry, pair with each invite given (a dev aid, one
    /// per line), start the manager's loop with Bonjour, and mirror
    /// its store. Every scene calls this when it appears, since any
    /// of them may be the only one restored; the first call does the
    /// work and the others wait for it.
    public func boot(invite: String? = nil) async {
        if let booting {
            await booting.value
            return
        }
        let task = Task { await self.bootOnce(invite: invite) }
        booting = task
        await task.value
    }

    private func bootOnce(invite: String?) async {
        try? await manager.load()
        for line in (invite ?? "").split(separator: "\n") {
            if let parsed = try? PairingInvite.parse(String(line)) {
                try? await manager.pair(invite: parsed)
            }
        }
        await manager.setOnChange { [weak self] in
            Task { @MainActor in await self?.pull() }
        }
        await manager.start()
        start()
    }

    /// A discovered host's `/info` and address, for the pairing sheet.
    public func lookup(_ instance: String) async throws -> HostLookup {
        guard let record = await manager.discovered.first(where: { $0.instance == instance }) else {
            throw RemoteError.transport("\(instance) is no longer on the network")
        }
        return try await manager.lookup(record)
    }

    /// Pair with a host looked up, by the code typed from its dialog.
    public func pair(_ lookup: HostLookup, code: String) async throws {
        try await manager.pair(lookup, code: code)
        await pull()
    }

    public func pull() async {
        store = await manager.store
        let choices = FilterChoices(store: store)
        if choices != filterChoices { filterChoices = choices }
        let sessions = SessionChoiceGroup.groups(store.sessions)
        if sessions != sessionChoices { sessionChoices = sessions }
        let ticketChoices = TicketFilterChoices(store: store)
        if ticketChoices != ticketFilterChoices { ticketFilterChoices = ticketChoices }
        let evidence = await manager.evidenceHosts
        if evidence != evidenceHosts { evidenceHosts = evidence }
        let create = await manager.createHosts
        if create != createHosts { createHosts = create }
        discovered = await manager.discovered.map {
            DiscoveredServerRow(instance: $0.instance, serverID: $0.serverID, name: $0.name)
        }.sorted { $0.instance < $1.instance }
        var screens: [SessionRef: ScreenBody] = [:]
        for ref in holders.keys {
            screens[ref] = await manager.screen(for: ref)
        }
        self.screens = screens
        var lastResponses: [SessionRef: LastResponseBody] = [:]
        var conversations: [SessionRef: ConversationState] = [:]
        var runs: [SessionRef: [RunView]] = [:]
        for ref in conversationHolders.keys {
            lastResponses[ref] = await manager.lastResponse(for: ref)
            conversations[ref] = await manager.conversation(for: ref)
        }
        for ref in Set(runHolders.keys.map(\.session)).union(self.runs.keys) {
            runs[ref] = await manager.runs(for: ref)
        }
        var runOutputs: [RunRef: RunOutputBody] = [:]
        for ref in runHolders.keys {
            runOutputs[ref] = await manager.runOutput(for: ref)
        }
        self.lastResponses = lastResponses
        self.conversations = conversations
        self.runs = runs
        self.runOutputs = runOutputs
        var files: [FileRef: FileBody] = [:]
        for ref in fileHolders.keys {
            files[ref] = await manager.file(for: ref)
        }
        self.files = files
        var drafts: [SessionRef: DraftView] = [:]
        for ref in draftHolders.keys {
            drafts[ref] = await manager.draft(for: ref)
        }
        self.drafts = drafts
        for (ref, draft) in drafts {
            if let field = fields[ref], !field.editing { field.takeHost(draft.text) }
        }
        var draftHosts: Set<ServerID> = []
        for host in store.hosts where await manager.hasDraft(host.id) {
            draftHosts.insert(host.id)
        }
        self.draftHosts = draftHosts
        let present = Set(store.sessions.map(\.ref))
        let connected = Set(store.hosts.filter { $0.state.isConnected }.map(\.id))
        seen.formUnion(present)
        removed = seen.subtracting(present).filter { connected.contains($0.serverID) }
    }

    /// A screen view opened on `ref`: the first holder takes the watch.
    public func watch(_ ref: SessionRef) async {
        holders[ref, default: 0] += 1
        if holders[ref] == 1 {
            await manager.watch(screen: ref)
        }
        await pull()
    }

    /// A screen view closed: the last holder releases the watch.
    public func unwatch(_ ref: SessionRef) async {
        guard let n = holders[ref] else { return }
        if n > 1 {
            holders[ref] = n - 1
            return
        }
        holders[ref] = nil
        screens[ref] = nil
        await manager.unwatch(screen: ref)
    }

    /// A response or conversation view opened on `ref`: the first
    /// holder takes the watch and reads both the last response and the
    /// last page, so either view has what it shows.
    public func watchConversation(_ ref: SessionRef) async {
        conversationHolders[ref, default: 0] += 1
        if conversationHolders[ref] == 1 {
            await manager.watch(conversation: ref)
            await manager.readLastResponse(ref)
            await manager.readConversation(ref, from: nil, limit: Self.conversationPage)
        }
        await pull()
    }

    public func unwatchConversation(_ ref: SessionRef) async {
        guard let n = conversationHolders[ref] else { return }
        if n > 1 {
            conversationHolders[ref] = n - 1
            return
        }
        conversationHolders[ref] = nil
        lastResponses[ref] = nil
        conversations[ref] = nil
        await manager.unwatch(conversation: ref)
    }

    /// The page before the first turn held.
    public func loadEarlier(_ ref: SessionRef) async {
        guard let first = conversations[ref]?.firstHeld, first > 1 else { return }
        await manager.readConversation(ref, before: first, limit: Self.conversationPage)
        await pull()
    }

    // MARK: The draft (§4.5)

    /// Whether the session's host keeps a draft for it.
    public func hasDraft(_ ref: SessionRef) -> Bool {
        draftHosts.contains(ref.serverID)
    }

    /// A control bar or the prompt window opened on `ref`: the first
    /// holder reads the draft; `changed` keeps it current after that.
    public func holdDraft(_ ref: SessionRef) async {
        draftHolders[ref, default: 0] += 1
        if draftHolders[ref] == 1 {
            await manager.readDraft(ref)
        }
        await pull()
    }

    public func releaseDraft(_ ref: SessionRef) async {
        guard let n = draftHolders[ref] else { return }
        if n > 1 {
            draftHolders[ref] = n - 1
            return
        }
        draftHolders[ref] = nil
        drafts[ref] = nil
    }

    /// Input went into a session: typing, dictation or a tool.
    public func inputActivated(_ ref: SessionRef) {
        activeInput = ref
    }

    private func took(_ ref: SessionRef, _ outcome: DraftOutcome) {
        if case .applied(let draft) = outcome, draftHolders[ref] != nil {
            drafts[ref] = draft
            if let field = fields[ref], !field.editing { field.takeHost(draft.text) }
        }
    }

    // MARK: The field

    public func field(_ ref: SessionRef) -> EditField {
        if let field = fields[ref] { return field }
        let field = EditField()
        fields[ref] = field
        return field
    }

    /// The field's text changed by hand on a host with a draft: sent
    /// whole once the typing pauses, as the editor's replace. The
    /// host's reply is the text now; a newer edit outranks it.
    public func fieldEdited(_ ref: SessionRef) {
        let field = field(ref)
        guard hasDraft(ref), field.text != field.hostText else { return }
        field.edit += 1
        let mine = field.edit
        Task {
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard mine == field.edit else { return }
            let outcome = await replaceDraft(ref, text: field.text)
            guard mine == field.edit else { return }
            field.settled = mine
            switch outcome {
            case .applied(let host): field.takeHost(host.text)
            default: field.note = outcome.note
            }
        }
    }

    /// Typing that has not reached the host yet, sent before a send.
    public func flushField(_ ref: SessionRef) async {
        let field = field(ref)
        guard hasDraft(ref), field.text != field.hostText else { return }
        field.edit += 1
        field.settled = field.edit
        _ = await replaceDraft(ref, text: field.text)
    }

    /// A key of the keys panel pressed on a session's field.
    public func press(_ key: PanelKey, on ref: SessionRef) {
        field(ref).press(key)
        inputActivated(ref)
        fieldEdited(ref)
    }

    public func draftAction(_ ref: SessionRef, _ tool: DraftTool) async -> DraftOutcome {
        inputActivated(ref)
        let outcome = await manager.draftAction(ref, tool)
        took(ref, outcome)
        return outcome
    }

    public func replaceDraft(_ ref: SessionRef, text: String) async -> DraftOutcome {
        inputActivated(ref)
        let outcome = await manager.replaceDraft(ref, text: text)
        took(ref, outcome)
        return outcome
    }

    public func sendDraft(_ ref: SessionRef) async -> DraftOutcome {
        let outcome = await manager.sendDraft(ref)
        took(ref, outcome)
        await pull()
        return outcome
    }

    public func dictate(_ ref: SessionRef, recognizer: UInt64, events: [DictationEvent]) async -> DraftOutcome {
        inputActivated(ref)
        let outcome = await manager.dictate(ref, recognizer: recognizer, events: events)
        took(ref, outcome)
        return outcome
    }

    /// The device's scope on a session's host, which says whether the
    /// window's controls can do anything.
    public func scope(for ref: SessionRef) -> Scope? {
        store.hosts.first { $0.id == ref.serverID }?.server.scope
    }

    public func send(_ ref: SessionRef, text: String) async -> CommandOutcome {
        let outcome = await manager.send(ref, text: text)
        await pull()
        return outcome
    }

    /// A host's projects and workspaces, for the new-session sheet.
    public func catalog(for host: ServerID) async -> HostCatalog? {
        await manager.catalog(for: host)
    }

    /// Open a session in a project on a host; the store is pulled
    /// after, so an opened one is listed for its window.
    public func newSession(
        on host: ServerID, project: String, name: String, kind: SessionKind, cwd: String?, prompt: String?
    ) async -> NewSessionOutcome {
        let outcome = await manager.newSession(
            on: host, project: project, name: name, kind: kind, cwd: cwd, prompt: prompt)
        await pull()
        return outcome
    }

    public func resume(_ ref: SessionRef) async -> CommandOutcome {
        let outcome = await manager.resume(ref)
        await pull()
        return outcome
    }

    public func trust(_ ref: SessionRef) async -> CommandOutcome {
        let outcome = await manager.trust(ref)
        await pull()
        return outcome
    }

    /// One ticket read alone, for its detail view.
    public func loadTicket(_ ref: TicketRef) async {
        if let ticket = await manager.readTicket(ref) {
            ticketDetails[ref] = ticket
        }
    }

    public func decide(_ ref: TicketRef, decision: String, answer: String, note: String?) async -> CommandOutcome {
        let outcome = await manager.decide(ref, decision: decision, answer: answer, note: note)
        await pull()
        await loadTicket(ref)
        return outcome
    }

    public func resumeTicket(_ ref: TicketRef) async -> CommandOutcome {
        let outcome = await manager.resumeTicket(ref)
        await pull()
        await loadTicket(ref)
        return outcome
    }

    public func closeTicket(_ ref: TicketRef) async -> CommandOutcome {
        let outcome = await manager.closeTicket(ref)
        await pull()
        await loadTicket(ref)
        return outcome
    }

    /// A ticket's events, read again whole; the host serves them from
    /// its own cache while the ticket has not moved.
    public func loadTicketEvents(_ ref: TicketRef) async {
        guard evidenceHosts.contains(ref.serverID) else { return }
        if let events = try? await manager.readTicketEvents(ref) {
            ticketEvents[ref] = events
        }
    }

    /// One file of a ticket by its key.
    public func readTicketFile(_ ref: TicketRef, key: String) async throws -> TicketFileBody {
        try await manager.readTicketFile(ref, key: key)
    }

    /// The ticket list as filtered and ordered.
    public var tickets: [UnifiedTicket] {
        presentation.ticketFilters.apply(store.tickets)
    }

    public var ticketFilters: TicketFilters {
        get { presentation.ticketFilters }
        set {
            presentation.ticketFilters = newValue
            save()
        }
    }

    /// A turn whole, for a window that opened its activity: the page
    /// carried only the head of each tool call's text.
    public func loadTurn(_ ref: SessionRef, n: Int) async {
        await manager.readTurn(ref, n: n)
        await pull()
    }

    /// The runs of a command or service, read now and kept fresh by
    /// the host's `changed {runs}`.
    public func readRuns(_ ref: SessionRef) async {
        await manager.readRuns(ref)
        runs[ref] = await manager.runs(for: ref)
    }

    /// An output view opened on a run: the first holder takes the
    /// watch and reads the tail.
    public func watchRunOutput(_ ref: RunRef) async {
        runHolders[ref, default: 0] += 1
        if runHolders[ref] == 1 {
            await manager.watch(runOutput: ref)
            await manager.readRunOutput(ref)
        }
        await pull()
    }

    public func unwatchRunOutput(_ ref: RunRef) async {
        guard let n = runHolders[ref] else { return }
        if n > 1 {
            runHolders[ref] = n - 1
            return
        }
        runHolders[ref] = nil
        runOutputs[ref] = nil
        await manager.unwatch(runOutput: ref)
    }

    /// The mode a card window shows, `compact` until it chooses.
    public func mode(for window: UUID) -> ViewMode {
        presentation.mode(for: window) ?? .compact
    }

    public func setMode(_ mode: ViewMode, for window: UUID) {
        presentation.setMode(mode, for: window)
        save()
    }

    /// Where a card window stands in its conversation, if it has been
    /// there before.
    public func place(for window: UUID) -> ConversationPlace? {
        presentation.place(for: window)
    }

    public func setPlace(_ place: ConversationPlace, for window: UUID) {
        presentation.setPlace(place, for: window)
        save()
    }

    public var sessions: [UnifiedSession] {
        store.sessions.filter(presentation.filters.admits)
    }

    /// The main window's filters, kept across launches.
    public var filters: Filters {
        get { presentation.filters }
        set {
            presentation.filters = newValue
            save()
        }
    }

    public var browse: BrowseMode {
        get { presentation.browse }
        set {
            presentation.browse = newValue
            save()
        }
    }

    public var focused: SessionRef? {
        get { presentation.focused }
        set {
            presentation.focused = newValue
            save()
        }
    }

    /// A file window opened on `ref`: the first holder takes the watch,
    /// which reads the file.
    public func watchFile(_ ref: FileRef) async {
        fileHolders[ref, default: 0] += 1
        if fileHolders[ref] == 1 {
            await manager.watch(file: ref)
        }
        await pull()
    }

    public func unwatchFile(_ ref: FileRef) async {
        guard let n = fileHolders[ref] else { return }
        if n > 1 {
            fileHolders[ref] = n - 1
            return
        }
        fileHolders[ref] = nil
        files[ref] = nil
        await manager.unwatch(file: ref)
    }

    /// A file window is on screen.
    public func fileWindowAppeared(_ request: FileWindowRequest) {
        openFiles[request.id] = request.ref
    }

    public func fileWindowClosed(_ request: FileWindowRequest) {
        openFiles[request.id] = nil
    }

    /// The request a tap on a file link opens: the window on that file
    /// when one is on screen, else a new one; and the line to show,
    /// which the window reads from `lineRequests`.
    public func fileWindowRequest(for ref: FileRef, line: Int?) -> FileWindowRequest {
        if let line {
            let n = (lineRequests[ref]?.n ?? 0) + 1
            lineRequests[ref] = LineRequest(line: line, n: n)
        }
        return FileWindowRequest.open(on: ref, among: openFiles) ?? FileWindowRequest(ref: ref)
    }

    // MARK: Boards

    public var boards: [Board] { presentation.boards }
    /// The boards whose windows are on screen.
    public private(set) var openBoards: Set<UUID> = []

    public func boardAppeared(_ id: UUID) { openBoards.insert(id) }
    public func boardClosed(_ id: UUID) { openBoards.remove(id) }

    @discardableResult
    public func addBoard(named name: String) -> Board {
        let board = Board(name: name.isEmpty ? "Board \(presentation.boards.count + 1)" : name)
        presentation.boards.append(board)
        save()
        return board
    }

    public func renameBoard(_ id: UUID, to name: String) {
        let name = name.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        edit(id) { $0.name = name }
    }

    public func removeBoard(_ id: UUID) {
        presentation.boards.removeAll { $0.id == id }
        save()
    }

    /// Put `item` on the board at the next free place; an item already
    /// on it is left where it is.
    public func addTile(_ item: BoardItem, to board: UUID) {
        edit(board) { b in
            guard !b.tiles.contains(where: { $0.item == item }) else { return }
            b.tiles.append(BoardTile(item: item, frame: BoardLayout.place(avoiding: b.tiles.map(\.frame))))
        }
    }

    public func removeTile(_ tile: UUID, from board: UUID) {
        edit(board) { $0.tiles.removeAll { $0.id == tile } }
    }

    public func moveTile(_ tile: UUID, on board: UUID, by translation: CGSize, in size: CGSize) {
        edit(board) { b in
            guard let i = b.tiles.firstIndex(where: { $0.id == tile }) else { return }
            b.tiles[i].frame = BoardLayout.moved(b.tiles[i].frame, by: translation, in: size)
        }
    }

    public func resizeTile(_ tile: UUID, on board: UUID, by translation: CGSize, in size: CGSize) {
        edit(board) { b in
            guard let i = b.tiles.firstIndex(where: { $0.id == tile }) else { return }
            b.tiles[i].frame = BoardLayout.resized(b.tiles[i].frame, by: translation, in: size)
        }
    }

    private func edit(_ board: UUID, _ change: (inout Board) -> Void) {
        guard let i = presentation.boards.firstIndex(where: { $0.id == board }) else { return }
        change(&presentation.boards[i])
        save()
    }

    /// A card window is on screen.
    public func windowAppeared(_ request: CardWindowRequest) {
        open[request.id] = request.ref
    }

    /// A card window went away.
    public func windowClosed(_ request: CardWindowRequest) {
        open[request.id] = nil
    }

    /// The request a click on `ref` opens: its window when one is on
    /// screen, which the window group then brings forward, or a new
    /// one.
    public func windowRequest(for ref: SessionRef) -> CardWindowRequest {
        CardWindowRequest.open(on: ref, among: open) ?? CardWindowRequest(ref: ref)
    }

    private func save() {
        try? presentationStore.save(presentation)
    }
}

public struct DiscoveredServerRow: Identifiable, Sendable, Equatable {
    public let instance: String
    public let serverID: ServerID?
    public let name: String?
    public var id: String { instance }
}

/// Sessions and sets from every connected host, each card labelled
/// with its source and marked when the host is not connected, as a
/// list, a grid or one session in focus; a session opens as a window
/// of its own. Sets are grouped by source (§8).
public struct BrowserView: View {
    @Bindable var model: BrowserModel
    @Environment(\.openWindow) private var openWindow
    /// The new-session sheet, open while this is set.
    @State private var newSession: NewSessionSeed?

    public init(model: BrowserModel) {
        self.model = model
    }

    public var body: some View {
        Group {
            switch model.browse {
            case .list: list
            case .grid: grid
            case .focus: focus
            }
        }
        .toolbar {
            ToolbarItemGroup {
                if !model.createHosts.isEmpty {
                    Button {
                        newSession = NewSessionSeed()
                    } label: {
                        Label("New session", systemImage: "plus")
                    }
                    .help("Open a session in a project on a host")
                }
                Picker("Layout", selection: $model.browse) {
                    ForEach(BrowseMode.allCases) { mode in
                        Label(mode.title, systemImage: mode.symbol).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
                FilterMenu(model: model).equatable()
            }
        }
        .sheet(item: $newSession) { seed in
            NewSessionSheet(model: model, seed: seed)
        }
    }

    /// The listed session's menu: another window on it, and a new
    /// session beside it when its host opens one from here.
    @ViewBuilder
    private func menu(for item: UnifiedSession) -> some View {
        Button("Open another window") { openAnother(item) }
        if model.createHosts.contains(item.ref.serverID) {
            Button("New session in this project") {
                newSession = NewSessionSeed(host: item.ref.serverID, project: item.session.project)
            }
        }
    }

    /// The session's window, brought forward when it has one.
    private func open(_ item: UnifiedSession) {
        openWindow(value: model.windowRequest(for: item.ref))
    }

    /// A second window on the session, from the context menu.
    private func openAnother(_ item: UnifiedSession) {
        openWindow(value: CardWindowRequest(ref: item.ref))
    }

    private var empty: some View {
        Text(model.store.sessions.isEmpty
            ? "No sessions from a connected host." : "Nothing passes the filters.")
            .foregroundStyle(.secondary)
    }

    private var list: some View {
        List {
            Section("Sessions") {
                if model.sessions.isEmpty { empty }
                ForEach(model.sessions) { item in
                    Button { open(item) } label: { SessionCardView(item: item) }
                        .buttonStyle(.plain)
                        .contextMenu { menu(for: item) }
                }
            }
            boardsSection
        }
    }

    private var grid: some View {
        ScrollView {
            if model.sessions.isEmpty { empty.padding() }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 280), spacing: 12)], spacing: 12) {
                ForEach(model.sessions) { item in
                    Button { open(item) } label: {
                        SessionCardView(item: item)
                            .padding(12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                    }
                    .buttonStyle(.plain)
                    .contextMenu { menu(for: item) }
                }
            }
            .padding()
            List { boardsSection }.frame(minHeight: 200)
        }
    }

    private var focus: some View {
        let items = model.sessions
        return VStack(spacing: 12) {
            if let i = FocusCursor.index(of: model.focused, in: items) {
                let item = items[i]
                HStack {
                    Button { model.focused = FocusCursor.moved(model.focused, by: -1, in: items) } label: {
                        Image(systemName: "chevron.left")
                    }
                    Spacer()
                    Text("\(i + 1) of \(items.count)").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button { model.focused = FocusCursor.moved(model.focused, by: 1, in: items) } label: {
                        Image(systemName: "chevron.right")
                    }
                }
                SessionCardView(item: item).font(.title3)
                CompactDetailView(item: item)
                Button("Open in a window") { open(item) }
            } else {
                Spacer()
                empty
                Spacer()
            }
        }
        .padding()
    }

    /// The working sets, one section per source.
    @ViewBuilder
    /// The pin boards, the client's own: a tap opens one, and a new one
    /// is made here or from any card's pin menu. The hosts' working
    /// sets are not listed: a board is the headset's working set.
    private var boardsSection: some View {
        Section("Boards") {
            ForEach(model.boards) { board in
                Button {
                    openWindow(value: BoardWindowRequest(board: board.id))
                } label: {
                    HStack {
                        Image(systemName: "pin")
                        Text(board.name)
                        Spacer()
                        Text("\(board.tiles.count) pinned").font(.caption).foregroundStyle(.secondary)
                    }
                }
                .buttonStyle(.plain)
                .contextMenu {
                    Button("Remove board", role: .destructive) {
                        model.removeBoard(board.id)
                    }
                }
            }
            Button {
                let board = model.addBoard(named: "")
                openWindow(value: BoardWindowRequest(board: board.id))
            } label: {
                Label("New board", systemImage: "plus")
            }
        }
    }
}

/// The filters of §8 as toggles: hosts, kinds, and the states the
/// listed sessions are in. It reads only the choices and the filters,
/// never the store: a menu whose body is re-evaluated while open
/// scrolls back to its top, and the store changes on every pull.
struct FilterMenu: View, Equatable {
    @Bindable var model: BrowserModel

    /// The parent redraws on every pull; this says when the menu has
    /// nothing new to draw, so SwiftUI leaves it as it stands.
    static func == (lhs: FilterMenu, rhs: FilterMenu) -> Bool {
        lhs.model.filterChoices == rhs.model.filterChoices && lhs.model.filters == rhs.model.filters
    }

    var body: some View {
        Menu {
            Section("Hosts") {
                ForEach(model.filterChoices.hosts) { host in
                    Toggle(host.name, isOn: binding(\.servers, host.id, all: model.filterChoices.hosts.map(\.id)))
                }
            }
            Section("Kinds") {
                ForEach([SessionKind.claude, .codex, .shell, .command, .service], id: \.self) { kind in
                    Toggle(kind.rawValue, isOn: binding(\.kinds, kind, all: [.claude, .codex, .shell, .command, .service]))
                }
            }
            Section("States") {
                let states = model.filterChoices.states
                ForEach(states, id: \.self) { state in
                    Toggle(state, isOn: binding(\.cards, state, all: states))
                }
            }
            if model.filters != Filters() {
                Button("Clear filters") { model.filters = Filters() }
            }
        } label: {
            Label("Filter", systemImage: model.filters == Filters() ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
        }
    }

    /// An empty set admits everything, so turning one member off from
    /// there selects all the others first.
    private func binding<T: Hashable>(_ path: WritableKeyPath<Filters, Set<T>>, _ member: T, all: [T]) -> Binding<Bool> {
        Binding(
            get: { model.filters[keyPath: path].isEmpty || model.filters[keyPath: path].contains(member) },
            set: { on in
                var filters = model.filters
                if on {
                    filters[keyPath: path].insert(member)
                    if filters[keyPath: path].count == all.count { filters[keyPath: path] = [] }
                } else {
                    if filters[keyPath: path].isEmpty { filters[keyPath: path] = Set(all) }
                    filters[keyPath: path].remove(member)
                }
                model.filters = filters
            })
    }
}

/// One session: its name, kind and card word, with the source label.
public struct SessionCardView: View {
    public let item: UnifiedSession

    public init(item: UnifiedSession) {
        self.item = item
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(item.session.name).font(.headline)
                Spacer()
                SourceLabel(name: item.serverName, stale: item.stale)
            }
            HStack(spacing: 8) {
                Text(item.session.kind.rawValue).font(.caption).foregroundStyle(.secondary)
                Text(item.session.card)
                    .font(.caption)
                    .foregroundStyle(item.session.waiting ? Color.orange : Color.secondary)
                if let reason = item.session.waitingReason {
                    Text(reason).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            if !item.session.notes.isEmpty {
                Text(item.session.notes).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .opacity(item.stale ? 0.6 : 1)
    }
}

/// The source-machine label, with the disconnected state made plain.
public struct SourceLabel: View {
    public enum Size: Sendable {
        case regular
        /// For a card window's header, where width is short.
        case small
    }

    public let name: String
    public let stale: Bool
    public var size: Size = .regular

    public init(name: String, stale: Bool, size: Size = .regular) {
        self.name = name
        self.stale = stale
        self.size = size
    }

    public var body: some View {
        HStack(spacing: 3) {
            Image(systemName: stale ? "wifi.slash" : "desktopcomputer")
            Text(stale ? "\(name) (offline)" : name)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .font(size == .small ? .caption2 : .caption)
        .padding(.horizontal, size == .small ? 5 : 6)
        .padding(.vertical, size == .small ? 1 : 2)
        .background(.quaternary, in: Capsule())
    }
}

/// Known and discovered hosts, with pairing, enable and forget.
public struct ServersView: View {
    @Bindable var model: BrowserModel
    @State private var inviteText = ""
    @State private var error: String?
    @State private var pairing: DiscoveredServerRow?

    public init(model: BrowserModel) {
        self.model = model
    }

    public var body: some View {
        List {
            Section("Known hosts") {
                ForEach(model.store.hosts) { host in
                    VStack(alignment: .leading) {
                        HStack {
                            Text(host.server.name).font(.headline)
                            if let version = host.version {
                                Text(version).font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Text(stateWord(host)).font(.caption).foregroundStyle(.secondary)
                        }
                        if host.identityChanged {
                            Text("This host's key changed; pair again.").font(.caption)
                                .foregroundStyle(.red)
                        }
                        if let error = host.lastError, !host.state.isConnected {
                            Text(error).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
                        }
                        HStack {
                            Button(host.server.enabled ? "Disable" : "Enable") {
                                Task { await model.manager.setEnabled(host.id, !host.server.enabled) }
                            }
                            Button("Forget", role: .destructive) {
                                Task { await model.manager.forget(host.id) }
                            }
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            Section("Discovered") {
                if model.discovered.isEmpty {
                    Text("Nothing on this network yet.").foregroundStyle(.secondary)
                }
                ForEach(model.discovered) { row in
                    HStack {
                        Text(row.name ?? row.instance)
                        Spacer()
                        Text(row.serverID?.short ?? "").font(.caption).foregroundStyle(.secondary)
                        if row.serverID != nil, !known(row) {
                            Button("Pair") { pairing = row }
                                .buttonStyle(.borderless)
                        }
                    }
                }
            }
            Section("Pair by text") {
                TextField("Paste the QR code's text", text: $inviteText, axis: .vertical)
                Button("Pair") {
                    Task { await pair() }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.caption)
                }
            }
        }
        .sheet(item: $pairing) { row in
            PairSheet(row: row, model: model)
        }
    }

    /// A discovered record whose host is already known and paired.
    private func known(_ row: DiscoveredServerRow) -> Bool {
        model.store.hosts.contains { $0.id == row.serverID && !$0.server.needsPairing }
    }

    private func stateWord(_ host: HostStatus) -> String {
        switch host.state {
        case .discovered: return "discovered"
        case .unpaired: return "needs pairing"
        case .pairing: return "pairing"
        case .connecting: return "connecting"
        case .syncing: return "syncing"
        case .connected: return "connected"
        case .reconnecting(let n): return "reconnecting (\(n))"
        case .offline(let reason): return "offline: \(reason)"
        }
    }

    private func pair() async {
        do {
            let invite = try PairingInvite.parse(inviteText)
            try await model.manager.pair(invite: invite)
            inviteText = ""
            error = nil
            await model.pull()
        } catch {
            self.error = "\(error)"
        }
    }
}

/// Pairing with a discovered host without a camera: the host is looked
/// up and the key it presents is shown as the host's dialog shows its
/// own, for the owner to compare by eye, then the dialog's code is
/// typed. The key seen is pinned for the pairing, so the code reaches
/// only the host whose key was checked (§5).
struct PairSheet: View {
    let row: DiscoveredServerRow
    @Bindable var model: BrowserModel
    @Environment(\.dismiss) private var dismiss
    @State private var lookup: HostLookup?
    @State private var code = ""
    @State private var problem: String?
    @State private var busy = false

    private var ready: Bool {
        lookup != nil && code.filter { !$0.isWhitespace }.count == 8 && !busy
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    LabeledContent("Host", value: lookup?.info.serverName ?? row.name ?? row.instance)
                    LabeledContent("Address", value: lookup.map { "\($0.address)" } ?? "looking up")
                    LabeledContent("Key", value: lookup.map { KeyFingerprint.short($0.info.spkiSha256) } ?? "")
                        .fontDesign(.monospaced)
                    Text("Open Remote devices on the host and allow pairing. Check that its Key line reads the same, then type its code.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Section("Code") {
                    TextField("Eight characters", text: $code)
                        .fontDesign(.monospaced)
                        .codeEntry()
                    if let lookup, !lookup.info.pairingOpen {
                        Text("The host is not allowing pairing right now.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                if let problem {
                    Section {
                        Text(problem).foregroundStyle(.red).font(.caption)
                    }
                }
            }
            .navigationTitle("Pair")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Pair") { Task { await pair() } }.disabled(!ready)
                }
            }
        }
        .task {
            do {
                lookup = try await model.lookup(row.instance)
            } catch {
                problem = "\(error)"
            }
        }
    }

    private func pair() async {
        guard let lookup else { return }
        busy = true
        defer { busy = false }
        do {
            try await model.pair(lookup, code: code)
            dismiss()
        } catch {
            problem = "\(error)"
        }
    }
}

extension View {
    /// A pairing code is typed as the host shows it: capitals, no
    /// autocorrect. The package also builds on the Mac for the tests,
    /// where these modifiers do not exist.
    @ViewBuilder
    func codeEntry() -> some View {
        #if os(iOS) || os(visionOS)
        self.textInputAutocapitalization(.characters).autocorrectionDisabled()
        #else
        self
        #endif
    }
}
