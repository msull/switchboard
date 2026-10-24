import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// A key of the keys panel: the punctuation, breaks, deletions and
/// caret moves a prompt needs after dictation, which the system
/// keyboard puts behind a full layout or a long reach. Undo and redo
/// are the host editor's and go through its tools.
public enum PanelKey: String, CaseIterable, Sendable {
    case period, comma, question, exclamation, colon, semicolon
    case newline, paragraph
    case deleteBackward, deleteWord
    case lineStart, left, right, lineEnd, start, end

    /// What the key inserts, for the keys that insert.
    public var inserted: String? {
        switch self {
        case .period: "."
        case .comma: ","
        case .question: "?"
        case .exclamation: "!"
        case .colon: ":"
        case .semicolon: ";"
        case .newline: "\n"
        case .paragraph: "\n\n"
        default: nil
        }
    }

    public var isPunctuation: Bool {
        switch self {
        case .period, .comma, .question, .exclamation, .colon, .semicolon: true
        default: false
        }
    }

    /// The rows the panel lays the keys out in.
    public static let rows: [[PanelKey]] = [
        [.period, .comma, .question, .exclamation, .colon, .semicolon],
        [.newline, .paragraph, .deleteBackward, .deleteWord],
        [.lineStart, .left, .right, .lineEnd, .start, .end],
    ]

    public var label: String {
        switch self {
        case .period: "Period"
        case .comma: "Comma"
        case .question: "Question mark"
        case .exclamation: "Exclamation mark"
        case .colon: "Colon"
        case .semicolon: "Semicolon"
        case .newline: "New line"
        case .paragraph: "New paragraph"
        case .deleteBackward: "Delete"
        case .deleteWord: "Delete word"
        case .lineStart: "Start of line"
        case .left: "Left"
        case .right: "Right"
        case .lineEnd: "End of line"
        case .start: "Start of text"
        case .end: "End of text"
        }
    }
}

/// A field's text with its selection as UTF-16 offsets, empty at the
/// caret: what a key edits. Offsets rather than string indices so the
/// state survives the text being replaced under it.
public struct EditState: Equatable, Sendable {
    public var text: String
    public var selection: Range<Int>

    public init(_ text: String, selection: Range<Int>) {
        self.text = text
        self.selection = EditState.clamp(selection, in: text)
    }

    public init(_ text: String, caret: Int) {
        self.init(text, caret: caret, length: 0)
    }

    private init(_ text: String, caret: Int, length: Int) {
        self.init(text, selection: caret..<(caret + length))
    }

    public var caret: Int { selection.upperBound }

    static func clamp(_ range: Range<Int>, in text: String) -> Range<Int> {
        let count = text.utf16.count
        let lower = min(max(range.lowerBound, 0), count)
        let upper = min(max(range.upperBound, lower), count)
        return lower..<upper
    }

    func index(_ offset: Int) -> String.Index {
        String.Index(utf16Offset: min(max(offset, 0), text.utf16.count), in: text)
    }

    func offset(_ index: String.Index) -> Int {
        index.utf16Offset(in: text)
    }

    var startIndex: String.Index { index(selection.lowerBound) }
    var endIndex: String.Index { index(selection.upperBound) }

    /// The character before the selection, and the one after it.
    var before: Character? {
        startIndex > text.startIndex ? text[text.index(before: startIndex)] : nil
    }
    var after: Character? {
        endIndex < text.endIndex ? text[endIndex] : nil
    }

    /// The selection replaced, the caret after what went in.
    mutating func replaceSelection(with inserted: String) {
        text.replaceSubrange(startIndex..<endIndex, with: inserted)
        let caret = selection.lowerBound + inserted.utf16.count
        selection = caret..<caret
    }

    mutating func delete(_ range: Range<Int>) {
        let range = EditState.clamp(range, in: text)
        text.removeSubrange(index(range.lowerBound)..<index(range.upperBound))
        selection = range.lowerBound..<range.lowerBound
    }

    /// Spaces and tabs on either side of the caret removed, so a mark
    /// or a break sits against the words.
    mutating func stripSpaces(before: Bool, after: Bool) {
        if before {
            var i = startIndex
            while i > text.startIndex, EditState.isSpace(text[text.index(before: i)]) {
                i = text.index(before: i)
            }
            let lower = offset(i)
            if lower < selection.lowerBound {
                text.removeSubrange(i..<startIndex)
                selection = lower..<(lower + selection.count)
            }
        }
        if after {
            var i = endIndex
            while i < text.endIndex, EditState.isSpace(text[i]) {
                i = text.index(after: i)
            }
            if i > endIndex {
                text.removeSubrange(endIndex..<i)
            }
        }
    }

    static func isSpace(_ c: Character) -> Bool { c == " " || c == "\t" }
}

/// What each key does to a field: pure, so a key is one function of
/// the state before it.
public enum KeyEdit {
    public static func apply(_ key: PanelKey, to state: EditState) -> EditState {
        var s = state
        if let inserted = key.inserted {
            if key.isPunctuation {
                // A mark goes against the word before it; a word after
                // the caret gets the space the mark usually precedes.
                s.stripSpaces(before: true, after: false)
                s.replaceSelection(with: inserted)
                if let next = s.after, !next.isWhitespace, !next.isPunctuation {
                    s.replaceSelection(with: " ")
                }
            } else {
                s.stripSpaces(before: true, after: true)
                if key == .paragraph, let last = s.before, last != "\n", !last.isPunctuation {
                    // A paragraph ends a sentence: the line gets its
                    // mark when it has none, so dictation can go on.
                    s.replaceSelection(with: ".")
                }
                s.replaceSelection(with: inserted)
            }
            return s
        }
        switch key {
        case .deleteBackward:
            if !s.selection.isEmpty {
                s.replaceSelection(with: "")
            } else if s.startIndex > s.text.startIndex {
                let from = s.offset(s.text.index(before: s.startIndex))
                s.delete(from..<s.selection.lowerBound)
            }
        case .deleteWord:
            if !s.selection.isEmpty {
                s.replaceSelection(with: "")
            } else {
                var i = s.startIndex
                while i > s.text.startIndex, s.text[s.text.index(before: i)].isWhitespace {
                    i = s.text.index(before: i)
                }
                while i > s.text.startIndex, !s.text[s.text.index(before: i)].isWhitespace {
                    i = s.text.index(before: i)
                }
                s.delete(s.offset(i)..<s.selection.lowerBound)
            }
        case .left:
            if !s.selection.isEmpty {
                s.selection = s.selection.lowerBound..<s.selection.lowerBound
            } else if s.startIndex > s.text.startIndex {
                let to = s.offset(s.text.index(before: s.startIndex))
                s.selection = to..<to
            }
        case .right:
            if !s.selection.isEmpty {
                s.selection = s.selection.upperBound..<s.selection.upperBound
            } else if s.endIndex < s.text.endIndex {
                let to = s.offset(s.text.index(after: s.endIndex))
                s.selection = to..<to
            }
        case .lineStart:
            var i = s.startIndex
            while i > s.text.startIndex, s.text[s.text.index(before: i)] != "\n" {
                i = s.text.index(before: i)
            }
            let to = s.offset(i)
            s.selection = to..<to
        case .lineEnd:
            var i = s.endIndex
            while i < s.text.endIndex, s.text[i] != "\n" {
                i = s.text.index(after: i)
            }
            let to = s.offset(i)
            s.selection = to..<to
        case .start:
            s.selection = 0..<0
        case .end:
            let to = s.text.utf16.count
            s.selection = to..<to
        default:
            break
        }
        return s
    }
}

/// The line the caret is on, cut to what a strip above the keys can
/// show: the text before the selection (its tail), the selection, and
/// the text after it (its head), each cut marked with an ellipsis.
public struct LinePreview: Equatable, Sendable {
    public var before: String
    public var selected: String
    public var after: String

    public init(before: String, selected: String, after: String) {
        self.before = before
        self.selected = selected
        self.after = after
    }
}

extension KeyEdit {
    /// The caret's line of `state`, keeping `keep` characters before
    /// the selection and `keepAfter` after it.
    public static func preview(_ state: EditState, keep: Int = 36, keepAfter: Int = 14) -> LinePreview {
        let text = state.text
        var lineStart = state.startIndex
        while lineStart > text.startIndex, text[text.index(before: lineStart)] != "\n" {
            lineStart = text.index(before: lineStart)
        }
        var lineEnd = state.endIndex
        while lineEnd < text.endIndex, text[lineEnd] != "\n" {
            lineEnd = text.index(after: lineEnd)
        }
        let before = text[lineStart..<state.startIndex]
        let after = text[state.endIndex..<lineEnd]
        return LinePreview(
            before: before.count > keep ? "…" + before.suffix(keep) : String(before),
            selected: String(text[state.startIndex..<state.endIndex]),
            after: after.count > keepAfter ? after.prefix(keepAfter) + "…" : String(after))
    }
}

/// A session's prompt field as the device keeps it: the text the
/// card's field shows and where its caret is, one per session and
/// shared by every window that edits it (the card's control bar and
/// the keys panel), with the host sync of a hosted draft. On a host
/// with a draft the field's text is a copy of the editor's; typing and
/// keys replace it whole once they pause.
@MainActor
@Observable
public final class EditField {
    public var text = "" {
        didSet { caret = EditState.clamp(caret, in: text) }
    }
    /// The field's selection as the text field keeps it; nil when the
    /// field has not placed one, and the last placed one is kept so a
    /// key pressed in another window still knows where the caret was.
    public var selection: TextSelection? {
        didSet {
            guard let selection else { return }
            switch selection.indices {
            case .selection(let range):
                take(range)
            case .multiSelection(let ranges):
                if let range = ranges.ranges.first { take(range) }
            @unknown default:
                break
            }
        }
    }

    /// A range the text field placed, as offsets. The field's indices
    /// are into its own copy of the text, which can be a different
    /// length from this one in the same frame (it reports a selection
    /// change before the text binding settles), and an index past the
    /// end traps when it is measured, so one that does not fit is left
    /// alone: the caret last placed stands until the next one.
    private func take(_ range: Range<String.Index>) {
        guard range.lowerBound >= text.startIndex, range.upperBound <= text.endIndex else { return }
        caret = EditState.clamp(range.lowerBound.utf16Offset(in: text)..<range.upperBound.utf16Offset(in: text), in: text)
    }
    /// The selection as offsets, empty at the caret; at the end until
    /// the field places one.
    public private(set) var caret: Range<Int> = 0..<0
    /// The host editor's text as last taken, so a change of the field
    /// that is not the host's is an edit to send; the edits are
    /// numbered so the reply to an older one never overwrites a newer
    /// one, and the host's text is applied only between edits.
    public internal(set) var hostText = ""
    var edit = 0
    var settled = 0
    /// What the last send of an edit said when it was not applied.
    public internal(set) var note: String?

    public init() {}

    public var editing: Bool { edit != settled }

    public var state: EditState { EditState(text, selection: caret) }

    /// A key pressed: the field is the state after it and the text
    /// field shows the caret there.
    public func press(_ key: PanelKey) {
        let next = KeyEdit.apply(key, to: state)
        text = next.text
        place(next.selection)
    }

    /// Text that arrived from outside the field, the host's or what
    /// was heard: a caret at the end stays at the end, so dictation
    /// keeps it in front; one inside the text keeps its place.
    public func set(_ new: String) {
        guard new != text else { return }
        let atEnd = caret.upperBound >= text.utf16.count
        text = new
        if atEnd {
            let end = new.utf16.count
            place(end..<end)
        } else {
            place(EditState.clamp(caret, in: new))
        }
    }

    func takeHost(_ hostText: String) {
        self.hostText = hostText
        set(hostText)
    }

    private func place(_ range: Range<Int>) {
        let range = EditState.clamp(range, in: text)
        caret = range
        let lower = String.Index(utf16Offset: range.lowerBound, in: text)
        let upper = String.Index(utf16Offset: range.upperBound, in: text)
        selection = TextSelection(range: lower..<upper)
    }
}

/// The keys window, one per app: a panel of keys placed as the system
/// keyboard is, low and close, that edits the field of the session
/// whose input is active, so a prompt can be punctuated and moved
/// through by touch after dictation without the full keyboard.
public enum KeysWindow {
    public static let id = "keys"
    public static let value = Key()

    /// The window's presented value, a type of its own, as the prompt
    /// window's is.
    public struct Key: Codable, Hashable, Sendable {
        public init() {}
    }
}

private extension View {
    /// Content kept above the window while `shown`, moving with it. Only
    /// the headset has ornaments; elsewhere the panel is not opened.
    @ViewBuilder
    func peek(_ shown: Bool, @ViewBuilder content: () -> some View) -> some View {
        #if os(visionOS)
        ornament(visibility: shown ? .visible : .hidden, attachmentAnchor: .scene(.top), contentAlignment: .bottom) {
            content()
                .glassBackgroundEffect()
                .padding(.bottom, 8)
        }
        #else
        self
        #endif
    }
}

/// The keys panel: three rows of keys the size of the system
/// keyboard's, with undo and redo where the host keeps the draft, under
/// a strip with the caret's line and a row with the session's name, a
/// microphone of the panel's own, a peek at the whole prompt and the
/// AI pass.
public struct KeysPanelView: View {
    @Bindable var model: BrowserModel
    @State private var held: SessionRef?
    @State private var busy = false
    @State private var note: String?
    @State private var dictation = Dictation()
    @State private var peeking = false

    public init(model: BrowserModel) {
        self.model = model
    }

    private var ref: SessionRef? { model.activeInput }
    private var item: UnifiedSession? {
        guard let ref else { return nil }
        return model.store.sessions.first { $0.ref == ref }
    }
    private var hosted: Bool { ref.map(model.hasDraft) ?? false }
    private var draft: DraftView? {
        guard let ref, hosted else { return nil }
        return model.drafts[ref]
    }
    private var enabled: Bool {
        guard let ref, let item else { return false }
        let standing = CardStanding.of(ref, in: model.store, removed: model.removed)
        return standing.controlsEnabled && model.scope(for: ref) == .control && !busy && item.session.kind == .claude
    }

    /// The host's editor is listening, through this panel's microphone
    /// or another window's.
    private var listening: Bool { dictation.listening || (draft?.listening ?? false) }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // The top row is as wide as a row of keys and no wider, so
            // the window, sized to its content, is the keys' width.
            HStack(spacing: Self.gap) {
                Text(note ?? caption)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                micButton
                peekButton
                smallTool(.cleanUp, "sparkles", "Clean up with AI", on: draft?.ai ?? false, working: draft?.busy == true)
            }
            .frame(width: Self.rowWidth)
            // The caret's line, as the system keyboard shows the text
            // it is typing into, so several changes can be made in a
            // row without looking back at the card.
            Text(line)
                .font(.callout)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(width: Self.rowWidth - 16, alignment: .leading)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            ForEach(Array(PanelKey.rows.enumerated()), id: \.offset) { row, keys in
                HStack(spacing: Self.gap) {
                    ForEach(keys, id: \.self) { key in
                        keyButton(key)
                    }
                    if row == 1 {
                        toolButton(.undo, "arrow.uturn.backward", "Undo", on: draft?.canUndo ?? false)
                        toolButton(.redo, "arrow.uturn.forward", "Redo", on: draft?.canRedo ?? false)
                    }
                }
            }
        }
        .padding(16)
        .peek(peeking) { peek }
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
            LaunchLog.note("keys appeared")
            dictation.onEvents = { recognizer, events in
                guard let ref = await model.activeInput, await model.hasDraft(ref) else { return }
                _ = await model.dictate(ref, recognizer: recognizer, events: events)
            }
        }
        .onDisappear {
            LaunchLog.note("keys disappeared")
            dictation.stop()
            if let held {
                Task { await model.releaseDraft(held) }
            }
        }
    }

    /// The microphone: red while the host's editor listens, from here
    /// or from a card; a tap starts or stops this panel's own, so a
    /// listen begun in a card is stopped there.
    private var micButton: some View {
        Button {
            note = nil
            Task { await dictation.toggle() }
        } label: {
            Image(systemName: listening ? "waveform" : "mic.fill")
                .symbolEffect(.variableColor.iterative, isActive: listening)
                .frame(width: Self.smallWidth, height: Self.smallHeight)
        }
        .buttonStyle(.bordered)
        .foregroundStyle(listening ? AnyShapeStyle(.red) : AnyShapeStyle(.primary))
        .disabled(!enabled || !hosted || (listening && !dictation.listening))
        .accessibilityLabel(dictation.listening ? "Stop dictation" : "Dictate")
        .help(dictation.listening ? "Stop dictation" : "Dictate into the prompt")
    }

    /// The toggle that keeps the whole prompt open above the panel
    /// (`peek`), until it is pressed again.
    private var peekButton: some View {
        Toggle(isOn: $peeking) {
            Image(systemName: "text.viewfinder")
                .frame(width: Self.smallWidth, height: Self.smallHeight)
        }
        .toggleStyle(.button)
        .buttonStyle(.bordered)
        .disabled(ref == nil)
        .accessibilityLabel("Peek at the prompt")
        .help("The whole prompt, above the keys, until pressed again")
    }

    /// The whole prompt, shown above the panel while the peek is on.
    /// On the headset it is an ornament: attached to the window's top
    /// edge, so it goes wherever the panel is dragged and stays open
    /// while the keys are pressed, and the panel itself never changes
    /// size or place.
    private var peek: some View {
        ScrollView {
            Text(whole)
                .font(.body)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .padding()
        }
        .frame(width: Self.rowWidth + 48)
        .frame(minHeight: 80, maxHeight: 420)
    }

    /// The whole prompt for the peek: the host's draft with the live
    /// utterance marked, or the device's field on a host without one.
    private var whole: AttributedString {
        if let draft {
            return draft.text.isEmpty ? Self.empty : PromptWindowView.marked(draft)
        }
        guard let ref, let field = model.fields[ref], !field.text.isEmpty else { return Self.empty }
        return AttributedString(field.text)
    }

    private static var empty: AttributedString {
        var empty = AttributedString("(empty)")
        empty.foregroundColor = .secondary
        return empty
    }

    /// The caret's line with the caret drawn in, or the selection
    /// marked, in the panel's text strip.
    private var line: AttributedString {
        guard let ref, let field = model.fields[ref], !field.text.isEmpty else { return Self.empty }
        let preview = KeyEdit.preview(field.state)
        var text = AttributedString(preview.before)
        if preview.selected.isEmpty {
            var caret = AttributedString("|")
            caret.foregroundColor = Color(red: 1.0, green: 0.78, blue: 0.43)
            text += caret
        } else {
            var selected = AttributedString(preview.selected)
            selected.backgroundColor = Color(red: 1.0, green: 0.78, blue: 0.43).opacity(0.4)
            text += selected
        }
        text += AttributedString(preview.after)
        return text
    }

    private var caption: String {
        guard let item else { return "No session bound" }
        guard enabled || busy else { return "\(item.session.name): not editable now" }
        return item.session.name
    }

    static let gap = 6.0
    static let keyWidth = 56.0
    static let keyHeight = 48.0
    /// The buttons of the top row, beside the session's name.
    static let smallWidth = 22.0
    static let smallHeight = 22.0
    /// Every row is six keys wide (the second with undo and redo).
    static let rowWidth = 6 * keyWidth + 5 * gap

    private func keyButton(_ key: PanelKey) -> some View {
        Button {
            guard let ref else { return }
            note = nil
            model.press(key, on: ref)
        } label: {
            keyLabel(key).frame(width: Self.keyWidth, height: Self.keyHeight)
        }
        .buttonStyle(.bordered)
        .disabled(!enabled)
        .accessibilityLabel(key.label)
        .help(key.label)
    }

    @ViewBuilder
    private func keyLabel(_ key: PanelKey) -> some View {
        switch key {
        case .period, .comma, .question, .exclamation, .colon, .semicolon:
            Text(key.inserted ?? "").font(.title.weight(.medium))
        case .newline: Image(systemName: "return")
        case .paragraph: Image(systemName: "paragraphsign")
        case .deleteBackward: Image(systemName: "delete.left")
        // The filled symbol is the word key: every key is one glyph, so
        // every key is the same size.
        case .deleteWord: Image(systemName: "delete.left.fill")
        case .lineStart: Image(systemName: "arrow.left.to.line")
        case .left: Image(systemName: "arrow.left")
        case .right: Image(systemName: "arrow.right")
        case .lineEnd: Image(systemName: "arrow.right.to.line")
        case .start: Image(systemName: "arrow.up.to.line")
        case .end: Image(systemName: "arrow.down.to.line")
        }
    }

    private func toolButton(_ tool: DraftTool, _ symbol: String, _ label: String, on: Bool) -> some View {
        Button { apply(tool) } label: {
            Image(systemName: symbol).frame(width: Self.keyWidth, height: Self.keyHeight)
        }
        .buttonStyle(.bordered)
        .disabled(!enabled || !hosted || !on)
        .accessibilityLabel(label)
        .help(label)
    }

    /// A tool in the top row. While the host is working on it the
    /// button stays where it is, the same size, with a spinner in place
    /// of its symbol, so the buttons beside it never move under a
    /// finger on its way to them.
    private func smallTool(_ tool: DraftTool, _ symbol: String, _ label: String, on: Bool, working: Bool = false) -> some View {
        Button { apply(tool) } label: {
            ZStack {
                Image(systemName: symbol).opacity(working ? 0 : 1)
                if working {
                    ProgressView().controlSize(.small)
                }
            }
            .frame(width: Self.smallWidth, height: Self.smallHeight)
        }
        .buttonStyle(.bordered)
        .disabled(!enabled || !hosted || !on || working)
        .accessibilityLabel(label)
        .help(label)
    }

    /// One of the host editor's tools, after what is being heard has
    /// reached the host.
    private func apply(_ tool: DraftTool) {
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
}
