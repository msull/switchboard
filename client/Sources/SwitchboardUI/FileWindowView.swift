import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// The files an agent names in its prose, as the host found them: the
/// links put into the text for the Markdown reader, and the URL a tap
/// on one carries back.
public enum FileLinks {
    /// The scheme of a file link; the path is the URL's path, the
    /// line its fragment (`L12`).
    public static let scheme = "switchboard-file"

    /// `text` with each link's range made a Markdown link to its file.
    /// Ranges are UTF-8 byte offsets from the host; one that does not
    /// land on character boundaries, or runs past the text, is left
    /// alone rather than cutting a character.
    public static func linked(_ text: String, _ links: [LinkView]) -> String {
        guard !links.isEmpty else { return text }
        var out = text
        let utf8 = text.utf8
        // From the end back, so the ranges still ahead stay where they were.
        for link in links.sorted(by: { $0.start > $1.start }) {
            guard link.start >= 0, link.start < link.end, link.end <= utf8.count,
                let lower = utf8.index(utf8.startIndex, offsetBy: link.start, limitedBy: utf8.endIndex),
                let upper = utf8.index(utf8.startIndex, offsetBy: link.end, limitedBy: utf8.endIndex),
                let start = lower.samePosition(in: text), let end = upper.samePosition(in: text),
                let url = url(path: link.path, line: link.line)
            else { continue }
            let shown = String(text[start..<end])
            out.replaceSubrange(start..<end, with: "[\(shown)](\(url))")
        }
        return out
    }

    /// The URL of a file link.
    public static func url(path: String, line: Int?) -> String? {
        var allowed = CharacterSet.urlPathAllowed
        // Parentheses and spaces would end or break the Markdown link.
        allowed.remove(charactersIn: "()")
        guard let encoded = path.addingPercentEncoding(withAllowedCharacters: allowed) else { return nil }
        return "\(scheme):///\(encoded)" + (line.map { "#L\($0)" } ?? "")
    }

    /// The file and line a tapped link names, or nil for any other URL.
    public static func parse(_ url: URL) -> (path: String, line: Int?)? {
        guard url.scheme == scheme else { return nil }
        let path = String(url.path.drop(while: { $0 == "/" }))
        guard !path.isEmpty else { return nil }
        let line = url.fragment.flatMap { $0.hasPrefix("L") ? Int($0.dropFirst()) : nil }
        return (path, line)
    }
}

/// One file of a project on a host, read through the port as a card
/// reads a conversation: watched while the window is open, so an edit
/// on the Mac shows here. Markdown is read as the agent's prose; any
/// other text is shown by line, numbered, and the line a link named is
/// scrolled to and marked.
public struct FileWindowView: View {
    @Bindable var model: BrowserModel
    let request: FileWindowRequest
    @State private var waited = false
    @State private var marked: Int?
    /// In a board tile rather than a window of its own.
    public let embedded: Bool

    public init(model: BrowserModel, request: FileWindowRequest, embedded: Bool = false) {
        self.model = model
        self.request = request
        self.embedded = embedded
    }

    private var ref: FileRef { request.ref }
    private var file: FileBody? { model.files[ref] }
    private var name: String { ref.path.split(separator: "/").last.map(String.init) ?? ref.path }
    private var host: String { model.store.hosts.first { $0.id == ref.serverID }?.server.name ?? "" }
    private var markdown: Bool { name.lowercased().hasSuffix(".md") }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text(ref.path).font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                Spacer()
                if let file {
                    Text(detail(file)).font(.caption).foregroundStyle(.secondary)
                }
                if !embedded {
                    PinToBoardMenu(model: model, item: .file(ref), name: name)
                }
            }
            if let file {
                content(file)
            } else if waited {
                Text("The host did not serve this file: it may be gone, not text, a dotenv file, or over the port's size cap.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding()
        .navigationTitle(host.isEmpty ? name : "\(name) · \(host)")
        .task(id: ref) {
            await model.watchFile(ref)
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            waited = true
        }
        .onAppear { if !embedded { model.fileWindowAppeared(request) } }
        .onDisappear {
            if !embedded { model.fileWindowClosed(request) }
            Task { await model.unwatchFile(ref) }
        }
    }

    private func detail(_ file: FileBody) -> String {
        var parts = ["\(file.size) bytes"]
        if let ms = file.modifiedMs {
            let date = Date(timeIntervalSince1970: Double(ms) / 1000)
            parts.append(date.formatted(date: .abbreviated, time: .shortened))
        }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private func content(_ file: FileBody) -> some View {
        if file.text.isEmpty {
            Text("Empty.").foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else if markdown {
            ScrollView {
                AgentText(text: file.text)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                    .textSelection(.enabled)
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView([.vertical, .horizontal]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        let lines = file.text.split(separator: "\n", omittingEmptySubsequences: false)
                        ForEach(Array(lines.enumerated()), id: \.offset) { i, line in
                            HStack(alignment: .top, spacing: 12) {
                                Text("\(i + 1)")
                                    .font(.system(size: 13, design: .monospaced))
                                    .foregroundStyle(.tertiary)
                                    .frame(width: 48, alignment: .trailing)
                                Text(String(line))
                                    .font(.system(size: 14, design: .monospaced))
                                    .textSelection(.enabled)
                            }
                            .padding(.vertical, 1)
                            .background(marked == i + 1 ? Color.accentColor.opacity(0.18) : Color.clear)
                            .id(i + 1)
                        }
                    }
                    .padding(.vertical, 4)
                }
                .onChange(of: model.lineRequests[ref], initial: true) { _, request in
                    guard let request else { return }
                    marked = request.line
                    withAnimation { proxy.scrollTo(request.line, anchor: .center) }
                }
            }
        }
    }
}
