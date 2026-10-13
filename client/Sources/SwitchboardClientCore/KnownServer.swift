import Foundation
import SwitchboardProtocol
import SwitchboardTransport

/// A host the user paired with or added by hand, kept apart from what
/// discovery currently shows. Its token and pinned key hash live in the
/// credential store, never here.
public struct KnownServer: Codable, Hashable, Sendable, Identifiable {
    public var id: ServerID
    /// The display name, as `/info` last reported it.
    public var name: String
    /// Where it last answered, preferred first. A changed address is
    /// not a new server: the pin decides.
    public var addresses: [Address]
    public var enabled: Bool
    public var pairedAt: Date?
    public var scope: Scope?
    /// Set when the host's key changed under this id or the token was
    /// refused: the entry and its views stay, the next step is a pairing.
    public var needsPairing: Bool

    public init(
        id: ServerID, name: String, addresses: [Address], enabled: Bool = true,
        pairedAt: Date? = nil, scope: Scope? = nil, needsPairing: Bool = true
    ) {
        self.id = id
        self.name = name
        self.addresses = addresses
        self.enabled = enabled
        self.pairedAt = pairedAt
        self.scope = scope
        self.needsPairing = needsPairing
    }
}

/// Where the known-server list is kept: the app's container, or memory.
public protocol RegistryStore: Sendable {
    func load() throws -> [KnownServer]
    func save(_ servers: [KnownServer]) throws
}

/// The per-server secrets: the bearer token and the pinned key hash.
public protocol CredentialStore: Sendable {
    func token(for server: ServerID) throws -> String?
    func setToken(_ token: String?, for server: ServerID) throws
    func pin(for server: ServerID) throws -> String?
    func setPin(_ pin: String?, for server: ServerID) throws
}

extension CredentialStore {
    public func forget(_ server: ServerID) throws {
        try setToken(nil, for: server)
        try setPin(nil, for: server)
    }
}

public final class MemoryRegistry: RegistryStore, @unchecked Sendable {
    private let lock = NSLock()
    private var servers: [KnownServer] = []

    public init() {}

    public func load() throws -> [KnownServer] {
        lock.lock()
        defer { lock.unlock() }
        return servers
    }

    public func save(_ servers: [KnownServer]) throws {
        lock.lock()
        self.servers = servers
        lock.unlock()
    }
}

public final class MemoryCredentials: CredentialStore, @unchecked Sendable {
    private let lock = NSLock()
    private var tokens: [ServerID: String] = [:]
    private var pins: [ServerID: String] = [:]

    public init() {}

    public func token(for server: ServerID) throws -> String? {
        lock.lock()
        defer { lock.unlock() }
        return tokens[server]
    }

    public func setToken(_ token: String?, for server: ServerID) throws {
        lock.lock()
        tokens[server] = token
        lock.unlock()
    }

    public func pin(for server: ServerID) throws -> String? {
        lock.lock()
        defer { lock.unlock() }
        return pins[server]
    }

    public func setPin(_ pin: String?, for server: ServerID) throws {
        lock.lock()
        pins[server] = pin
        lock.unlock()
    }

    /// Everything held, for a test that asserts a forget emptied it.
    public var isEmpty: Bool {
        lock.lock()
        defer { lock.unlock() }
        return tokens.isEmpty && pins.isEmpty
    }
}

/// The known-server list as one JSON file.
public final class FileRegistry: RegistryStore, @unchecked Sendable {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    public func load() throws -> [KnownServer] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode([KnownServer].self, from: Data(contentsOf: url))
    }

    public func save(_ servers: [KnownServer]) throws {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try encoder.encode(servers).write(to: url, options: .atomic)
    }
}

/// Credentials in one owner-only JSON file: for the command-line
/// browser, whose unsigned binary would otherwise prompt for the login
/// Keychain on every rebuild. The apps use `KeychainCredentials`.
public final class FileCredentials: CredentialStore, @unchecked Sendable {
    public let url: URL
    private let lock = NSLock()

    public init(url: URL) {
        self.url = url
    }

    private struct Entry: Codable {
        var token: String?
        var pin: String?
    }

    private func read() throws -> [String: Entry] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        return try JSONDecoder().decode([String: Entry].self, from: Data(contentsOf: url))
    }

    private func write(_ entries: [String: Entry]) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let data = try JSONEncoder().encode(entries)
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }

    private func update(_ server: ServerID, _ change: (inout Entry) -> Void) throws {
        lock.lock()
        defer { lock.unlock() }
        var entries = try read()
        var entry = entries[server.raw] ?? Entry()
        change(&entry)
        if entry.token == nil && entry.pin == nil {
            entries[server.raw] = nil
        } else {
            entries[server.raw] = entry
        }
        try write(entries)
    }

    public func token(for server: ServerID) throws -> String? {
        lock.lock()
        defer { lock.unlock() }
        return try read()[server.raw]?.token
    }

    public func setToken(_ token: String?, for server: ServerID) throws {
        try update(server) { $0.token = token }
    }

    public func pin(for server: ServerID) throws -> String? {
        lock.lock()
        defer { lock.unlock() }
        return try read()[server.raw]?.pin
    }

    public func setPin(_ pin: String?, for server: ServerID) throws {
        try update(server) { $0.pin = pin }
    }
}
