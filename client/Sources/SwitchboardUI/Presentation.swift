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

    /// The icon that stands for the mode when the titles do not fit.
    public var symbol: String {
        switch self {
        case .compact: return "rectangle.on.rectangle"
        case .screen: return "terminal"
        case .lastResponse: return "text.bubble"
        case .conversation: return "bubble.left.and.bubble.right"
        case .runOutput: return "doc.plaintext"
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

    /// Whether the mode shows text the window scales: everything but
    /// the card, whose few lines are fine at the system size.
    public var scalable: Bool {
        switch self {
        case .screen, .runOutput, .lastResponse, .conversation: return true
        case .compact: return false
        }
    }
}

/// The text scale in a card window: a multiplier on the base type
/// sizes, set by a slider or snapped to a preset. The monospaced
/// modes and the prose ones share it, so switching modes keeps the
/// size the window was set to.
public enum ScreenScale {
    public static let initial = 1.0
    public static let range = 0.5...3.0
    /// The sizes to snap to.
    public static let presets = [0.5, 0.75, 1.0, 1.5, 2.0]
    /// The sizes at scale 1: three quarters of the system sizes, which
    /// is what reads best on the headset at the distance a card window
    /// usually sits, so the default is the size most windows want.
    /// The monospaced size.
    public static let basePoints = 11.25
    /// The prose sizes: an answer, a secondary line, a caption, a
    /// smaller caption.
    public static let prosePoints = 12.75
    public static let calloutPoints = 12.0
    public static let captionPoints = 9.0
    public static let caption2Points = 8.25

    public static func clamped(_ scale: Double) -> Double {
        min(max(scale, range.lowerBound), range.upperBound)
    }

    /// The scale as a percentage for the slider's readout.
    public static func label(_ scale: Double) -> String {
        "\(Int((scale * 100).rounded()))%"
    }
}

/// One activity in one turn, for the folds a window keeps open.
public struct ActivityKey: Codable, Hashable, Sendable {
    public let turn: Int
    public let index: Int

    public init(turn: Int, index: Int) {
        self.turn = turn
        self.index = index
    }
}

/// Where a window stands in a conversation (§7.1): the turn at the top
/// of the view or the end it follows, and the folds open, keyed by the
/// transcript so another one starts over. Kept across the page merges
/// and reconnects that replace the turns under the view.
public struct ConversationPlace: Codable, Equatable, Sendable {
    public var resumeID: String
    /// The turn at the top of the view while not following the end.
    public var turn: Int?
    /// At the end: the view moves with the host as turns arrive.
    public var following = true
    public var openTurns: Set<Int> = []
    public var openActivities: Set<ActivityKey> = []

    public init(resumeID: String, turn: Int? = nil, following: Bool = true) {
        self.resumeID = resumeID
        self.turn = turn
        self.following = following
    }

    /// How far short of the end the view may stand and still follow
    /// it, in points: a bounce or a finger lifted early leaves it a
    /// little above, which is not a reader going back.
    public static let followSlack = 32.0

    /// Whether a view whose visible span ends at `visibleBottom`, in
    /// content `contentHeight` tall, follows the end.
    public static func follows(visibleBottom: Double, contentHeight: Double) -> Bool {
        visibleBottom >= contentHeight - followSlack
    }

    /// The place against the conversation now held: another transcript
    /// starts over at the end with nothing open; a turn no longer held
    /// (the page was replaced after a reconnect) follows the end too.
    public func fitted(to conversation: ConversationState) -> ConversationPlace {
        if conversation.resumeID != resumeID {
            return ConversationPlace(resumeID: conversation.resumeID)
        }
        var place = self
        if let turn, !conversation.turns.contains(where: { $0.n == turn }) {
            place.turn = nil
            place.following = true
        }
        return place
    }

    /// The turn to put at the top on a restore: none when following
    /// the end, which the view scrolls to instead.
    public func target(in conversation: ConversationState) -> Int? {
        guard !following, let turn, conversation.turns.contains(where: { $0.n == turn }) else { return nil }
        return turn
    }
}

/// The main window, one per app: the id and the value it is opened
/// with, so opening it again brings the open one forward instead of
/// making a second.
public enum MainWindow {
    public static let id = "main"
    public static let value = 0
}

/// The prompt window, one per app: the whole draft of the session whose
/// input is active, with the editor's tools, wherever it is placed.
public enum PromptWindow {
    public static let id = "prompt"
    public static let value = Key()

    /// The window's presented value, a type of its own: the system
    /// matches windows by presented type and value, so a prompt window
    /// presenting the main window's `Int` was taken for it.
    public struct Key: Codable, Hashable, Sendable {
        public init() {}
    }
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

/// What a card window is opened on: a qualified session and an id of
/// its own. Opening a request whose window is open brings that window
/// forward, so a click on a listed session goes to its window when
/// there is one, and a request with a fresh id is a second window.
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
