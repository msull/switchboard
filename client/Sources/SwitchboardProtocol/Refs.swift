import Foundation

/// A logical installation, as `/info` and every pairing name it. Not a
/// trusted identity: trust is the pinned key.
public struct ServerID: Hashable, Codable, Sendable, CustomStringConvertible {
    public let raw: String

    public init(_ raw: String) { self.raw = raw.lowercased() }

    public init(from decoder: Decoder) throws {
        self.init(try decoder.singleValueContainer().decode(String.self))
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(raw)
    }

    public var description: String { raw }

    /// The short form the host's Bonjour instance name carries.
    public var short: String {
        String(raw.replacingOccurrences(of: "-", with: "").prefix(6))
    }
}

/// Every reference a client holds is qualified by its owning server,
/// so two hosts with the same record id never collide.
/// One run of a session on one host.
public struct RunRef: Hashable, Codable, Sendable {
    public let session: SessionRef
    public let n: Int

    public init(session: SessionRef, n: Int) {
        self.session = session
        self.n = n
    }
}

public struct SessionRef: Hashable, Codable, Sendable {
    public let serverID: ServerID
    public let recordID: String

    public init(serverID: ServerID, recordID: String) {
        self.serverID = serverID
        self.recordID = recordID
    }
}

/// A Dispatch ticket on one host.
public struct TicketRef: Hashable, Codable, Sendable {
    public let serverID: ServerID
    public let ticketID: String

    public init(serverID: ServerID, ticketID: String) {
        self.serverID = serverID
        self.ticketID = ticketID
    }
}

public struct WorkingSetRef: Hashable, Codable, Sendable {
    public let serverID: ServerID
    public let setID: String

    public init(serverID: ServerID, setID: String) {
        self.serverID = serverID
        self.setID = setID
    }
}

public struct FileRef: Hashable, Codable, Sendable {
    public let serverID: ServerID
    public let projectID: String
    public let path: String

    public init(serverID: ServerID, projectID: String, path: String) {
        self.serverID = serverID
        self.projectID = projectID
        self.path = path
    }
}
