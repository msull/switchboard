import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// The shared read model the views draw from, refreshed from the
/// manager's store on the main actor.
@MainActor
@Observable
public final class BrowserModel {
    public let manager: ConnectionManager
    public private(set) var store = UnifiedStore()
    public private(set) var discovered: [DiscoveredServerRow] = []
    /// The card windows that have appeared this run, restored by the
    /// system or opened here, so the reopen pass skips them.
    public private(set) var appeared: Set<UUID> = []
    /// The screen last read per watched session, mirrored from its
    /// connection; the holders per session are the open screen views.
    public private(set) var screens: [SessionRef: ScreenBody] = [:]
    private var holders: [SessionRef: Int] = [:]
    /// Every session seen from a host, and the ones a connected host
    /// no longer lists: a window on one says it was removed.
    private var seen: Set<SessionRef> = []
    public private(set) var removed: Set<SessionRef> = []
    /// The client's own presentation state, kept in its container.
    public private(set) var presentation = PresentationState()
    private let presentationStore: PresentationStore
    private var pump: Task<Void, Never>?
    private var booting: Task<Void, Never>?
    /// Whether the main window is on screen, and whether a card
    /// window has already taken the job of bringing it back.
    public private(set) var mainWindow = MainWindowGuard()

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

    public func pull() async {
        store = await manager.store
        discovered = await manager.discovered.map {
            DiscoveredServerRow(instance: $0.instance, serverID: $0.serverID, name: $0.name)
        }.sorted { $0.instance < $1.instance }
        var screens: [SessionRef: ScreenBody] = [:]
        for ref in holders.keys {
            screens[ref] = await manager.screen(for: ref)
        }
        self.screens = screens
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

    /// The mode a card window shows, `compact` until it chooses.
    public func mode(for window: UUID) -> ViewMode {
        presentation.mode(for: window) ?? .compact
    }

    public func setMode(_ mode: ViewMode, for window: UUID) {
        presentation.setMode(mode, for: window)
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

    public func mainWindowAppeared() {
        mainWindow.visible = true
    }

    public func mainWindowDisappeared() {
        mainWindow.visible = false
    }

    /// A card window asks, after the restoration grace, whether it is
    /// the one to open the main window.
    public func claimMainWindow() -> Bool {
        mainWindow.claim()
    }

    /// A card window is on screen: remembered for the next launch.
    public func windowAppeared(_ request: CardWindowRequest) {
        appeared.insert(request.id)
        presentation.windows[request.id] = request.ref
        save()
    }

    /// A card window was closed by the user; a window the process
    /// dies with stays in the file and is reopened.
    public func windowClosed(_ request: CardWindowRequest) {
        presentation.windows[request.id] = nil
        save()
    }

    /// The saved windows the system did not restore, once their
    /// sessions are listed.
    public func windowsToReopen() -> [CardWindowRequest] {
        presentation.windowsToReopen(appeared: appeared, listed: Set(store.sessions.map(\.ref)))
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
                Picker("Layout", selection: $model.browse) {
                    ForEach(BrowseMode.allCases) { mode in
                        Label(mode.title, systemImage: mode.symbol).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
                FilterMenu(model: model)
            }
        }
    }

    private func open(_ item: UnifiedSession) {
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
                }
            }
            setsSections
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
                }
            }
            .padding()
            List { setsSections }.frame(minHeight: 200)
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
    private var setsSections: some View {
        let sources = Dictionary(grouping: model.store.sets, by: \.serverName)
        ForEach(sources.keys.sorted(), id: \.self) { source in
            Section("Working sets · \(source)") {
                ForEach(sources[source] ?? []) { item in
                    HStack {
                        Text(item.set.name)
                        Spacer()
                        Text("\(item.set.items.count) pins").font(.caption).foregroundStyle(.secondary)
                        SourceLabel(name: item.serverName, stale: item.stale)
                    }
                }
            }
        }
    }
}

/// The filters of §8 as toggles: hosts, kinds, and the states the
/// listed sessions are in.
struct FilterMenu: View {
    @Bindable var model: BrowserModel

    var body: some View {
        Menu {
            Section("Hosts") {
                ForEach(model.store.hosts) { host in
                    Toggle(host.server.name, isOn: binding(\.servers, host.id, all: model.store.hosts.map(\.id)))
                }
            }
            Section("Kinds") {
                ForEach([SessionKind.claude, .codex, .shell, .command, .service], id: \.self) { kind in
                    Toggle(kind.rawValue, isOn: binding(\.kinds, kind, all: [.claude, .codex, .shell, .command, .service]))
                }
            }
            Section("States") {
                let states = Array(Set(model.store.sessions.map(\.session.card))).sorted()
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
    public let name: String
    public let stale: Bool

    public init(name: String, stale: Bool) {
        self.name = name
        self.stale = stale
    }

    public var body: some View {
        HStack(spacing: 4) {
            Image(systemName: stale ? "wifi.slash" : "desktopcomputer")
            Text(stale ? "\(name) (offline)" : name)
        }
        .font(.caption)
        .padding(.horizontal, 6)
        .padding(.vertical, 2)
        .background(.quaternary, in: Capsule())
    }
}

/// Known and discovered hosts, with pairing, enable and forget.
public struct ServersView: View {
    @Bindable var model: BrowserModel
    @State private var inviteText = ""
    @State private var error: String?

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
                    }
                }
            }
            Section("Pair") {
                TextField("Paste the QR code's text", text: $inviteText, axis: .vertical)
                Button("Pair") {
                    Task { await pair() }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.caption)
                }
            }
        }
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
