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
    /// The session focus mode is on, kept by ref so a reorder keeps it.
    public var focused: SessionRef?

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

    private mutating func trimModes() {
        while modes.count > Self.modeCap, let oldest = modes.min(by: { $0.value.at < $1.value.at }) {
            modes[oldest.key] = nil
        }
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
