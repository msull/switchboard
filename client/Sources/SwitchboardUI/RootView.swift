import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// The main window: the unified browser and the hosts screen as tabs.
public struct RootView: View {
    @Bindable var model: BrowserModel
    @Environment(\.openWindow) private var openWindow
    /// A dev aid: card windows to open once their sessions are listed,
    /// as `<record id>[:<mode>]` entries, so a simulator run reaches a
    /// window without a tap.
    public var openOnStart: [String] = []

    public init(model: BrowserModel, openOnStart: [String] = []) {
        self.model = model
        self.openOnStart = openOnStart
    }

    public var body: some View {
        tabs.task(id: openOnStart) { await openRequested() }
    }

    private func openRequested() async {
        var pending = openOnStart.filter { !$0.isEmpty }
        for _ in 0..<100 where !pending.isEmpty {
            for entry in pending {
                let parts = entry.split(separator: ":", maxSplits: 1).map(String.init)
                guard let item = model.store.sessions.first(where: { $0.session.id == parts[0] })
                else { continue }
                let request = CardWindowRequest(ref: item.ref)
                if parts.count == 2, let mode = ViewMode(rawValue: parts[1]) {
                    model.setMode(mode, for: request.id)
                }
                openWindow(value: request)
                pending.removeAll { $0 == entry }
            }
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
    }

    private var tabs: some View {
        TabView {
            NavigationStack {
                BrowserView(model: model)
                    .navigationTitle("Sessions")
            }
            .tabItem { Label("Sessions", systemImage: "rectangle.grid.2x2") }
            NavigationStack {
                ServersView(model: model)
                    .navigationTitle("Hosts")
            }
            .tabItem { Label("Hosts", systemImage: "desktopcomputer") }
        }
    }
}
