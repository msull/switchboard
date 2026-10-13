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
    /// The text scale: window state, so it survives a mode switch and
    /// goes with the window, and a new window starts at 1.
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
                if mode.scalable {
                    scaleControl
                }
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
                .disabled(!standing.controlsEnabled)
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
                            .fixedSize()
                            .tint(preset == screenScale ? .accentColor : nil)
                    }
                }
            }
            .padding()
            .frame(width: 440)
        }
    }

    @ViewBuilder
    private func content(_ item: UnifiedSession) -> some View {
        switch mode {
        case .compact:
            CompactDetailView(item: item)
        case .screen:
            ScreenView(model: model, ref: item.ref, scale: screenScale)
        case .lastResponse:
            LastResponseView(model: model, ref: item.ref).environment(\.textScale, screenScale)
        case .conversation:
            ConversationView(model: model, ref: item.ref).environment(\.textScale, screenScale)
        case .runOutput:
            RunOutputView(model: model, ref: item.ref, scale: screenScale)
        }
    }
}

/// The time of a wire millisecond stamp, short.
func clock(_ ms: UInt64?) -> String? {
    ms.map { Date(timeIntervalSince1970: Double($0) / 1000).formatted(date: .omitted, time: .shortened) }
}

/// The window's text scale, handed down to the prose views so every
/// line in them sizes off the one control.
private struct TextScaleKey: EnvironmentKey {
    static let defaultValue = ScreenScale.initial
}

extension EnvironmentValues {
    var textScale: Double {
        get { self[TextScaleKey.self] }
        set { self[TextScaleKey.self] = newValue }
    }
}

/// The prose fonts at a scale.
enum Scaled {
    static func prose(_ scale: Double) -> Font { .system(size: ScreenScale.prosePoints * scale) }
    static func callout(_ scale: Double) -> Font { .system(size: ScreenScale.calloutPoints * scale) }
    static func caption(_ scale: Double) -> Font { .system(size: ScreenScale.captionPoints * scale) }
    static func caption2(_ scale: Double) -> Font { .system(size: ScreenScale.caption2Points * scale) }
    static func mono(_ scale: Double) -> Font {
        .system(size: ScreenScale.captionPoints * scale, design: .monospaced)
    }
}

/// Text the agent wrote, with its Markdown read where it parses and
/// shown plain where it does not.
struct AgentText: View {
    let text: String
    @Environment(\.textScale) private var scale

    var body: some View {
        Group {
            if let rich = try? AttributedString(
                markdown: text,
                options: AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace))
            {
                Text(rich)
            } else {
                Text(text)
            }
        }
        .font(Scaled.prose(scale))
        .textSelection(.enabled)
    }
}

/// The last turn's answer, live through the conversation watch held
/// while the view is on screen (§4.3: a projection of the conversation).
struct LastResponseView: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    @Environment(\.textScale) private var scale

    var body: some View {
        Group {
            if let last = model.lastResponses[ref] {
                if last.n == 0 {
                    placeholder("No response yet.")
                } else {
                    ScrollViewReader { proxy in
                        ScrollView {
                            VStack(alignment: .leading, spacing: 8) {
                                HStack {
                                    Text("Turn \(last.n) of \(last.total)")
                                    if let at = clock(last.endMs) { Text("· \(at)") }
                                    if last.truncated { Text("· cut at the host's cap") }
                                }
                                .font(Scaled.caption(scale)).foregroundStyle(.secondary)
                                AgentText(text: last.text)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                Color.clear.frame(height: 1).id("end")
                            }
                        }
                        .onChange(of: last) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
                    }
                }
            } else {
                placeholder("No conversation: the session has not started one, or is not a Claude Code session.")
            }
        }
        .task(id: ref) {
            await model.watchConversation(ref)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
            await model.unwatchConversation(ref)
        }
    }

    private func placeholder(_ text: String) -> some View {
        VStack {
            Spacer()
            Text(text).foregroundStyle(.secondary).multilineTextAlignment(.center)
            Spacer()
        }
        .frame(maxWidth: .infinity)
    }
}

/// The conversation by turn: the prompt, the activity folded under a
/// line, the answer; earlier pages load above, and the end follows the
/// host's `changed` (§4.3).
struct ConversationView: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    @State private var loading = false

    var body: some View {
        Group {
            if let conversation = model.conversations[ref] {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 14) {
                            if conversation.hasEarlier {
                                Button {
                                    loading = true
                                    Task {
                                        await model.loadEarlier(ref)
                                        loading = false
                                    }
                                } label: {
                                    Label(
                                        "Earlier turns (\((conversation.firstHeld ?? 1) - 1) before)",
                                        systemImage: "arrow.up")
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                                .disabled(loading)
                            }
                            ForEach(conversation.turns) { turn in
                                TurnRow(model: model, ref: ref, turn: turn)
                            }
                            Color.clear.frame(height: 1).id("end")
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .onChange(of: conversation.lastHeld) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
                    .onChange(of: conversation.turns.last?.finalText) { _, _ in
                        proxy.scrollTo("end", anchor: .bottom)
                    }
                    .onAppear { proxy.scrollTo("end", anchor: .bottom) }
                }
            } else {
                VStack {
                    Spacer()
                    Text("No conversation: the session has not started one, or is not a Claude Code session.")
                        .foregroundStyle(.secondary).multilineTextAlignment(.center)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            }
        }
        .task(id: ref) {
            await model.watchConversation(ref)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
            await model.unwatchConversation(ref)
        }
    }
}

/// One turn: the prompt, the activity behind a disclosure, the answer.
struct TurnRow: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    let turn: TurnView
    @State private var open = false
    @Environment(\.textScale) private var scale

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("\(turn.n)").font(Scaled.caption(scale).monospacedDigit()).foregroundStyle(.secondary)
                Text(turn.user).font(Scaled.prose(scale)).textSelection(.enabled)
                    .padding(8)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
                Spacer(minLength: 0)
                if let at = clock(turn.atMs) {
                    Text(at).font(Scaled.caption2(scale)).foregroundStyle(.secondary)
                }
            }
            if !turn.activity.isEmpty {
                DisclosureGroup(isExpanded: $open) {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(turn.activity) { activity in
                            ActivityRow(activity: activity)
                        }
                    }
                    .padding(.leading, 8)
                } label: {
                    Text(summary).font(Scaled.caption(scale)).foregroundStyle(.secondary)
                }
                // The page carries the head of each tool call's text;
                // opening the activity fetches the turn whole.
                .onChange(of: open) { _, open in
                    if open, turn.activity.contains(where: \.more) {
                        Task { await model.loadTurn(ref, n: turn.n) }
                    }
                }
            }
            if turn.finalText.isEmpty {
                Text(turn.endMs == nil ? "working…" : "(no answer)")
                    .font(Scaled.callout(scale)).foregroundStyle(.secondary)
            } else {
                AgentText(text: turn.finalText)
            }
            if turn.truncated {
                Text("Cut at the host's turn cap.").font(Scaled.caption2(scale)).foregroundStyle(.secondary)
            }
        }
    }

    private var summary: String {
        var parts: [String] = []
        if turn.tools > 0 { parts.append("\(turn.tools) tool\(turn.tools == 1 ? "" : "s")") }
        if turn.errors > 0 { parts.append("\(turn.errors) failed") }
        if turn.thinking > 0 { parts.append("thought \(turn.thinking)×") }
        let system = turn.activity.filter { $0.kind == .system }.count
        if system > 0 { parts.append("\(system) system") }
        return parts.isEmpty ? "\(turn.activity.count) steps" : parts.joined(separator: ", ")
    }
}

/// One activity line; a tool call opens on its input and result.
struct ActivityRow: View {
    let activity: ActivityView
    @State private var open = false
    @Environment(\.textScale) private var scale

    var body: some View {
        if let detail = activity.detail {
            DisclosureGroup(isExpanded: $open) {
                VStack(alignment: .leading, spacing: 4) {
                    if !detail.input.isEmpty {
                        Text(detail.input).font(Scaled.mono(scale)).textSelection(.enabled)
                    }
                    if !detail.result.isEmpty {
                        Divider()
                        Text(detail.result).font(Scaled.mono(scale)).textSelection(.enabled)
                    }
                    if activity.more {
                        Text("Loading the rest…").font(Scaled.caption2(scale)).foregroundStyle(.secondary)
                    }
                }
                .padding(.leading, 8)
            } label: {
                line
            }
        } else {
            line
        }
    }

    private var line: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: symbol).font(Scaled.caption2(scale))
            Text(activity.text ?? activity.line).font(Scaled.callout(scale))
                .foregroundStyle(activity.error ? AnyShapeStyle(.red) : AnyShapeStyle(.primary))
                .textSelection(.enabled)
        }
    }

    private var symbol: String {
        switch activity.kind {
        case .tool: return activity.error ? "xmark.circle" : "wrench"
        case .text: return "text.alignleft"
        case .system: return "gearshape"
        }
    }
}

/// A command's or service's runs and the tail of one run's log, live
/// through a watch on the run chosen (the latest until another is).
struct RunOutputView: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    let scale: Double
    @State private var chosen: Int?

    private var runs: [RunView] { model.runs[ref] ?? [] }
    private var current: RunRef? {
        (chosen ?? runs.last?.n).map { RunRef(session: ref, n: $0) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if runs.isEmpty {
                VStack {
                    Spacer()
                    Text("No runs yet.").foregroundStyle(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            } else {
                runPicker
                if let current {
                    RunTail(model: model, run: current, scale: scale)
                }
            }
        }
        .task(id: ref) { await model.readRuns(ref) }
    }

    private var runPicker: some View {
        HStack(spacing: 6) {
            Picker("Run", selection: Binding(get: { current?.n ?? 0 }, set: { chosen = $0 })) {
                ForEach(runs) { run in
                    Text("Run \(run.n)").tag(run.n)
                }
            }
            .pickerStyle(.menu)
            .controlSize(.mini)
            if let run = runs.first(where: { $0.n == current?.n }) {
                Text(status(of: run)).font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    private func status(of run: RunView) -> String {
        var parts: [String] = []
        if let at = clock(run.startedMs) { parts.append("started \(at)") }
        if run.open {
            parts.append("running")
        } else if let exit = run.exit {
            parts.append(exit == 0 ? "exit 0" : "exit \(exit)")
        } else {
            parts.append("ended")
        }
        if !run.artifacts.isEmpty { parts.append("\(run.artifacts.count) files") }
        return parts.joined(separator: " · ")
    }
}

/// The tail of one run's log, watched while on screen.
struct RunTail: View {
    @Bindable var model: BrowserModel
    let run: RunRef
    let scale: Double

    var body: some View {
        Group {
            if let output = model.runOutputs[run] {
                GeometryReader { viewport in
                    ScrollViewReader { proxy in
                        ScrollView([.vertical, .horizontal]) {
                            VStack(alignment: .leading, spacing: 0) {
                                Text(output.text)
                                    .font(.system(size: ScreenScale.basePoints * scale, design: .monospaced))
                                    .textSelection(.enabled)
                                Color.clear.frame(height: 1).id("end")
                            }
                            .frame(minWidth: viewport.size.width, minHeight: viewport.size.height, alignment: .topLeading)
                        }
                        .onChange(of: output.text) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
                        .onAppear { proxy.scrollTo("end", anchor: .bottom) }
                    }
                }
                if output.truncated {
                    Text("Cut at the host's size cap.").font(.caption2).foregroundStyle(.secondary)
                }
            } else {
                VStack {
                    Spacer()
                    Text("No output: the run's log is gone, or has not been read.").foregroundStyle(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            }
        }
        .task(id: run) {
            await model.watchRunOutput(run)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
            await model.unwatchRunOutput(run)
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
