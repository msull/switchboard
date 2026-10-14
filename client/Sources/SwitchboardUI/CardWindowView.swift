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
    @Environment(\.openWindow) private var openWindow
    @Environment(\.scenePhase) private var scenePhase

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
                if item.session.kind == .claude {
                    ControlBar(
                        model: model, ref: item.ref, session: item.session,
                        enabled: standing.controlsEnabled, scope: model.scope(for: item.ref))
                }
            } else {
                Spacer()
            }
        }
        .padding()
        .navigationTitle(item?.session.name ?? "Session")
        .onAppear {
            LaunchLog.note("card \(request.id) appeared")
            mode = model.mode(for: request.id)
            model.windowAppeared(request)
        }
        .onDisappear {
            LaunchLog.note("card \(request.id) disappeared")
            model.windowClosed(request)
        }
        .onChange(of: scenePhase) { _, phase in LaunchLog.note("card \(request.id) phase \(phase)") }
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
            Spacer(minLength: 4)
            if let item {
                SourceLabel(name: item.serverName, stale: item.stale, size: .small)
            }
            // Icon only, and small: a tall narrow card has no room for
            // a word here.
            Button {
                openWindow(id: PromptWindow.id, value: PromptWindow.value)
            } label: {
                Image(systemName: "text.alignleft")
                    .font(.caption)
            }
            .buttonStyle(.bordered)
            .buttonBorderShape(.circle)
            .controlSize(.mini)
            .accessibilityLabel("Prompt window")
            .help("The prompt window: the whole draft of the session being worked on")
            Button {
                LaunchLog.note("card \(request.id) sessions button opens main")
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

    private var scaleControl: some View {
        ScaleControl(scale: $screenScale)
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
            ConversationView(model: model, ref: item.ref, window: request.id)
                .environment(\.textScale, screenScale)
        case .runOutput:
            RunOutputView(model: model, ref: item.ref, scale: screenScale)
        }
    }
}

/// A text scale: a small readout that opens a slider and the snap
/// sizes, kept out of the window's bottom edge where the system's own
/// controls sit.
struct ScaleControl: View {
    @Binding var scale: Double
    @State private var popover = false

    var body: some View {
        Button {
            popover.toggle()
        } label: {
            Label(ScreenScale.label(scale), systemImage: "textformat.size")
                .font(.caption2.monospacedDigit())
        }
        .buttonStyle(.bordered)
        .controlSize(.mini)
        .accessibilityLabel("Text size")
        .popover(isPresented: $popover) {
            VStack(spacing: 12) {
                HStack(spacing: 6) {
                    Image(systemName: "textformat.size.smaller")
                    Slider(value: $scale, in: ScreenScale.range)
                    Image(systemName: "textformat.size.larger")
                }
                .foregroundStyle(.secondary)
                HStack(spacing: 6) {
                    ForEach(ScreenScale.presets, id: \.self) { preset in
                        Button(ScreenScale.label(preset)) { scale = preset }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            .fixedSize()
                            .tint(preset == scale ? .accentColor : nil)
                    }
                }
            }
            .padding()
            .frame(width: 440)
        }
    }
}

/// The time of a wire millisecond stamp, short.
func clock(_ ms: UInt64?) -> String? {
    ms.map { Date(timeIntervalSince1970: Double($0) / 1000).formatted(date: .omitted, time: .shortened) }
}

/// The controls of a Claude Code session (§4.5): the prompt, the
/// folder question answered, and a resume for one that is not running,
/// which spends a turn and so asks first. On a host that keeps a draft
/// the field is the host's Prompt Box editor: what is typed replaces
/// its text, dictation goes to it as speech events, and its tools
/// (undo, redo, the last sentence deleted, clear, the AI pass) sit in
/// the row above; on an older host the field is the device's own and
/// dictation lands in it as text. Only a device with the control scope
/// can use any of it.
struct ControlBar: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    let session: SessionView
    let enabled: Bool
    let scope: Scope?
    @State private var draft = ""
    @State private var busy = false
    @State private var note: String?
    @State private var confirmResume = false
    @State private var dictation = Dictation()
    /// The field's text when dictation started, on a host without a
    /// draft: what is heard goes after it, revised in place.
    @State private var typedBeforeDictation = ""
    /// On a host with a draft: the text last taken from it, so a change
    /// of the field that is not the host's is an edit to send; the
    /// edits are numbered so the reply to an older one never overwrites
    /// a newer one, and the host's text is applied only between edits.
    @State private var hostText = ""
    @State private var edit = 0
    @State private var settled = 0

    private var canControl: Bool { enabled && scope == .control && !busy }
    private var hosted: Bool { model.hasDraft(ref) }
    private var hostDraft: DraftView? { hosted ? model.drafts[ref] : nil }
    private var editing: Bool { edit != settled }
    private var empty: Bool { draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if scope != .control {
                Text("This device reads only. Raise its scope on the host.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                if session.trustQuestion || (session.liveness != .running && session.resumeID != nil) {
                HStack(spacing: 8) {
                    if session.trustQuestion {
                        Button("Trust this folder") { run { await model.trust(ref) } }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .disabled(!canControl)
                    }
                    if session.liveness != .running, session.resumeID != nil {
                        Button("Resume…") { confirmResume = true }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            .disabled(!canControl)
                            .confirmationDialog(
                                "Resume spends a turn", isPresented: $confirmResume, titleVisibility: .visible
                            ) {
                                Button("Resume") { run { await model.resume(ref) } }
                            }
                    }
                }
                }
                // The editor's tools sit over the field, at its left,
                // away from Send so a tap at one is not the other.
                if hosted { DraftTools(draft: hostDraft, enabled: canControl, apply: tool) }
                HStack(spacing: 8) {
                    HStack(alignment: .bottom, spacing: 6) {
                        Button {
                            if !dictation.listening { typedBeforeDictation = draft }
                            Task { await dictation.toggle() }
                        } label: {
                            Image(systemName: dictation.listening ? "waveform" : "mic.fill")
                                .symbolEffect(.variableColor.iterative, isActive: dictation.listening)
                                .frame(width: 22, height: 22)
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(dictation.listening ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                        .disabled(!canControl)
                        .accessibilityLabel(dictation.listening ? "Stop dictation" : "Dictate")
                        TextField("Message", text: $draft, axis: .vertical)
                            .lineLimit(2...5)
                            .textFieldStyle(.plain)
                            .onSubmit(send)
                            .disabled(!canControl)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                    .onChange(of: dictation.heard) { _, heard in
                        guard !hosted else { return }
                        draft = Dictation.joined(typedBeforeDictation, heard)
                    }
                    .onChange(of: draft) { _, draft in
                        if hosted { hostEdited(draft) } else { localEdited(draft) }
                    }
                    .onChange(of: hostDraft) { _, host in
                        guard let host, !editing, host.text != draft else { return }
                        hostText = host.text
                        draft = host.text
                    }
                    .onChange(of: dictation.problem) { _, problem in
                        if let problem { note = problem }
                    }
                    Button(action: send) {
                        Image(systemName: "paperplane.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(!canControl || empty)
                    .accessibilityLabel("Send")
                }
                DraftTools.captionLine(note ?? hostDraft.flatMap(DraftTools.caption))
            }
        }
        .task(id: hosted) {
            guard hosted else { return }
            await model.holdDraft(ref)
            if let host = model.drafts[ref], !editing {
                hostText = host.text
                draft = host.text
            }
        }
        .onAppear {
            dictation.onEvents = { recognizer, events in
                guard await model.hasDraft(ref) else { return }
                _ = await model.dictate(ref, recognizer: recognizer, events: events)
            }
        }
        .onDisappear {
            dictation.stop()
            if hosted { Task { await model.releaseDraft(ref) } }
        }
    }

    /// An edit by hand on a host without a draft, while listening: the
    /// text is the field's now, and what is heard next goes after it.
    /// The live utterance goes on revising when it is still whole at
    /// the end.
    private func localEdited(_ draft: String) {
        guard dictation.listening, draft != Dictation.joined(typedBeforeDictation, dictation.heard)
        else { return }
        let live = dictation.live.trimmingCharacters(in: .whitespaces)
        if !live.isEmpty, draft.hasSuffix(live) {
            typedBeforeDictation = String(draft.dropLast(live.count))
            dictation.absorb(keepingLive: true)
        } else {
            typedBeforeDictation = draft
            dictation.absorb(keepingLive: false)
        }
    }

    /// An edit by hand on a host with a draft: sent whole once the
    /// typing pauses, as the editor's replace. The host's reply is the
    /// text now; a newer edit outranks it.
    private func hostEdited(_ text: String) {
        guard text != hostText else { return }
        edit += 1
        let mine = edit
        Task {
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard mine == edit else { return }
            let outcome = await model.replaceDraft(ref, text: text)
            guard mine == edit else { return }
            settled = mine
            switch outcome {
            case .applied(let host):
                hostText = host.text
                if host.text != draft { draft = host.text }
            default:
                note = outcome.note
            }
        }
    }

    private func tool(_ tool: DraftTool) {
        busy = true
        note = nil
        Task {
            await dictation.settle()
            let outcome = await model.draftAction(ref, tool)
            note = outcome.note
            busy = false
        }
    }

    private func send() {
        dictation.stop()
        guard !empty, canControl else { return }
        if hosted {
            busy = true
            note = nil
            Task {
                await dictation.settle()
                if draft != hostText {
                    // The typing has not reached the host yet.
                    edit += 1
                    settled = edit
                    _ = await model.replaceDraft(ref, text: draft)
                }
                let outcome = await model.sendDraft(ref)
                note = outcome.note
                if case .applied(let host) = outcome {
                    hostText = host.text
                    draft = host.text
                }
                busy = false
            }
            return
        }
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        run {
            let outcome = await model.send(ref, text: text)
            if outcome == .accepted { draft = "" }
            return outcome
        }
    }

    private func run(_ command: @escaping () async -> CommandOutcome) {
        busy = true
        note = nil
        Task {
            let outcome = await command()
            note = outcome.note
            busy = false
        }
    }
}

/// The editor's tools on a draft, as icons: undo, redo, the last
/// sentence deleted, clear, and the AI pass where the host has a key.
struct DraftTools: View {
    let draft: DraftView?
    let enabled: Bool
    let apply: (DraftTool) -> Void

    var body: some View {
        HStack(spacing: 2) {
            button(.undo, "arrow.uturn.backward", "Undo", on: draft?.canUndo ?? false)
            button(.redo, "arrow.uturn.forward", "Redo", on: draft?.canRedo ?? false)
            button(.deleteSentence, "delete.left", "Delete the last sentence", on: !(draft?.text.isEmpty ?? true))
            button(.clear, "xmark.circle", "Clear", on: !(draft?.text.isEmpty ?? true))
            if draft?.busy == true {
                ProgressView().controlSize(.small).frame(width: 28, height: 22)
            } else {
                button(.cleanUp, "sparkles", "Clean up with AI", on: draft?.ai ?? false)
            }
        }
    }

    private func button(_ tool: DraftTool, _ symbol: String, _ label: String, on: Bool) -> some View {
        Button { apply(tool) } label: {
            Image(systemName: symbol).frame(width: 28, height: 22)
        }
        .buttonStyle(.plain)
        .foregroundStyle(on && enabled ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
        .disabled(!on || !enabled)
        .accessibilityLabel(label)
        .help(label)
    }

    /// What the line under the field says of a draft: a trouble, or
    /// the AI pass running; listening and finishing are the field's
    /// own business and say nothing.
    static func caption(_ draft: DraftView) -> String? {
        switch draft.status {
        case "degraded", "error": return draft.statusText ?? draft.status
        default: return draft.busy ? "The AI pass is running on the host." : nil
        }
    }

    /// The line under the field, one line tall whether or not it has
    /// something to say, so a note never shifts the field.
    static func captionLine(_ text: String?) -> some View {
        Text(text ?? " ")
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
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

/// Text the agent wrote: its inline Markdown read where it parses and
/// shown plain where it does not, and its tables laid out as grids.
struct AgentText: View {
    let text: String
    @Environment(\.textScale) private var scale

    var body: some View {
        let segments = MarkdownTable.split(text)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
                switch segment {
                case .prose(let prose):
                    Text(Self.inline(prose))
                        .font(Scaled.prose(scale))
                        .textSelection(.enabled)
                case .table(let table):
                    MarkdownTableView(table: table)
                }
            }
        }
    }

    /// The text with its inline marks read, or as written when they
    /// do not parse.
    static func inline(_ text: String) -> AttributedString {
        (try? AttributedString(
            markdown: text,
            options: AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(text)
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
    /// The window, whose place in the conversation is kept.
    let window: UUID
    @State private var loading = false
    /// Where the view stands: the turn at the top, or the end it
    /// follows, and the folds open. Seeded from the window's saved
    /// place and kept across merges and reconnects; a turn's id is its
    /// number, so a page replacing the turns under the view keeps the
    /// one at the top in place.
    @State private var place: ConversationPlace?
    /// The turn at the top of the view, as the scroll view reports it.
    @State private var top: Int?
    /// The end was asked for and the view is not there yet: a scroll
    /// still decelerating overrides the first scroll, so the end is
    /// asked for again when it settles.
    @State private var pulling = false
    /// The end marker is on screen, as the scroll view reports it.
    @State private var atEnd = true

    /// The ids in the scroll target layout that are not turns.
    private static let earlierID = 0
    private static let endID = Int.max

    var body: some View {
        Group {
            if let conversation = model.conversations[ref] {
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            LazyVStack(alignment: .leading, spacing: 14) {
                                if conversation.hasEarlier {
                                    Button {
                                        loading = true
                                        let keep = conversation.firstHeld
                                        Task {
                                            await model.loadEarlier(ref)
                                            // The turns go in above; the one that was
                                            // at the top stays there.
                                            if let keep { top = keep }
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
                                    .id(Self.earlierID)
                                }
                                ForEach(conversation.turns) { turn in
                                    TurnRow(
                                        model: model, ref: ref, turn: turn,
                                        open: fold(turn.n),
                                        openActivity: { index in fold(ActivityKey(turn: turn.n, index: index)) })
                                }
                            }
                            .scrollTargetLayout()
                            // The end: following is any of this marker on
                            // screen, so a bounce or a finger lifted a hair
                            // early still follows, and whatever the view's
                            // insets are, a scroll to it reaches it. It sits
                            // outside the lazy stack, which would make and
                            // drop it as the view moves and lose its reports.
                            Color.clear.frame(height: ConversationPlace.endSlack)
                                .id(Self.endID)
                                .onScrollVisibilityChange(threshold: 0.02) { visible in atEnd = visible }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .scrollPosition(id: $top, anchor: .top)
                    // The view opens at the end, laid out from the bottom
                    // rather than scrolled there across rows not yet
                    // measured; and the field under the view growing
                    // shrinks the view, where the end stays in view.
                    .defaultScrollAnchor(.bottom)
                    .defaultScrollAnchor(.bottom, for: .sizeChanges)
                    .onChange(of: atEnd) { _, visible in
                        place?.following = visible
                        if visible { pulling = false }
                    }
                    .onScrollPhaseChange { _, phase in
                        if phase == .idle, pulling {
                            withAnimation { proxy.scrollTo(Self.endID, anchor: .bottom) }
                        }
                    }
                    .overlay(alignment: .bottom) {
                        if !atEnd {
                            Button {
                                pulling = true
                                withAnimation { proxy.scrollTo(Self.endID, anchor: .bottom) }
                            } label: {
                                Image(systemName: "ellipsis")
                            }
                            .buttonStyle(.bordered)
                            .buttonBorderShape(.circle)
                            .controlSize(.small)
                            .accessibilityLabel("Follow the end of the conversation")
                            .padding(.bottom, 6)
                        }
                    }
                    .onAppear { restore(conversation, proxy) }
                    .onChange(of: conversation.resumeID) { _, _ in restore(conversation, proxy) }
                    .onChange(of: conversation.lastHeld) { _, _ in follow(proxy) }
                    .onChange(of: conversation.turns.last?.finalText) { _, _ in follow(proxy) }
                    .onChange(of: top) { _, new in
                        if let new, new != Self.earlierID, new != Self.endID { place?.turn = new }
                    }
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
        .onChange(of: place?.openTurns) { _, _ in remember() }
        .onChange(of: place?.openActivities) { _, _ in remember() }
        .onDisappear { remember() }
    }

    /// Put the view where the window stood: the end it follows, or
    /// the turn it had at the top, when that turn is still held.
    private func restore(_ conversation: ConversationState, _ proxy: ScrollViewProxy) {
        let saved = place ?? model.place(for: window) ?? ConversationPlace(resumeID: conversation.resumeID)
        let fitted = saved.fitted(to: conversation)
        place = fitted
        if let turn = fitted.target(in: conversation) {
            top = turn
            proxy.scrollTo(turn, anchor: .top)
        } else {
            proxy.scrollTo(Self.endID, anchor: .bottom)
        }
    }

    /// The end moved: follow it only when the view was there.
    private func follow(_ proxy: ScrollViewProxy) {
        if place?.following ?? true {
            proxy.scrollTo(Self.endID, anchor: .bottom)
        }
    }

    private func remember() {
        if let place { model.setPlace(place, for: window) }
    }

    private func fold(_ turn: Int) -> Binding<Bool> {
        Binding(
            get: { place?.openTurns.contains(turn) ?? false },
            set: { open in
                if open { place?.openTurns.insert(turn) } else { place?.openTurns.remove(turn) }
            })
    }

    private func fold(_ key: ActivityKey) -> Binding<Bool> {
        Binding(
            get: { place?.openActivities.contains(key) ?? false },
            set: { open in
                if open { place?.openActivities.insert(key) } else { place?.openActivities.remove(key) }
            })
    }
}

/// One turn: the prompt, the activity behind a disclosure, the answer.
struct TurnRow: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    let turn: TurnView
    /// The folds, owned by the window so they survive the turn being
    /// replaced under the row.
    @Binding var open: Bool
    let openActivity: (Int) -> Binding<Bool>
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
                            ActivityRow(activity: activity, open: openActivity(activity.index))
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
    @Binding var open: Bool
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
