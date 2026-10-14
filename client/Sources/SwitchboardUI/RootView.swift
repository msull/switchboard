import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// The main window: the unified browser and the hosts screen as tabs.
public struct RootView: View {
    @Bindable var model: BrowserModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    /// A dev aid: card windows to open once their sessions are listed,
    /// as `<record id>[:<mode>]` entries, so a simulator run reaches a
    /// window without a tap; `prompt` opens the prompt window.
    public var openOnStart: [String] = []

    public init(model: BrowserModel, openOnStart: [String] = []) {
        self.model = model
        self.openOnStart = openOnStart
    }

    public var body: some View {
        tabs
            .onAppear { model.mainWindowAppeared() }
            .onDisappear { model.mainWindowDisappeared() }
            .task(id: openOnStart) { await openRequested() }
            .task { await reopenSaved() }
            .task { await closeAllOnRequest() }
    }

    @MainActor private static var closedOnRequest = false

    /// A dev aid: `SWITCHBOARD_CLOSE_WINDOWS_AFTER` closes every window
    /// that many seconds after launch, which is the state a user leaves
    /// the app in by closing its last window; a simulator run can then
    /// be activated again as a tap on the icon would.
    private func closeAllOnRequest() async {
        guard let text = ProcessInfo.processInfo.environment["SWITCHBOARD_CLOSE_WINDOWS_AFTER"],
            let seconds = Double(text), !Self.closedOnRequest
        else { return }
        // Once per process: a window opened by a later activation stays.
        Self.closedOnRequest = true
        try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        dismissWindow(id: PromptWindow.id)
        for (id, ref) in model.presentation.windows {
            dismissWindow(value: CardWindowRequest(id: id, ref: ref))
        }
        dismissWindow(id: MainWindow.id)
    }

    /// The card windows open at the last save that the system did not
    /// bring back: after the launch grace, each whose session is listed
    /// is opened under its saved id, and so its mode.
    private func reopenSaved() async {
        try? await Task.sleep(nanoseconds: LaunchGrace.nanoseconds)
        for _ in 0..<100 {
            let pending = model.windowsToReopen()
            for request in pending {
                openWindow(value: request)
            }
            if model.presentation.windows.allSatisfy({ model.appeared.contains($0.key) }) { return }
            try? await Task.sleep(nanoseconds: 300_000_000)
        }
    }

    private func openRequested() async {
        var pending = openOnStart.filter { !$0.isEmpty }
        if !pending.isEmpty {
            try? await Task.sleep(nanoseconds: LaunchGrace.nanoseconds)
        }
        // `prompt` opens the prompt window, `prompt:<seconds>` after a wait.
        if let entry = pending.first(where: { $0 == "prompt" || $0.hasPrefix("prompt:") }) {
            pending.removeAll { $0 == entry }
            if let wait = Double(entry.dropFirst("prompt:".count)), entry.hasPrefix("prompt:") {
                try? await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
            }
            openWindow(id: PromptWindow.id, value: PromptWindow.value)
        }
        for _ in 0..<100 where !pending.isEmpty {
            for entry in pending {
                let parts = entry.split(separator: ":", maxSplits: 1).map(String.init)
                guard let item = model.store.sessions.first(where: { $0.session.id == parts[0] })
                else { continue }
                // A saved window on the session that the reopen pass
                // has not brought back yet would be missed: wait for
                // it, so the dev aid lands where a click would.
                if model.windowsToReopen().contains(where: { $0.ref == item.ref }) { continue }
                let request = model.windowRequest(for: item.ref)
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
                    .toolbar {
                        Button {
                            openWindow(id: PromptWindow.id, value: PromptWindow.value)
                        } label: {
                            Label("Prompt window", systemImage: "text.alignleft")
                        }
                        .help("The prompt window: the whole draft of the session being worked on")
                    }
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
