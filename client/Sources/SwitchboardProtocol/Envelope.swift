import Foundation

/// The protocol this client speaks; `/info` reports the host's.
public enum RemoteProtocol {
    public static let version = 1
    /// The Bonjour service type the host advertises.
    public static let service = "_switchboard._tcp"
}

/// Every reply carries the host's run (`epoch`) and the revision at
/// which the body was observed; the client keeps one held revision per
/// resource and applies a reply only at or past it.
public struct Envelope<Body: Decodable & Sendable>: Decodable, Sendable {
    public let epoch: String
    public let revision: UInt64
    public let body: Body

    public init(epoch: String, revision: UInt64, body: Body) {
        self.epoch = epoch
        self.revision = revision
        self.body = body
    }
}

/// `{ "error": { "code": "...", "message": "..." } }`.
public struct ErrorBody: Decodable, Sendable, Equatable {
    public let code: String
    public let message: String
}

public struct ErrorEnvelope: Decodable, Sendable {
    public let epoch: String?
    public let revision: UInt64?
    public let error: ErrorBody
}

/// The decoder every reply goes through. Keys are the wire's own
/// snake_case names, spelled out in each type's `CodingKeys`, so a
/// renamed field fails loudly rather than decoding as `nil`.
public enum Wire {
    public static func decoder() -> JSONDecoder { JSONDecoder() }
    public static func encoder() -> JSONEncoder { JSONEncoder() }

    public static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        try decoder().decode(type, from: data)
    }
}
