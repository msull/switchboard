import Foundation

/// The commands a device with the `control` scope may send (§4.5).
public enum CommandVerb: String, Sendable {
    case send, resume, trust
}

/// One record a command made; a session command makes none.
public struct MadeView: Decodable, Sendable, Equatable {
    public let kind: String
    public let id: String

    public init(kind: String, id: String) {
        self.kind = kind
        self.id = id
    }
}

/// A command accepted: the host ran it, or answered a repeat of its
/// op from the log.
public struct CommandBody: Decodable, Sendable, Equatable {
    public let made: [MadeView]

    public init(made: [MadeView] = []) {
        self.made = made
    }
}

/// `op.status`: what became of an operation the host may or may not
/// have received (§4.5).
public enum OpStatusView: Sendable, Equatable {
    /// Never seen: resubmitting with the same op is safe.
    case unknown
    /// Received and started; the outcome is not known. Never resubmit.
    case interrupted
    /// Still running on the host.
    case inProgress
    /// Finished: accepted, or failed with the host's reason.
    case done(failed: String?)
}

public struct OpStatusBody: Decodable, Sendable, Equatable {
    public let status: OpStatusView

    public init(status: OpStatusView) {
        self.status = status
    }

    enum CodingKeys: String, CodingKey { case status }
    enum StatusKeys: String, CodingKey { case status, reply }
    enum ReplyKeys: String, CodingKey { case reply, reason }

    public init(from decoder: Decoder) throws {
        let outer = try decoder.container(keyedBy: CodingKeys.self)
        let c = try outer.nestedContainer(keyedBy: StatusKeys.self, forKey: .status)
        switch try c.decode(String.self, forKey: .status) {
        case "done":
            let reply = try c.nestedContainer(keyedBy: ReplyKeys.self, forKey: .reply)
            if try reply.decode(String.self, forKey: .reply) == "failed" {
                status = .done(failed: try reply.decodeIfPresent(String.self, forKey: .reason) ?? "failed")
            } else {
                status = .done(failed: nil)
            }
        case "interrupted": status = .interrupted
        case "in-progress", "in_progress": status = .inProgress
        default: status = .unknown
        }
    }
}
