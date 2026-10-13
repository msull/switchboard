import Foundation

/// One frame on the events socket (§4.4). `hello` opens every
/// subscription; `changed` and `removed` name a resource at the
/// revision the host assigned the pass that moved it; `resync_required`
/// is the last frame before a host closes a subscriber that fell behind.
public struct EventFrame: Decodable, Sendable, Equatable {
    public enum Kind: Equatable, Sendable {
        case hello
        case changed
        case removed
        case watching
        case resyncRequired
        /// A newer host's frame this client does not know.
        case unknown(String)

        init(word: String) {
            switch word {
            case "hello": self = .hello
            case "changed": self = .changed
            case "removed": self = .removed
            case "watching": self = .watching
            case "resync_required": self = .resyncRequired
            default: self = .unknown(word)
            }
        }
    }

    public let kind: Kind
    public let serverID: ServerID?
    public let epoch: String?
    public let revision: UInt64?
    public let resource: ResourceRef?
    public let atMs: UInt64?

    enum CodingKeys: String, CodingKey {
        case kind = "type"
        case serverID = "server_id"
        case epoch, revision, resource
        case atMs = "at_ms"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = Kind(word: try c.decode(String.self, forKey: .kind))
        serverID = try c.decodeIfPresent(ServerID.self, forKey: .serverID)
        epoch = try c.decodeIfPresent(String.self, forKey: .epoch)
        revision = try c.decodeIfPresent(UInt64.self, forKey: .revision)
        resource = try c.decodeIfPresent(ResourceRef.self, forKey: .resource)
        atMs = try c.decodeIfPresent(UInt64.self, forKey: .atMs)
    }

    public init(
        kind: Kind, serverID: ServerID? = nil, epoch: String? = nil, revision: UInt64? = nil,
        resource: ResourceRef? = nil, atMs: UInt64? = nil
    ) {
        self.kind = kind
        self.serverID = serverID
        self.epoch = epoch
        self.revision = revision
        self.resource = resource
        self.atMs = atMs
    }
}

/// What a `changed` or `removed` frame names; a global collection's id
/// is empty.
public struct ResourceRef: Decodable, Sendable, Hashable {
    public let kind: String
    public let id: String

    public init(kind: String, id: String) {
        self.kind = kind
        self.id = id
    }
}

/// The resources a client holds a revision for, one key each (§4.2):
/// a reply or an event is applied only at or past the held revision.
public enum ResourceKey: Hashable, Sendable {
    case project(String)
    case space(String)
    case set(String)
    case session(String)
    case projects
    case spaces
    case sets(space: String)
    case sessions(project: String)
    case waiting

    /// The key a frame names, or `nil` for a kind this client does not
    /// hold (a content kind before its endpoint exists here).
    public init?(_ ref: ResourceRef) {
        switch ref.kind {
        case "project": self = .project(ref.id)
        case "space": self = .space(ref.id)
        case "set": self = .set(ref.id)
        case "session": self = .session(ref.id)
        case "projects": self = .projects
        case "spaces": self = .spaces
        case "sets": self = .sets(space: ref.id)
        case "sessions": self = .sessions(project: ref.id)
        case "waiting": self = .waiting
        default: return nil
        }
    }
}
