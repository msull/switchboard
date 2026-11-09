import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol
import SwitchboardTransport

/// What a card window's right-hand pane holds: one document, or a
/// second session worked beside the first.
public enum SidePane: Hashable, Sendable {
    case document(FileRef)
    case session(SessionRef)
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
    @State private var body_: EditedFilesBody?
    @State private var failure: String?
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Files", systemImage: "doc.on.doc").font(.headline)
                Spacer()
                if let files = body_?.files, !files.isEmpty {
                    Text(totals(files)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
            if let base = body_?.base {
                Text(base.label).font(.caption).foregroundStyle(.secondary)
            }
            if let body_ {
                if body_.files.isEmpty {
                    Text("No files written yet.").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 2) {
                            ForEach(body_.files) { file in
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
                let fileRef = FileRef(serverID: ref.serverID, projectID: project, path: file.path)
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
        if !file.inRepo {
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
    @Binding var content: SidePane?
    /// The embedded card's or file's own id, so its mode and watch
    /// belong to this pane and not to another window on the same thing.
    @State private var inner = UUID()

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let content {
                HStack {
                    Text(title(content)).font(.headline).lineLimit(1)
                    Spacer()
                    Button {
                        self.content = nil
                    } label: {
                        Image(systemName: "xmark").font(.caption)
                    }
                    .buttonStyle(.bordered)
                    .buttonBorderShape(.circle)
                    .controlSize(.mini)
                    .accessibilityLabel("Empty the pane")
                    .help("Empty this pane")
                }
                switch content {
                case .document(let ref):
                    FileWindowView(model: model, request: FileWindowRequest(id: inner, ref: ref), embedded: true)
                case .session(let ref):
                    CardWindowView(model: model, request: CardWindowRequest(id: inner, ref: ref), embedded: true)
                }
            } else {
                SlotChooser(model: model, owner: owner, project: project) { chosen in
                    inner = UUID()
                    content = chosen
                }
            }
        }
        .padding(content == nil ? 16 : 8)
    }

    private func title(_ pane: SidePane) -> String {
        switch pane {
        case .document(let ref):
            return ref.path.split(separator: "/").last.map(String.init) ?? ref.path
        case .session(let ref):
            return model.store.sessions.first { $0.ref == ref }?.session.name ?? "Session"
        }
    }
}

/// What an empty right-hand pane offers: the Markdown files the session
/// wrote, then every other session, grouped by host.
private struct SlotChooser: View {
    @Bindable var model: BrowserModel
    let owner: SessionRef
    let project: String
    let choose: (SidePane) -> Void
    @State private var documents: [EditedFile] = []

    private var sessions: [UnifiedSession] {
        model.store.sessions
            .filter { $0.ref != owner }
            .sorted { ($0.serverName, $0.session.name) < ($1.serverName, $1.session.name) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Beside this card", systemImage: "sidebar.right").font(.headline)
            Text("A document to keep in view, or a second session to work in tandem.")
                .font(.caption).foregroundStyle(.secondary)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    if !documents.isEmpty {
                        Text("Documents this session wrote").font(.subheadline.bold()).padding(.top, 4)
                        ForEach(documents) { file in
                            Button {
                                choose(.document(FileRef(serverID: owner.serverID, projectID: project, path: file.path)))
                            } label: {
                                Label(file.path, systemImage: "doc.richtext")
                                    .lineLimit(1).truncationMode(.head)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .buttonStyle(.bordered)
                        }
                    }
                    Text("Sessions").font(.subheadline.bold()).padding(.top, 8)
                    ForEach(sessions) { item in
                        Button {
                            choose(.session(item.ref))
                        } label: {
                            HStack {
                                Text(item.session.name).lineLimit(1)
                                Spacer()
                                Text(item.serverName).font(.caption).foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(.bordered)
                    }
                }
            }
        }
        .task(id: owner) {
            let files = (try? await model.readEditedFiles(owner))?.files ?? []
            documents = files.filter { $0.path.lowercased().hasSuffix(".md") }
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
            ScrollView([.vertical, .horizontal]) {
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

extension View {
    /// A pane kept beside the window on `edge` while `shown`, moving with
    /// it. Only the headset has ornaments; elsewhere there is no pane.
    @ViewBuilder
    func sidePane(_ edge: HorizontalEdge, shown: Bool, @ViewBuilder content: () -> some View) -> some View {
        #if os(visionOS)
        ornament(
            visibility: shown ? .visible : .hidden,
            attachmentAnchor: .scene(edge == .leading ? .leading : .trailing),
            contentAlignment: edge == .leading ? .trailing : .leading
        ) {
            content()
                .glassBackgroundEffect()
                .padding(edge == .leading ? .trailing : .leading, 12)
        }
        #else
        self
        #endif
    }
}
