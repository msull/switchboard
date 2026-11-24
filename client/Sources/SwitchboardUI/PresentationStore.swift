import Foundation
import SwitchboardClientCore
import SwitchboardProtocol

/// The user's presentation state (§7): what the client itself decides
/// and keeps in its container, never anything a host sent. A card
/// window's mode is keyed by the window id it was opened with.
public struct PresentationState: Codable, Equatable, Sendable {
    public var modes: [UUID: ModeEntry] = [:]
    /// How the main window browses, and what it shows.
    public var browse: BrowseMode = .list
    public var filters = Filters()
    /// The ticket list's filters and order.
    public var ticketFilters = TicketFilters()
    /// The session focus mode is on, kept by ref so a reorder keeps it.
    public var focused: SessionRef?
    /// The pin boards, the client's own, in the order they were made.
    public var boards: [Board] = []
    /// Each session card's side panes, kept by session rather than by
    /// window: a card closed and opened again is a new window.
    public var panes: [SessionRef: PanesEntry] = [:]
    /// The files opened on this device, last opened first, so one found
    /// once can be put beside a card again without finding it again.
    public var recentFiles: [FileRef] = []

    /// How many opened files are remembered.
    public static let recentFilesCap = 40

    public struct PanesEntry: Codable, Equatable, Sendable {
        public var settings: PaneSettings
        public var at: Date
    }

    public struct ModeEntry: Codable, Equatable, Sendable {
        public var mode: ViewMode
        public var at: Date
        /// Where the window stands in its conversation, once it has one.
        public var place: ConversationPlace?

        public init(mode: ViewMode, at: Date = Date(), place: ConversationPlace? = nil) {
            self.mode = mode
            self.at = at
            self.place = place
        }
    }

    /// How many windows' modes are kept; a window closed by the user
    /// leaves its entry, so the oldest go first past this.
    public static let modeCap = 256

    public init() {}

    /// A state saved by an older client lacks what was added since.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        modes = try c.decodeIfPresent([UUID: ModeEntry].self, forKey: .modes) ?? [:]
        browse = try c.decodeIfPresent(BrowseMode.self, forKey: .browse) ?? .list
        filters = try c.decodeIfPresent(Filters.self, forKey: .filters) ?? Filters()
        ticketFilters = try c.decodeIfPresent(TicketFilters.self, forKey: .ticketFilters) ?? TicketFilters()
        focused = try c.decodeIfPresent(SessionRef.self, forKey: .focused)
        boards = try c.decodeIfPresent([Board].self, forKey: .boards) ?? []
        panes = try c.decodeIfPresent([SessionRef: PanesEntry].self, forKey: .panes) ?? [:]
        recentFiles = try c.decodeIfPresent([FileRef].self, forKey: .recentFiles) ?? []
    }

    public mutating func setMode(_ mode: ViewMode, for window: UUID, at: Date = Date()) {
        modes[window] = ModeEntry(mode: mode, at: at, place: modes[window]?.place)
        trimModes()
    }

    public func mode(for window: UUID) -> ViewMode? {
        modes[window]?.mode
    }

    public mutating func setPlace(_ place: ConversationPlace, for window: UUID, at: Date = Date()) {
        modes[window] = ModeEntry(mode: modes[window]?.mode ?? .compact, at: at, place: place)
        trimModes()
    }

    public func place(for window: UUID) -> ConversationPlace? {
        modes[window]?.place
    }

    public mutating func setPanes(_ settings: PaneSettings, for session: SessionRef, at: Date = Date()) {
        panes[session] = PanesEntry(settings: settings, at: at)
        while panes.count > Self.modeCap, let oldest = panes.min(by: { $0.value.at < $1.value.at }) {
            panes[oldest.key] = nil
        }
    }

    /// `file` was opened: it goes to the head of the recent files, once.
    public mutating func noteOpened(_ file: FileRef) {
        recentFiles.removeAll { $0 == file }
        recentFiles.insert(file, at: 0)
        if recentFiles.count > Self.recentFilesCap {
            recentFiles.removeLast(recentFiles.count - Self.recentFilesCap)
        }
    }

    public func panes(for session: SessionRef) -> PaneSettings? {
        panes[session]?.settings
    }

    private mutating func trimModes() {
        while modes.count > Self.modeCap, let oldest = modes.min(by: { $0.value.at < $1.value.at }) {
            modes[oldest.key] = nil
        }
    }

}

/// One session card's side panes: which are open, what the right one
/// holds, and how they sit. Set per card, since the angle that reads
/// well depends on where the window stands in the room.
public struct PaneSettings: Codable, Equatable, Sendable {
    /// Degrees each pane turns in toward the viewer.
    public var tilt: Double = 20
    /// Each side's size against what the window's height gives it.
    public var leading: Double = 1
    public var trailing: Double = 1
    /// The left pane is open. (Named for what it first held.)
    public var filesOpen = false
    public var slotOpen = false
    /// What the left pane holds, the edited files to begin with; nil is
    /// its chooser.
    public var leadingPane: SidePane? = .files
    /// What the right pane holds; nil is its chooser.
    public var slot: SidePane?
    /// The files pane leaves off files that match the base.
    public var hideUnchanged = false

    public static let standard = PaneSettings()
    public static let tilts: ClosedRange<Double> = 0...45
    public static let sizes: ClosedRange<Double> = 0.5...1.5

    public init() {}

    /// A card saved by an older client lacks what was added since.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let standard = Self.standard
        tilt = try c.decodeIfPresent(Double.self, forKey: .tilt) ?? standard.tilt
        leading = try c.decodeIfPresent(Double.self, forKey: .leading) ?? standard.leading
        trailing = try c.decodeIfPresent(Double.self, forKey: .trailing) ?? standard.trailing
        filesOpen = try c.decodeIfPresent(Bool.self, forKey: .filesOpen) ?? false
        slotOpen = try c.decodeIfPresent(Bool.self, forKey: .slotOpen) ?? false
        slot = try c.decodeIfPresent(SidePane.self, forKey: .slot)
        hideUnchanged = try c.decodeIfPresent(Bool.self, forKey: .hideUnchanged) ?? false
        // Absent is a card saved before the left pane could hold anything
        // but the files; null is a left pane the user emptied.
        leadingPane =
            c.contains(.leadingPane) ? try c.decodeIfPresent(SidePane.self, forKey: .leadingPane) : .files
    }

    /// Written by hand so an emptied left pane is saved as null: the
    /// synthesized form leaves a nil out, which would read back as the
    /// files.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(tilt, forKey: .tilt)
        try c.encode(leading, forKey: .leading)
        try c.encode(trailing, forKey: .trailing)
        try c.encode(filesOpen, forKey: .filesOpen)
        try c.encode(slotOpen, forKey: .slotOpen)
        try c.encode(leadingPane, forKey: .leadingPane)
        try c.encodeIfPresent(slot, forKey: .slot)
        try c.encode(hideUnchanged, forKey: .hideUnchanged)
    }

    enum CodingKeys: String, CodingKey {
        case tilt, leading, trailing, filesOpen, slotOpen, leadingPane, slot, hideUnchanged
    }

    /// The angle and sizes as they first were, the panes left as they are.
    public mutating func resetLayout() {
        tilt = Self.standard.tilt
        leading = Self.standard.leading
        trailing = Self.standard.trailing
    }
}

/// How the main window lays the sessions out (§7.2).
public enum BrowseMode: String, Codable, CaseIterable, Sendable, Identifiable {
    case list
    case grid
    case focus

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .list: return "List"
        case .grid: return "Grid"
        case .focus: return "Focus"
        }
    }

    public var symbol: String {
        switch self {
        case .list: return "list.bullet"
        case .grid: return "square.grid.2x2"
        case .focus: return "rectangle.inset.filled"
        }
    }
}

/// Focus mode's cursor over the sessions in view: one at a time, kept
/// by ref so a session moving in the order stays focused, and clamped
/// to what is listed when it is gone.
public enum FocusCursor {
    public static func index(of focused: SessionRef?, in items: [UnifiedSession]) -> Int? {
        guard !items.isEmpty else { return nil }
        return items.firstIndex { $0.ref == focused } ?? 0
    }

    public static func moved(_ focused: SessionRef?, by step: Int, in items: [UnifiedSession]) -> SessionRef? {
        guard let i = index(of: focused, in: items) else { return nil }
        let next = (i + step + items.count) % items.count
        return items[next].ref
    }
}

/// Where the presentation state lives.
public protocol PresentationStore: Sendable {
    func load() throws -> PresentationState
    func save(_ state: PresentationState) throws
}

/// A JSON file in the app's container.
public struct FilePresentationStore: PresentationStore {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    public func load() throws -> PresentationState {
        guard FileManager.default.fileExists(atPath: url.path) else { return PresentationState() }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(PresentationState.self, from: Data(contentsOf: url))
    }

    public func save(_ state: PresentationState) throws {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys]
        try encoder.encode(state).write(to: url, options: .atomic)
    }
}

/// Nothing kept, for tests and the command line.
public final class MemoryPresentationStore: PresentationStore, @unchecked Sendable {
    private let lock = NSLock()
    private var state = PresentationState()

    public init() {}

    public func load() throws -> PresentationState {
        lock.lock()
        defer { lock.unlock() }
        return state
    }

    public func save(_ state: PresentationState) throws {
        lock.lock()
        self.state = state
        lock.unlock()
    }
}
