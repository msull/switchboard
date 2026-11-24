import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol
import SwitchboardTransport

/// What one of a card window's side panes holds: a document, a second
/// session worked beside the first, or the files the session edited.
public enum SidePane: Hashable, Codable, Sendable {
    case document(FileRef)
    case session(SessionRef)
    /// The files the card's own session edited.
    case files
}

/// How long the files pane waits between reads while it is open. The
/// host keeps the list per version of the transcript, so a read that
/// finds nothing new costs it a file stat.
private let filesPoll: Duration = .seconds(4)

/// The left-hand pane of a card window: every file the session's tool
/// calls wrote under its project root, newest first, with the lines
/// added and removed since the branch split from the default branch (or
/// since the session started). A tap on the name opens the file; the
/// button beside it opens the diff.
struct EditedFilesPane: View {
    @Bindable var model: BrowserModel
    let ref: SessionRef
    let project: String
    /// Files that no longer differ from the base are left off the list.
    @Binding var hideUnchanged: Bool
    /// Empties the pane it is in, back to that pane's chooser.
    var close: (() -> Void)?
    @State private var body_: EditedFilesBody?
    @State private var failure: String?
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Files", systemImage: "doc.on.doc").font(.headline)
                Spacer()
                if let files = body_?.files, !files.isEmpty {
                    Text(totals(Self.shown(files, hideUnchanged: hideUnchanged)))
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                if let close {
                    Button(action: close) {
                        Image(systemName: "xmark").font(.caption)
                    }
                    .buttonStyle(.bordered)
                    .buttonBorderShape(.circle)
                    .controlSize(.mini)
                    .accessibilityLabel("Empty the pane")
                    .help("Put something else in this pane")
                }
            }
            if let base = body_?.base {
                Text(base.label).font(.caption).foregroundStyle(.secondary)
            }
            if let files = body_?.files {
                unchangedButton(files)
            }
            if let body_ {
                let shown = Self.shown(body_.files, hideUnchanged: hideUnchanged)
                if body_.files.isEmpty {
                    Text("No files written yet.").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                } else if shown.isEmpty {
                    Text("Every file is back as it was.").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 2) {
                            ForEach(shown) { file in
                                row(file, base: body_.base)
                            }
                        }
                    }
                }
            } else if let failure {
                Text(failure).font(.callout).foregroundStyle(.secondary)
                Spacer(minLength: 0)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding()
        .task(id: ref) {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: filesPoll)
            }
        }
    }

    private func load() async {
        do {
            let read = try await model.readEditedFiles(ref)
            if read != body_ { body_ = read }
            failure = nil
        } catch RemoteError.refused(_, "not_found", _) {
            failure = "This host has no files list yet: it needs a newer Switchboard."
        } catch RemoteError.refused(_, "no_conversation", let message) {
            failure = message
        } catch {
            // Offline or busy: what is held stays, and the next read retries.
            if body_ == nil { failure = "The host did not answer." }
        }
    }

    /// An unchanged file is one in the checkout that matches the base; a
    /// file git could not read is kept, since nothing says it matches.
    static func unchanged(_ file: EditedFile) -> Bool {
        file.inRepo && !file.changed
    }

    /// The list as shown: unchanged files left off when hidden, so one
    /// that changes again comes back on its own.
    static func shown(_ files: [EditedFile], hideUnchanged: Bool) -> [EditedFile] {
        hideUnchanged ? files.filter { !unchanged($0) } : files
    }

    @ViewBuilder
    private func unchangedButton(_ files: [EditedFile]) -> some View {
        let count = files.filter(Self.unchanged).count
        if count > 0 {
            Button(hideUnchanged ? "Show \(count) unchanged" : "Clear \(count) unchanged") {
                hideUnchanged.toggle()
            }
            .buttonStyle(.bordered)
            .controlSize(.mini)
            .help(hideUnchanged
                ? "List the files that match the base again"
                : "Leave off the files that match the base; one that changes again comes back")
        }
    }

    private func totals(_ files: [EditedFile]) -> String {
        let added = files.reduce(0) { $0 + $1.added }
        let removed = files.reduce(0) { $0 + $1.removed }
        return "\(files.count) · +\(added) −\(removed)"
    }

    private func row(_ file: EditedFile, base: DiffBase?) -> some View {
        let name = file.path.split(separator: "/").last.map(String.init) ?? file.path
        let dir = file.path.count > name.count ? String(file.path.dropLast(name.count + 1)) : ""
        return HStack(spacing: 8) {
            Button {
                let fileRef = FileRef(
                    serverID: ref.serverID, projectID: file.project ?? project, path: file.path)
                openWindow(value: model.fileWindowRequest(for: fileRef, line: nil))
            } label: {
                VStack(alignment: .leading, spacing: 0) {
                    Text(name).font(.callout).lineLimit(1).truncationMode(.middle)
                    if !dir.isEmpty {
                        Text(dir).font(.caption2).foregroundStyle(.secondary)
                            .lineLimit(1).truncationMode(.head)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Open \(file.path)")
            counts(file)
            if let base, file.inRepo, file.changed {
                Button {
                    openWindow(value: DiffWindowRequest(session: ref, project: project, base: base, path: file.path))
                } label: {
                    Image(systemName: "plusminus").font(.caption)
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.circle)
                .controlSize(.mini)
                .accessibilityLabel("Diff of \(name)")
                .help("The diff")
            }
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private func counts(_ file: EditedFile) -> some View {
        if file.project != nil {
            Text("other project").font(.caption2).foregroundStyle(.tertiary)
        } else if !file.inRepo {
            Text("no repo").font(.caption2).foregroundStyle(.tertiary)
        } else if file.binary {
            Text("binary").font(.caption2).foregroundStyle(.secondary)
        } else if !file.changed {
            Text("unchanged").font(.caption2).foregroundStyle(.tertiary)
        } else {
            HStack(spacing: 4) {
                Text("+\(file.added)").foregroundStyle(.green)
                Text("−\(file.removed)").foregroundStyle(.red)
            }
            .font(.caption.monospacedDigit())
        }
    }
}

/// The right-hand pane of a card window: empty, it offers the documents
/// the session wrote and the other sessions; filled, it shows the one
/// chosen, a document as a file window does and a session as a card.
struct SideSlotPane: View {
    @Bindable var model: BrowserModel
    let owner: SessionRef
    let project: String
    /// The session has edited files to list (a Claude session does).
    let files: Bool
    @Binding var content: SidePane?
    @Binding var hideUnchanged: Bool
    /// The embedded card's or file's own id, so its mode and watch
    /// belong to this pane and not to another window on the same thing.
    @State private var inner = UUID()

    /// What is shown: a session with no files list has the chooser where
    /// that list would be.
    private var shown: SidePane? { content == .files && !files ? nil : content }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch shown {
            case .files:
                EditedFilesPane(model: model, ref: owner, project: project, hideUnchanged: $hideUnchanged) {
                    content = nil
                }
            case .document(let ref):
                header(ref.path.split(separator: "/").last.map(String.init) ?? ref.path)
                FileWindowView(model: model, request: FileWindowRequest(id: inner, ref: ref), embedded: true)
            case .session(let ref):
                header(model.store.sessions.first { $0.ref == ref }?.session.name ?? "Session")
                CardWindowView(model: model, request: CardWindowRequest(id: inner, ref: ref), embedded: true)
            case nil:
                SlotChooser(model: model, owner: owner, project: project, files: files) { chosen in
                    inner = UUID()
                    if case .document(let ref) = chosen { model.noteOpened(ref) }
                    content = chosen
                }
            }
        }
        // The files list pads itself.
        .padding(shown == nil ? 16 : shown == .files ? 0 : 8)
    }

    private func header(_ title: String) -> some View {
        HStack {
            Text(title).font(.headline).lineLimit(1)
            Spacer()
            Button {
                content = nil
            } label: {
                Image(systemName: "xmark").font(.caption)
            }
            .buttonStyle(.bordered)
            .buttonBorderShape(.circle)
            .controlSize(.mini)
            .accessibilityLabel("Empty the pane")
            .help("Empty this pane")
        }
    }
}

/// What an empty right-hand pane offers: the Markdown files the session
/// wrote, then every other session, grouped by host.
/// What the slot's chooser offers, narrowed and ordered: a host keeps
/// every session it ever ran, so the whole list is hundreds of ended ones.
enum SlotChoices {
    /// The sessions to offer beside `owner`: never itself; only running
    /// ones unless `ended`; those whose name or host has `query` in it;
    /// the owner's project first (`here`), each group last active first.
    static func sessions(
        _ all: [UnifiedSession], owner: SessionRef, project: String, query: String, ended: Bool
    ) -> (here: [UnifiedSession], elsewhere: [UnifiedSession]) {
        let kept = all
            .filter { $0.ref != owner && (ended || $0.session.liveness == .running) }
            .filter { matches(query, $0.session.name) || matches(query, $0.serverName) }
            .sorted { a, b in
                let (at, bt) = (lastActive(a.session), lastActive(b.session))
                return at != bt ? at > bt : a.session.name < b.session.name
            }
        let here = kept.filter { $0.ref.serverID == owner.serverID && $0.session.project == project }
        return (here, kept.filter { !here.contains($0) })
    }

    /// How many sessions `ended` would add.
    static func endedCount(_ all: [UnifiedSession], owner: SessionRef) -> Int {
        all.filter { $0.ref != owner && $0.session.liveness != .running }.count
    }

    /// How many of the files the session wrote are offered before a
    /// filter is typed: the newest, so older ones fall off as it works.
    static let writtenShown = 12

    /// The files the session wrote, as documents to put in a pane: the
    /// newest [`writtenShown`] of them, or every one `query` matches.
    static func written(
        _ files: [EditedFile], owner: SessionRef, project: String, query: String
    ) -> [FileRef] {
        let asked = !query.trimmingCharacters(in: .whitespaces).isEmpty
        return files
            .filter { matches(query, $0.path) }
            .prefix(asked ? files.count : writtenShown)
            .map { FileRef(serverID: owner.serverID, projectID: $0.project ?? project, path: $0.path) }
    }

    /// The files opened on this device from the owner's host that
    /// `query` matches, less those `shown` already lists.
    static func recent(_ opened: [FileRef], owner: SessionRef, query: String, besides shown: [FileRef]) -> [FileRef] {
        opened.filter { $0.serverID == owner.serverID && matches(query, $0.path) && !shown.contains($0) }
    }

    /// The fewest characters a query has before the host is searched.
    static let searchFrom = 2

    /// An empty query matches everything; otherwise `text` holds it, in any case.
    static func matches(_ query: String, _ text: String) -> Bool {
        let wanted = query.trimmingCharacters(in: .whitespaces)
        return wanted.isEmpty || text.localizedCaseInsensitiveContains(wanted)
    }

    private static func lastActive(_ session: SessionView) -> UInt64 {
        max(session.lastActivityMs ?? 0, session.lastStopAtMs ?? 0)
    }
}

private struct SlotChooser: View {
    @Bindable var model: BrowserModel
    let owner: SessionRef
    let project: String
    /// The edited-files list is offered.
    let files: Bool
    let choose: (SidePane) -> Void
    @State private var written: [EditedFile] = []
    @State private var query = ""
    @State private var ended = false
    /// What the host found for the query, and why it found nothing.
    @State private var found: [FileRef] = []
    @State private var searchNote: String?
    @State private var searching = false
    /// The host's project names, for a file in another project.
    @State private var projects: [String: String] = [:]

    var body: some View {
        let sessions = SlotChoices.sessions(
            model.store.sessions, owner: owner, project: project, query: query, ended: ended)
        let wrote = SlotChoices.written(written, owner: owner, project: project, query: query)
        let recent = SlotChoices.recent(model.recentFiles, owner: owner, query: query, besides: wrote)
        let hits = found.filter { !wrote.contains($0) && !recent.contains($0) }
        let endedCount = SlotChoices.endedCount(model.store.sessions, owner: owner)
        VStack(alignment: .leading, spacing: 8) {
            Label("Beside this card", systemImage: "rectangle.split.3x1").font(.headline)
            Text("A file to keep in view, or a second session to work in tandem.")
                .font(.caption).foregroundStyle(.secondary)
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Find a file or session by name", text: $query)
                    .textFieldStyle(.plain)
                    .autocorrectionDisabled()
                if searching { ProgressView().controlSize(.mini) }
                if !query.isEmpty {
                    Button {
                        query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Clear the filter")
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
            if endedCount > 0 {
                Toggle("Show \(endedCount) ended", isOn: $ended)
                    .font(.caption)
                    .controlSize(.mini)
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    if files && SlotChoices.matches(query, "edited files") {
                        Button {
                            choose(.files)
                        } label: {
                            Label("Edited files, with their diffs", systemImage: "doc.on.doc")
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(.bordered)
                    }
                    documents("Written by this session", wrote)
                    documents("Opened before", recent)
                    documents("In this host's projects", hits)
                    if let searchNote {
                        Text(searchNote).font(.caption).foregroundStyle(.secondary).padding(.top, 4)
                    }
                    group("This project", sessions.here)
                    group(sessions.here.isEmpty ? "Sessions" : "Other projects", sessions.elsewhere)
                    if !query.isEmpty, !searching, wrote.isEmpty, recent.isEmpty, hits.isEmpty,
                        sessions.here.isEmpty, sessions.elsewhere.isEmpty
                    {
                        Text("Nothing matches.").font(.callout).foregroundStyle(.secondary).padding(.top, 8)
                    }
                }
            }
        }
        .task(id: owner) {
            written = (try? await model.readEditedFiles(owner))?.files ?? []
            let listed = await model.catalog(for: owner.serverID)?.projects ?? []
            projects = Dictionary(listed.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
        }
        // Searched a moment after the typing stops; a new letter cancels
        // the search before it.
        .task(id: query) {
            let asked = query.trimmingCharacters(in: .whitespaces)
            guard asked.count >= SlotChoices.searchFrom else {
                found = []
                searchNote = nil
                searching = false
                return
            }
            searching = true
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
            do {
                let body = try await model.findFiles(on: owner.serverID, asked)
                guard !Task.isCancelled else { return }
                found = body.hits.map { FileRef(serverID: owner.serverID, projectID: $0.project, path: $0.path) }
                searchNote = nil
            } catch RemoteError.refused(_, "not_found", _) {
                found = []
                searchNote = "Searching every project needs a newer Switchboard on this host."
            } catch {
                guard !Task.isCancelled else { return }
                found = []
                searchNote = "The host did not answer the search."
            }
            searching = false
        }
    }

    @ViewBuilder
    private func documents(_ title: String, _ refs: [FileRef]) -> some View {
        if !refs.isEmpty {
            Text(title).font(.subheadline.bold()).padding(.top, 8)
            ForEach(refs, id: \.self) { ref in
                let name = ref.path.split(separator: "/").last.map(String.init) ?? ref.path
                let folder = ref.path.count > name.count ? String(ref.path.dropLast(name.count + 1)) : ""
                // A file outside the card's own project says which it is in.
                let place = [ref.projectID == project ? nil : projects[ref.projectID] ?? "another project", folder]
                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
                Button {
                    choose(.document(ref))
                } label: {
                    VStack(alignment: .leading, spacing: 0) {
                        Label(name, systemImage: name.lowercased().hasSuffix(".md") ? "doc.richtext" : "doc.text")
                            .lineLimit(1).truncationMode(.middle)
                        if !place.isEmpty {
                            Text(place).font(.caption2).foregroundStyle(.secondary)
                                .lineLimit(1).truncationMode(.head)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.bordered)
            }
        }
    }

    @ViewBuilder
    private func group(_ title: String, _ items: [UnifiedSession]) -> some View {
        if !items.isEmpty {
            Text(title).font(.subheadline.bold()).padding(.top, 8)
            ForEach(items) { item in
                Button {
                    choose(.session(item.ref))
                } label: {
                    HStack {
                        Text(item.session.name).lineLimit(1)
                        Spacer()
                        Text("\(item.session.card) · \(item.serverName)")
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.bordered)
            }
        }
    }
}

/// A diff window's request: one file of a session against the base its
/// files list named.
public struct DiffWindowRequest: Codable, Hashable, Sendable {
    public let id: UUID
    public let session: SessionRef
    public let project: String
    public let base: DiffBase
    public let path: String

    public init(id: UUID = UUID(), session: SessionRef, project: String, base: DiffBase, path: String) {
        self.id = id
        self.session = session
        self.project = project
        self.base = base
        self.path = path
    }
}

/// One file's diff against the base, read when the window opens and on
/// Refresh: removed lines red, added green, the words that changed in a
/// replaced line marked brighter.
public struct DiffWindowView: View {
    @Bindable var model: BrowserModel
    let request: DiffWindowRequest
    @State private var diff: FileDiffBody?
    @State private var failure: String?
    @State private var loading = false

    public init(model: BrowserModel, request: DiffWindowRequest) {
        self.model = model
        self.request = request
    }

    private var name: String { request.path.split(separator: "/").last.map(String.init) ?? request.path }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(request.path).font(.callout).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.head)
                    Text(request.base.label).font(.caption).foregroundStyle(.tertiary)
                }
                Spacer()
                if let diff, let hunks = diff.body.hunks {
                    let (added, removed) = Self.counts(hunks)
                    HStack(spacing: 4) {
                        Text("+\(added)").foregroundStyle(.green)
                        Text("−\(removed)").foregroundStyle(.red)
                    }
                    .font(.caption.monospacedDigit())
                }
                Button {
                    Task { await load() }
                } label: {
                    Image(systemName: "arrow.clockwise").font(.caption)
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.circle)
                .controlSize(.mini)
                .disabled(loading)
                .accessibilityLabel("Refresh")
                .help("Read the diff again")
            }
            if let diff {
                content(diff)
            } else if let failure {
                Text(failure).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding()
        .navigationTitle("\(name) diff")
        .task(id: request) { await load() }
    }

    private func load() async {
        loading = true
        defer { loading = false }
        do {
            diff = try await model.readDiff(request.session, base: request.base.sha, path: request.path)
            failure = nil
        } catch RemoteError.refused(_, _, let message) {
            failure = message
        } catch {
            failure = "The host did not answer."
        }
    }

    static func counts(_ hunks: [DiffHunk]) -> (Int, Int) {
        var added = 0
        var removed = 0
        for line in hunks.flatMap(\.lines) {
            if line.kind == "added" { added += 1 } else if line.kind == "removed" { removed += 1 }
        }
        return (added, removed)
    }

    @ViewBuilder
    private func content(_ diff: FileDiffBody) -> some View {
        switch diff.body.kind {
        case "binary":
            note("A binary file: no lines to show.")
        case "too_large":
            note("Too large to show here (\(diff.body.lines ?? 0) lines of diff).")
        case "empty":
            note(diff.status == "renamed" ? "Renamed, with no line changed." : "No line changed.")
        default:
            // Up and down only, as the file window: a lazy stack that also
            // scrolls sideways wraps every line narrow.
            ScrollView(.vertical) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array((diff.body.hunks ?? []).enumerated()), id: \.offset) { i, hunk in
                        if i > 0 {
                            Text("⋯").font(.caption).foregroundStyle(.tertiary).padding(.vertical, 4)
                        }
                        ForEach(Array(hunk.lines.enumerated()), id: \.offset) { _, line in
                            DiffLineRow(line: line)
                        }
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }

    private func note(_ text: String) -> some View {
        Text(text).foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

/// One line of a diff: both line numbers, the sign, and the text with
/// its changed words marked.
private struct DiffLineRow: View {
    let line: DiffLine

    private var tint: Color {
        switch line.kind {
        case "added": return .green
        case "removed": return .red
        default: return .clear
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            number(line.old)
            number(line.new)
            Text(line.kind == "added" ? "+" : line.kind == "removed" ? "−" : " ")
                .foregroundStyle(tint == .clear ? Color.secondary : tint)
            Text(DiffMarks.attributed(line.text, marks: line.marks, tint: tint))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.system(size: 14, design: .monospaced))
        .padding(.vertical, 1)
        .padding(.trailing, 8)
        .background(tint.opacity(tint == .clear ? 0 : 0.14))
    }

    private func number(_ n: Int?) -> some View {
        Text(n.map(String.init) ?? "")
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(.tertiary)
            .frame(width: 40, alignment: .trailing)
    }
}

/// A diff line's text with its changed words marked. The marks are
/// UTF-8 byte ranges from the host; one that does not land on character
/// boundaries is left unmarked rather than cutting a character.
public enum DiffMarks {
    public static func attributed(_ text: String, marks: [[Int]], tint: Color) -> AttributedString {
        var out = AttributedString(text)
        guard tint != .clear else { return out }
        let utf8 = text.utf8
        for mark in marks where mark.count == 2 {
            guard mark[0] >= 0, mark[0] < mark[1], mark[1] <= utf8.count,
                let lower = utf8.index(utf8.startIndex, offsetBy: mark[0], limitedBy: utf8.endIndex)?
                    .samePosition(in: text),
                let upper = utf8.index(utf8.startIndex, offsetBy: mark[1], limitedBy: utf8.endIndex)?
                    .samePosition(in: text),
                let start = AttributedString.Index(lower, within: out),
                let end = AttributedString.Index(upper, within: out)
            else { continue }
            out[start..<end].backgroundColor = tint.opacity(0.35)
        }
        return out
    }
}

/// How the side panes sit beside a card window.
enum SidePaneLayout {
    /// The card window's default height, which the panes' sizes are drawn for.
    static let designHeight: CGFloat = 900

    /// The size a pane is drawn at beside a window of the default height:
    /// the files list and a chooser narrow, a document or a session wide.
    static func drawn(_ pane: SidePane?) -> CGSize {
        switch pane {
        case .files: return CGSize(width: 360, height: 640)
        case nil: return CGSize(width: 380, height: 620)
        case .document, .session: return CGSize(width: 540, height: 860)
        }
    }

    /// A pane's size for a window `height` tall: its height follows the
    /// window's and its width follows its height, so it keeps its shape.
    /// Held between a third and one and a half of its drawn size, then
    /// times the user's `factor` for that side.
    static func size(width: CGFloat, height: CGFloat, windowHeight: CGFloat, factor: Double = 1) -> CGSize {
        let follow = windowHeight > 0 ? min(max(windowHeight / designHeight, 0.35), 1.5) : 1
        let scale = follow * CGFloat(factor)
        return CGSize(width: (width * scale).rounded(), height: (height * scale).rounded())
    }
}

extension View {
    /// A pane kept beside the window on `edge` while `shown`, moving with
    /// it and turned in toward the viewer like a side panel, hinged on the
    /// edge next to the window. Only the headset has ornaments; elsewhere
    /// there is no pane.
    @ViewBuilder
    func sidePane(
        _ edge: HorizontalEdge, shown: Bool, tilt: Double, @ViewBuilder content: () -> some View
    ) -> some View {
        #if os(visionOS)
        let inner: UnitPoint3D = edge == .leading ? .trailing : .leading
        ornament(
            visibility: shown ? .visible : .hidden,
            attachmentAnchor: .scene(edge == .leading ? .leading : .trailing),
            contentAlignment: edge == .leading ? .trailing : .leading
        ) {
            content()
                .glassBackgroundEffect()
                // A true turn in depth: the `axis:` form only draws a
                // perspective on the flat plane, which barely shows and
                // reads either way. With z toward the viewer and y down, a
                // positive turn about y brings the left edge forward, so
                // the left pane turns positive and the right one negative.
                .rotation3DEffect(
                    Rotation3D(
                        angle: .degrees(edge == .leading ? tilt : -tilt),
                        axis: .y),
                    anchor: inner)
                .padding(edge == .leading ? .trailing : .leading, 12)
        }
        #else
        self
        #endif
    }
}

/// A card window's own settings: how its side panes sit. Sliders move
/// the panes live; `save` runs when one is let go.
struct CardSettings: View {
    @Binding var settings: PaneSettings
    let save: () -> Void
    let done: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Card settings").font(.headline)
                Spacer()
                Button("Done", action: done)
                    .controlSize(.small)
            }
            row("Pane angle", value: $settings.tilt, in: PaneSettings.tilts,
                shown: "\(Int(settings.tilt.rounded()))°")
            row("Left pane size", value: $settings.leading, in: PaneSettings.sizes,
                shown: "\(Int((settings.leading * 100).rounded()))%")
            row("Right pane size", value: $settings.trailing, in: PaneSettings.sizes,
                shown: "\(Int((settings.trailing * 100).rounded()))%")
            Button("Reset") {
                settings.resetLayout()
                save()
            }
            .controlSize(.small)
        }
        .padding()
        .frame(width: 340)
    }

    private func row(_ title: String, value: Binding<Double>, in range: ClosedRange<Double>, shown: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(title).font(.callout)
                Spacer()
                Text(shown).font(.callout.monospacedDigit()).foregroundStyle(.secondary)
            }
            Slider(value: value, in: range) { editing in
                if !editing { save() }
            }
        }
    }
}
