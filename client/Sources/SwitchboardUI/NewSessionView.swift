import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

/// Where a new session goes when the sheet opens: a host and a
/// project chosen already (from a listed session's menu), or neither.
public struct NewSessionSeed: Identifiable, Hashable, Sendable {
    public let id = UUID()
    public var host: ServerID?
    public var project: String?

    public init(host: ServerID? = nil, project: String? = nil) {
        self.host = host
        self.project = project
    }
}

/// The name a new session gets when the owner types none: the kind's
/// word and one more than the project already has of that kind.
public enum NewSessionNaming {
    public static func defaultName(kind: SessionKind, existing: [SessionView]) -> String {
        let word: String
        switch kind {
        case .claude: word = "Claude"
        case .codex: word = "Codex"
        case .shell: word = "Shell"
        case .command: word = "Command"
        case .service: word = "Service"
        }
        return "\(word) \(existing.filter { $0.kind == kind }.count + 1)"
    }
}

/// The sheet that opens a session: a host under the control scope
/// that serves it, one of its projects by workspace, the kind, a
/// name, a directory under the project's root, and for an agent its
/// first message. Start posts it and opens the session's window.
struct NewSessionSheet: View {
    @Bindable var model: BrowserModel
    let seed: NewSessionSeed
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openWindow) private var openWindow
    @State private var host: ServerID?
    @State private var catalog: HostCatalog?
    @State private var project: String?
    @State private var kind: SessionKind = .claude
    @State private var name = ""
    @State private var cwd = ""
    @State private var prompt = ""
    @State private var busy = false
    @State private var note: String?

    private var hosts: [HostStatus] {
        model.store.hosts.filter { model.createHosts.contains($0.id) }
    }

    private var chosen: ProjectView? {
        catalog?.projects.first { $0.id == project }
    }

    /// The project's sessions as listed, which the default name counts.
    private var existing: [SessionView] {
        model.store.sessions
            .filter { $0.ref.serverID == host && $0.session.project == project }
            .map(\.session)
    }

    private var placeholder: String {
        NewSessionNaming.defaultName(kind: kind, existing: existing)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Where") {
                    Picker("Host", selection: $host) {
                        ForEach(hosts) { status in
                            Text(status.server.name).tag(Optional(status.id))
                        }
                    }
                    Picker("Project", selection: $project) {
                        ForEach(catalog?.byWorkspace ?? [], id: \.name) { group in
                            Section(group.name) {
                                ForEach(group.projects) { item in
                                    Text(item.name).tag(Optional(item.id))
                                }
                            }
                        }
                    }
                    if let chosen {
                        Text(chosen.root).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Section("What") {
                    Picker("Kind", selection: $kind) {
                        Text("Claude Code").tag(SessionKind.claude)
                        Text("Codex").tag(SessionKind.codex)
                        Text("Shell").tag(SessionKind.shell)
                    }
                    .pickerStyle(.segmented)
                    TextField("Name", text: $name, prompt: Text(placeholder))
                    TextField("Directory", text: $cwd, prompt: Text("the project's root"))
                        .autocorrectionDisabled()
                }
                if kind != .shell {
                    Section("First message") {
                        TextEditor(text: $prompt).frame(minHeight: 140)
                    }
                }
                if let note {
                    Section { Text(note).foregroundStyle(.red) }
                }
            }
            .navigationTitle("New session")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(busy ? "Starting…" : "Start") { Task { await start() } }
                        .disabled(busy || host == nil || project == nil)
                }
            }
        }
        .task(id: host) { await load() }
        .onAppear {
            if host == nil { host = seed.host ?? hosts.first?.id }
        }
    }

    /// The host's projects, and a project chosen among them: the seed's
    /// on the seed's host, else the first listed.
    private func load() async {
        guard let host else {
            catalog = nil
            return
        }
        catalog = await model.catalog(for: host)
        let listed = catalog?.projects.map(\.id) ?? []
        if project.map({ listed.contains($0) }) != true {
            let seeded = seed.host == host ? seed.project : nil
            let kept = seeded.flatMap { listed.contains($0) ? $0 : nil }
            project = kept ?? catalog?.byWorkspace.first?.projects.first?.id
        }
    }

    private func start() async {
        guard let host, let project else { return }
        busy = true
        note = nil
        let typed = name.trimmingCharacters(in: .whitespaces)
        let dir = cwd.trimmingCharacters(in: .whitespaces)
        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let outcome = await model.newSession(
            on: host, project: project, name: typed.isEmpty ? placeholder : typed, kind: kind,
            cwd: dir.isEmpty ? nil : dir, prompt: kind == .shell || text.isEmpty ? nil : text)
        busy = false
        switch outcome {
        case .opened(let id):
            dismiss()
            if let id {
                openWindow(value: model.windowRequest(for: SessionRef(serverID: host, recordID: id)))
            }
        default:
            note = outcome.note
        }
    }
}
