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
    /// The screen's text scale: window state, so it survives a mode
    /// switch and goes with the window, and a new window starts at 1.
    @State private var screenScale = ScreenScale.initial
    @State private var scalePopover = false
    @Environment(\.openWindow) private var openWindow

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
        .task { await bringMainWindowBack() }
    }

    /// A launch that restored only card windows has no browser: after
    /// the grace the system gets to restore one, the first card window
    /// to ask opens it.
    private func bringMainWindowBack() async {
        try? await Task.sleep(nanoseconds: 1_500_000_000)
        if model.claimMainWindow() {
            openWindow(id: MainWindow.id, value: MainWindow.value)
        }
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
            Spacer(minLength: 4)
            if let item {
                SourceLabel(name: item.serverName, stale: item.stale, size: .small)
            }
            // Icon only, and small: a tall narrow card has no room for
            // a word here.
            Button {
                openWindow(id: MainWindow.id, value: MainWindow.value)
            } label: {
                Image(systemName: "rectangle.grid.2x2")
                    .font(.caption)
            }
            .buttonStyle(.bordered)
            .buttonBorderShape(.circle)
            .controlSize(.mini)
            .accessibilityLabel("Sessions")
            .help("The main window, with every session")
        }
    }

    private func modeRow(for kind: SessionKind) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                // The pills never wrap: when the titles do not fit the
                // width, the row is icons instead.
                ViewThatFits(in: .horizontal) {
                    modeButtons(for: kind, titled: true)
                    modeButtons(for: kind, titled: false)
                }
                Spacer(minLength: 0)
                if mode == .screen {
                    scaleControl
                }
            }
            if ViewMode.offered(for: kind).contains(where: { !$0.drawn }) {
                Text("The greyed modes arrive with \(ViewMode.arrivesWith).")
                    .font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    private func modeButtons(for kind: SessionKind, titled: Bool) -> some View {
        HStack(spacing: 4) {
            ForEach(ViewMode.offered(for: kind)) { offered in
                Button {
                    mode = offered
                } label: {
                    if titled {
                        Text(offered.title)
                    } else {
                        Image(systemName: offered.symbol)
                    }
                }
                .buttonStyle(.bordered)
                .controlSize(.mini)
                .tint(offered == mode ? .accentColor : nil)
                .disabled(!offered.drawn || !standing.controlsEnabled)
                .accessibilityLabel(offered.title)
                .help(offered.title)
            }
        }
        // The full width is what `ViewThatFits` must measure; a pill
        // that could shrink by truncating its title would always fit.
        .fixedSize(horizontal: true, vertical: false)
    }

    /// The screen's text scale: a small readout that opens a slider
    /// and the snap sizes, kept out of the window's bottom edge where
    /// the system's own controls sit.
    private var scaleControl: some View {
        Button {
            scalePopover.toggle()
        } label: {
            Label(ScreenScale.label(screenScale), systemImage: "textformat.size")
                .font(.caption2.monospacedDigit())
        }
        .buttonStyle(.bordered)
        .controlSize(.mini)
        .accessibilityLabel("Text size")
        .popover(isPresented: $scalePopover) {
            VStack(spacing: 12) {
                HStack(spacing: 6) {
                    Image(systemName: "textformat.size.smaller")
                    Slider(value: $screenScale, in: ScreenScale.range)
                    Image(systemName: "textformat.size.larger")
                }
                .foregroundStyle(.secondary)
                HStack(spacing: 6) {
                    ForEach(ScreenScale.presets, id: \.self) { preset in
                        Button(ScreenScale.label(preset)) { screenScale = preset }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            .tint(preset == screenScale ? .accentColor : nil)
                    }
                }
            }
            .padding()
            .frame(width: 360)
        }
    }

    @ViewBuilder
    private func content(_ item: UnifiedSession) -> some View {
        switch mode {
        case .compact:
            CompactDetailView(item: item)
        case .screen:
            ScreenView(model: model, ref: item.ref, scale: screenScale)
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
/// screen; the last read stays up when the host goes away. The text
/// is drawn at the scale the window chose.
struct ScreenView: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    let scale: Double

    var body: some View {
        Group {
            if let screen = model.screens[ref] {
                // Both axes scroll, so a line never wraps at a large
                // scale; the text is at least as wide as the viewport
                // and as tall, so a short pane sits at the top left, not
                // centered.
                GeometryReader { viewport in
                    ScrollViewReader { proxy in
                        ScrollView([.vertical, .horizontal]) {
                            VStack(alignment: .leading, spacing: 0) {
                                Text(screen.text)
                                    .font(.system(size: ScreenScale.basePoints * scale, design: .monospaced))
                                    .textSelection(.enabled)
                                Color.clear.frame(height: 1).id("end")
                            }
                            .frame(minWidth: viewport.size.width, minHeight: viewport.size.height, alignment: .topLeading)
                        }
                        .onChange(of: screen.text) { _, _ in
                            proxy.scrollTo("end", anchor: .bottom)
                        }
                        .onAppear { proxy.scrollTo("end", anchor: .bottom) }
                    }
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
