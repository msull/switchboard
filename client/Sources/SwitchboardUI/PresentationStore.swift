import Foundation

/// The user's presentation state (§7): what the client itself decides
/// and keeps in its container, never anything a host sent. visionOS
/// restores an app's windows on relaunch, so a card window's mode is
/// keyed by the window id it was opened with.
public struct PresentationState: Codable, Equatable, Sendable {
    public var modes: [UUID: ModeEntry] = [:]

    public struct ModeEntry: Codable, Equatable, Sendable {
        public var mode: ViewMode
        public var at: Date

        public init(mode: ViewMode, at: Date = Date()) {
            self.mode = mode
            self.at = at
        }
    }

    /// How many windows' modes are kept; a window closed by the user
    /// leaves its entry, so the oldest go first past this.
    public static let modeCap = 256

    public init() {}

    public mutating func setMode(_ mode: ViewMode, for window: UUID, at: Date = Date()) {
        modes[window] = ModeEntry(mode: mode, at: at)
        while modes.count > Self.modeCap, let oldest = modes.min(by: { $0.value.at < $1.value.at }) {
            modes[oldest.key] = nil
        }
    }

    public func mode(for window: UUID) -> ViewMode? {
        modes[window]?.mode
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
