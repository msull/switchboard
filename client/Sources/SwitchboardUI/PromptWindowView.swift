import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// The prompt window: the whole draft of the session whose input is
/// active, large, with the live utterance marked, and the editor's
/// tools, a microphone and Send, so a prompt can be followed and
/// worked on from one fixed place while the cards come and go, as the
/// desktop's preview panel is used with the Working Set. It binds to
/// the last session a control bar typed, dictated or used a tool in,
/// and follows that binding. It is a view of the host's editor and
/// nothing else changes with it open.
public struct PromptWindowView: View {
    @Bindable var model: BrowserModel
    @Environment(\.openWindow) private var openWindow
    @State private var dictation = Dictation()
    @State private var scale = ScreenScale.initial
    @State private var busy = false
    @State private var note: String?
    /// The session whose draft this window holds now.
    @State private var held: SessionRef?

    public init(model: BrowserModel) {
        self.model = model
    }

    private var ref: SessionRef? { model.activeInput }
    private var item: UnifiedSession? {
        guard let ref else { return nil }
        return model.store.sessions.first { $0.ref == ref }
    }
    private var standing: CardStanding? {
        ref.map { CardStanding.of($0, in: model.store, removed: model.removed) }
    }
    private var hosted: Bool { ref.map(model.hasDraft) ?? false }
    private var draft: DraftView? {
        guard let ref, hosted else { return nil }
        return model.drafts[ref]
    }
    private var canControl: Bool {
        guard let ref, let standing else { return false }
        return standing.controlsEnabled && model.scope(for: ref) == .control && hosted && !busy
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            if let banner = standing?.banner {
                Label(banner, systemImage: standing == .removed ? "trash" : "wifi.slash")
                    .font(.callout)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            }
            ScrollView {
                if let draft {
                    Text(Self.marked(draft))
                        .font(Scaled.prose(scale))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .topLeading)
                        .padding(.vertical, 4)
                } else if ref == nil {
                    Text("Type or dictate in a session's card, and its prompt appears here.")
                        .font(Scaled.prose(scale))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .topLeading)
                } else if !hosted {
                    Text("This host keeps no draft; the card's field is the prompt.")
                        .font(Scaled.prose(scale))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            HStack(spacing: 10) {
                Button {
                    Task { await dictation.toggle() }
                } label: {
                    Image(systemName: dictation.listening ? "waveform" : "mic.fill")
                        .symbolEffect(.variableColor.iterative, isActive: dictation.listening)
                        .frame(width: 28, height: 22)
                }
                .buttonStyle(.plain)
                .foregroundStyle(dictation.listening ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                .disabled(!canControl)
                .accessibilityLabel(dictation.listening ? "Stop dictation" : "Dictate")
                DraftTools(draft: draft, enabled: canControl, apply: tool)
                Spacer(minLength: 0)
                Button(action: send) {
                    Label("Send", systemImage: "paperplane.fill")
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .disabled(!canControl || (draft?.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true))
            }
            DraftTools.captionLine(note ?? draft.flatMap(DraftTools.caption))
        }
        .padding()
        .navigationTitle("Prompt")
        .task(id: ref) {
            if let held, held != ref {
                await model.releaseDraft(held)
                self.held = nil
            }
            if let ref, hosted, held == nil {
                await model.holdDraft(ref)
                held = ref
            }
        }
        .onChange(of: dictation.problem) { _, problem in
            if let problem { note = problem }
        }
        .onAppear {
            dictation.onEvents = { recognizer, events in
                guard let ref = await model.activeInput, await model.hasDraft(ref) else { return }
                _ = await model.dictate(ref, recognizer: recognizer, events: events)
            }
        }
        .onDisappear {
            dictation.stop()
            if let held {
                Task { await model.releaseDraft(held) }
            }
        }
        .task { await bringMainWindowBack() }
    }

    /// A launch that restored only this window has no browser on a
    /// system that does not present it: after the launch grace, it is
    /// opened from here, as a card window would.
    private func bringMainWindowBack() async {
        try? await Task.sleep(nanoseconds: LaunchGrace.nanoseconds)
        if model.claimMainWindow() {
            openWindow(id: MainWindow.id, value: MainWindow.value)
        }
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading) {
                if let item {
                    Text(item.session.name).font(.title2.bold())
                    Text("\(item.session.kind.rawValue) · \(item.session.card)")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    Text("Prompt").font(.title2.bold())
                    Text("No session bound").font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 4)
            if let item {
                SourceLabel(name: item.serverName, stale: item.stale, size: .small)
            }
            ScaleControl(scale: $scale)
        }
    }

    /// The draft's text with the live utterance in amber, as the
    /// desktop preview shows it.
    static func marked(_ draft: DraftView) -> AttributedString {
        var text = AttributedString(draft.text)
        if let range = draft.liveRange, let marked = Range(range, in: text) {
            text[marked].foregroundColor = Color(red: 1.0, green: 0.78, blue: 0.43)
        }
        return text
    }

    private func tool(_ tool: DraftTool) {
        guard let ref else { return }
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
        guard let ref, canControl else { return }
        dictation.stop()
        busy = true
        note = nil
        Task {
            await dictation.settle()
            let outcome = await model.sendDraft(ref)
            note = outcome.note
            busy = false
        }
    }
}
