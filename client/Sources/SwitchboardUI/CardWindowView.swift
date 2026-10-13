import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// One window on one session (§7.2): the mode row, the content for the
/// mode, and a banner when the host is away or the session is gone.
/// Closing it releases its watch and nothing else; a session is never
/// ended from here.
public struct CardWindowView: View {
    @Bindable var model: BrowserModel
    public let request: CardWindowRequest
    @State private var mode: ViewMode = .compact

    public init(model: BrowserModel, request: CardWindowRequest) {
        self.model = model
        self.request = request
    }

    private var item: UnifiedSession? {
        model.store.sessions.first { $0.ref == request.ref }
    }

    private var standing: CardStanding {
        CardStanding.of(request.ref, in: model.store, removed: model.removed)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            if let banner = standing.banner {
                Label(banner, systemImage: standing == .removed ? "trash" : "wifi.slash")
                    .font(.callout)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            }
            if let item {
                modeRow(for: item.session.kind)
                content(item)
            } else {
                Spacer()
            }
        }
        .padding()
        .navigationTitle(item?.session.name ?? "Session")
        .onAppear {
            mode = model.mode(for: request.id)
            model.windowAppeared(request)
        }
        .onDisappear { model.windowClosed(request) }
        .onChange(of: mode) { _, new in model.setMode(new, for: request.id) }
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading) {
                Text(item?.session.name ?? request.ref.recordID).font(.title2.bold())
                if let item {
                    Text("\(item.session.kind.rawValue) · \(item.session.card)")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer()
            if let item {
                SourceLabel(name: item.serverName, stale: item.stale)
            }
        }
    }

    private func modeRow(for kind: SessionKind) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                ForEach(ViewMode.offered(for: kind)) { offered in
                    Button(offered.title) { mode = offered }
                        .buttonStyle(.bordered)
                        .tint(offered == mode ? .accentColor : nil)
                        .disabled(!offered.drawn || !standing.controlsEnabled)
                }
            }
            if ViewMode.offered(for: kind).contains(where: { !$0.drawn }) {
                Text("The greyed modes arrive with \(ViewMode.arrivesWith).")
                    .font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func content(_ item: UnifiedSession) -> some View {
        switch mode {
        case .compact:
            CompactDetailView(item: item)
        case .screen:
            ScreenView(model: model, ref: item.ref)
        case .lastResponse, .conversation, .runOutput:
            VStack {
                Spacer()
                Text("\(mode.title) arrives with \(ViewMode.arrivesWith).")
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity)
        }
    }
}

/// The card's fields in full.
struct CompactDetailView: View {
    let item: UnifiedSession

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                row("State", item.session.card)
                if let reason = item.session.waitingReason { row("Waiting on", reason) }
                row("Directory", item.session.cwd)
                if let code = item.session.lastExit { row("Last exit", "\(code)") }
                if let ms = item.session.lastStopAtMs {
                    row("Last stop", Date(timeIntervalSince1970: Double(ms) / 1000).formatted())
                }
                if !item.session.pending.isEmpty {
                    row("Pending", item.session.pending.joined(separator: ", "))
                }
                if !item.session.notes.isEmpty {
                    Text("Notes").font(.caption).foregroundStyle(.secondary)
                    Text(item.session.notes).textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.caption).foregroundStyle(.secondary).frame(width: 90, alignment: .leading)
            Text(value).textSelection(.enabled)
        }
    }
}

/// The pane's text, live through a watch held while the view is on
/// screen; the last read stays up when the host goes away.
struct ScreenView: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef

    var body: some View {
        Group {
            if let screen = model.screens[ref] {
                ScrollViewReader { proxy in
                    ScrollView {
                        Text(screen.text)
                            .font(.system(.body, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Color.clear.frame(height: 1).id("end")
                    }
                    .onChange(of: screen.text) { _, _ in
                        proxy.scrollTo("end", anchor: .bottom)
                    }
                    .onAppear { proxy.scrollTo("end", anchor: .bottom) }
                }
                if screen.truncated {
                    Text("Cut at the host's size cap.").font(.caption2).foregroundStyle(.secondary)
                }
            } else {
                VStack {
                    Spacer()
                    Text("No screen yet: the pane is not running, or has not been read.")
                        .foregroundStyle(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            }
        }
        .task(id: ref) {
            // The watch is held for as long as this view is on screen:
            // the task is cancelled when it leaves, and the release
            // runs after that.
            await model.watch(ref)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
            await model.unwatch(ref)
        }
    }
}
