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
    public var filters = Filters()
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

    /// Load the registry, pair with `invite` if one is given (a dev
    /// aid), start the manager's loop with Bonjour, and mirror its
    /// store; the apps call this once from their first scene.
    public func boot(invite: String? = nil) async {
        try? await manager.load()
        if let invite, let parsed = try? PairingInvite.parse(invite) {
            try? await manager.pair(invite: parsed)
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
        try? presentationStore.save(presentation)
    }

    public var sessions: [UnifiedSession] {
        store.sessions.filter(filters.admits)
    }
}

public struct DiscoveredServerRow: Identifiable, Sendable, Equatable {
    public let instance: String
    public let serverID: ServerID?
    public let name: String?
    public var id: String { instance }
}

/// Sessions and sets from every connected host, each card labelled
/// with its source and marked when the host is not connected; a
/// session opens as a window of its own.
public struct BrowserView: View {
    @Bindable var model: BrowserModel
    @Environment(\.openWindow) private var openWindow

    public init(model: BrowserModel) {
        self.model = model
    }

    public var body: some View {
        List {
            Section("Sessions") {
                if model.sessions.isEmpty {
                    Text("No sessions from a connected host.").foregroundStyle(.secondary)
                }
                ForEach(model.sessions) { item in
                    Button {
                        openWindow(value: CardWindowRequest(ref: item.ref))
                    } label: {
                        SessionCardView(item: item)
                    }
                    .buttonStyle(.plain)
                }
            }
            Section("Working sets") {
                ForEach(model.store.sets) { item in
                    HStack {
                        Text(item.set.name)
                        Spacer()
                        SourceLabel(name: item.serverName, stale: item.stale)
                    }
                }
            }
        }
        .toolbar {
            ToolbarItem {
                Menu("Hosts") {
                    ForEach(model.store.hosts) { host in
                        Toggle(
                            host.server.name,
                            isOn: Binding(
                                get: {
                                    model.filters.servers.isEmpty
                                        || model.filters.servers.contains(host.id)
                                },
                                set: { on in
                                    if on {
                                        model.filters.servers.remove(host.id)
                                    } else {
                                        if model.filters.servers.isEmpty {
                                            model.filters.servers = Set(model.store.hosts.map(\.id))
                                        }
                                        model.filters.servers.remove(host.id)
                                    }
                                }))
                    }
                }
            }
        }
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
