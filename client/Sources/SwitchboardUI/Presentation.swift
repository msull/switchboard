import Foundation
import SwitchboardClientCore
import SwitchboardProtocol

/// How a card window shows its session (§7.1). The modes a session can
/// take depend on its kind; the ones this client does not draw yet say
/// which milestone brings them.
public enum ViewMode: String, Codable, CaseIterable, Sendable, Identifiable {
    case compact
    case screen
    case lastResponse = "last_response"
    case conversation
    case runOutput = "run_output"

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .compact: return "Card"
        case .screen: return "Screen"
        case .lastResponse: return "Last response"
        case .conversation: return "Conversation"
        case .runOutput: return "Run output"
        }
    }

    /// The modes a session of `kind` can take, in the order the window
    /// offers them.
    public static func offered(for kind: SessionKind) -> [ViewMode] {
        switch kind {
        case .claude: return [.compact, .screen, .lastResponse, .conversation]
        case .codex, .shell: return [.compact, .screen]
        case .command, .service: return [.compact, .screen, .runOutput]
        }
    }

    /// Whether this client draws the mode; the rest arrive with
    /// Milestone C.
    public var drawn: Bool {
        switch self {
        case .compact, .screen: return true
        case .lastResponse, .conversation, .runOutput: return false
        }
    }

    /// The milestone that brings a mode this client does not draw.
    public static let arrivesWith = "Milestone C"
}

/// What a card window is opened on: a qualified session and an id of
/// its own, so two windows on one session are two windows.
/// The main window, one per app: the id and the value it is opened
/// with, so opening it again brings the open one forward instead of
/// making a second.
public enum MainWindow {
    public static let id = "main"
    public static let value = 0
}

/// Whether a card window should bring the main window back at launch.
/// visionOS restores the windows that were open when the app last
/// quit, so a run that ended with only card windows comes back
/// without the browser, and nothing else opens it. The first card
/// window to ask after the restoration grace claims the job, unless
/// the main window has appeared by then.
public struct MainWindowGuard: Equatable, Sendable {
    public var visible = false
    public var claimed = false

    public init() {}

    /// True once, for the window that should open the main one.
    public mutating func claim() -> Bool {
        if visible || claimed { return false }
        claimed = true
        return true
    }
}

public struct CardWindowRequest: Codable, Hashable, Sendable {
    public let id: UUID
    public let ref: SessionRef

    public init(id: UUID = UUID(), ref: SessionRef) {
        self.id = id
        self.ref = ref
    }
}

/// Where a card window's session stands, from the unified store.
public enum CardStanding: Equatable, Sendable {
    /// Its host is connected and lists it.
    case live
    /// Its host is not connected; what is shown is what was last seen.
    case offline
    /// Its host is connected and no longer lists it.
    case removed
    /// No host has listed it yet, or its host was forgotten.
    case unknown

    public static func of(_ ref: SessionRef, in store: UnifiedStore, removed: Set<SessionRef>)
        -> CardStanding
    {
        if let item = store.sessions.first(where: { $0.ref == ref }) {
            return item.stale ? .offline : .live
        }
        return removed.contains(ref) ? .removed : .unknown
    }

    /// Whether the window's controls work: only on a live session.
    public var controlsEnabled: Bool { self == .live }

    public var banner: String? {
        switch self {
        case .live: return nil
        case .offline: return "The host is not connected; this is what it last showed."
        case .removed: return "This session was removed on its host."
        case .unknown: return "Waiting for the host to list this session."
        }
    }
}
