import Foundation

/// What `/info` says to anyone.
public struct ServerInfo: Decodable, Sendable, Equatable {
    public let serverID: ServerID
    public let serverName: String
    public let protocolVersion: Int
    public let capabilities: [String]
    public let spkiSha256: String
    public let limits: Limits
    public let pairingOpen: Bool

    public struct Limits: Decodable, Sendable, Equatable {
        public let bodyBytes: Int
        public let stringBytes: Int
        public let notesBytes: Int
        public let screenLines: Int

        enum CodingKeys: String, CodingKey {
            case bodyBytes = "body_bytes"
            case stringBytes = "string_bytes"
            case notesBytes = "notes_bytes"
            case screenLines = "screen_lines"
        }
    }

    enum CodingKeys: String, CodingKey {
        case serverID = "server_id"
        case serverName = "server_name"
        case protocolVersion = "protocol"
        case capabilities
        case spkiSha256 = "spki_sha256"
        case limits
        case pairingOpen = "pairing_open"
    }
}

/// What a device may do with its token.
public enum Scope: String, Codable, Sendable {
    case read
    case control
}

/// `POST /pair`.
public struct PairRequest: Encodable, Sendable {
    public let code: String
    public let deviceName: String
    public let spkiSha256: String

    public init(code: String, deviceName: String, spkiSha256: String) {
        self.code = code
        self.deviceName = deviceName
        self.spkiSha256 = spkiSha256
    }

    enum CodingKeys: String, CodingKey {
        case code
        case deviceName = "device_name"
        case spkiSha256 = "spki_sha256"
    }
}

public struct PairReply: Decodable, Sendable {
    public let token: String
    public let scope: Scope
    public let serverID: ServerID
    public let deviceID: String

    enum CodingKeys: String, CodingKey {
        case token, scope
        case serverID = "server_id"
        case deviceID = "device_id"
    }
}

/// What the host's QR code holds while its pairing window is open: the
/// installation, the key to pin before the code is sent, the code, and
/// where the host answered from. Also pasted by hand.
public struct PairingInvite: Decodable, Sendable, Equatable {
    public let serverID: ServerID
    public let spkiSha256: String
    public let code: String
    public let addresses: [String]

    enum CodingKeys: String, CodingKey {
        case serverID = "server_id"
        case spkiSha256 = "spki_sha256"
        case code, addresses
    }

    public init(serverID: ServerID, spkiSha256: String, code: String, addresses: [String]) {
        self.serverID = serverID
        self.spkiSha256 = spkiSha256
        self.code = code
        self.addresses = addresses
    }

    /// The invite from the scanned or pasted text.
    public static func parse(_ text: String) throws -> PairingInvite {
        try Wire.decode(PairingInvite.self, from: Data(text.utf8))
    }
}
