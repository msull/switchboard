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
    /// The host app's version; a host older than this field has none.
    public let version: String?

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
        case version
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

/// A host's key as both ends show it to the owner for a comparison by
/// eye: the first sixteen hex digits of the SPKI hash in groups of
/// four, as the host's pairing dialog prints them.
public enum KeyFingerprint {
    public static func short(_ hex: String) -> String {
        let digits = Array(hex.prefix(16))
        return stride(from: 0, to: digits.count, by: 4)
            .map { String(digits[$0..<min($0 + 4, digits.count)]) }
            .joined(separator: " ")
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

    /// The invite from the scanned or pasted text: the host's page URL,
    /// `https://<address>/#code=…&key=…&id=…`, or the JSON form an
    /// older host shows.
    public static func parse(_ text: String) throws -> PairingInvite {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.hasPrefix("https://") else {
            return try Wire.decode(PairingInvite.self, from: Data(text.utf8))
        }
        // The authority is read as text so an IPv6 address keeps the
        // brackets `Address` expects.
        let afterScheme = text.dropFirst("https://".count)
        let address = String(afterScheme.prefix { $0 != "/" && $0 != "#" && $0 != "?" })
        var fields: [String: String] = [:]
        if let hash = text.firstIndex(of: "#") {
            for pair in text[text.index(after: hash)...].split(separator: "&") {
                let parts = pair.split(separator: "=", maxSplits: 1)
                if parts.count == 2 { fields[String(parts[0])] = String(parts[1]) }
            }
        }
        guard let code = fields["code"], let key = fields["key"], let id = fields["id"],
            !code.isEmpty, !key.isEmpty, !id.isEmpty, !address.isEmpty
        else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: [], debugDescription: "the invite URL lacks code, key or id"))
        }
        return PairingInvite(
            serverID: ServerID(id), spkiSha256: key, code: code, addresses: [address])
    }
}
