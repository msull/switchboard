import Foundation
import SwitchboardProtocol

/// One run of one session on a host, by the record id the host uses.
public struct RunKey: Hashable, Sendable {
    public let session: String
    public let n: Int

    public init(session: String, n: Int) {
        self.session = session
        self.n = n
    }
}

/// A conversation as the client holds it: the metadata of the last
/// page and the turns from every page merged by turn number, so a
/// window can page back to the start and follow the end (§4.3).
public struct ConversationState: Sendable, Equatable {
    public var resumeID: String
    public var title: String?
    public var model: String?
    public var branch: String?
    public var total: Int
    public var usage: UsageView
    public var contextTokens: UInt64?
    public var turns: [TurnView]

    public init(page: ConversationBody) {
        resumeID = page.resumeID
        title = page.title
        model = page.model
        branch = page.branch
        total = page.total
        usage = page.usage
        contextTokens = page.contextTokens
        turns = page.turns
    }

    /// A turn read whole takes the place of the page's copy, when it
    /// is from the same transcript and held.
    public mutating func fill(_ whole: TurnBody) {
        guard whole.resumeID == resumeID, let turn = whole.turn,
            let i = turns.firstIndex(where: { $0.n == turn.n })
        else { return }
        turns[i] = turn
    }

    /// The first turn held, so a window knows whether earlier ones
    /// exist to load.
    public var firstHeld: Int? { turns.first?.n }
    public var hasEarlier: Bool { (firstHeld ?? 1) > 1 }
    public var lastHeld: Int? { turns.last?.n }

    /// Fold a page in. Another `resume_id` (`/clear`, a discard) drops
    /// everything held; otherwise the page's range replaces what was
    /// held there, and a page that reaches the end replaces everything
    /// after it too, since the last turn is in progress until a stop.
    public mutating func merge(_ page: ConversationBody) {
        if page.resumeID != resumeID {
            self = ConversationState(page: page)
            return
        }
        title = page.title
        model = page.model
        branch = page.branch
        total = page.total
        usage = page.usage
        contextTokens = page.contextTokens
        let last = page.turns.last?.n ?? (page.from - 1)
        let kept = turns.filter { $0.n < page.from || (page.next != nil && $0.n > last) }
        turns = (kept + page.turns).sorted { $0.n < $1.n }
    }
}
