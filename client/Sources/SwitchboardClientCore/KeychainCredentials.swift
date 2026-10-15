import Foundation
import Security
import SwitchboardProtocol

/// Tokens and pins as generic-password items under one service, an
/// account per server and kind. The apps' store; signed builds keep
/// their items across updates.
public final class KeychainCredentials: CredentialStore, @unchecked Sendable {
    public let service: String

    public init(service: String = "switchboard-remote") {
        self.service = service
    }

    public enum KeychainError: Error, CustomStringConvertible {
        case status(OSStatus)
        public var description: String {
            if case .status(let code) = self {
                return (SecCopyErrorMessageString(code, nil) as String?) ?? "OSStatus \(code)"
            }
            return "keychain"
        }
    }

    private func query(_ account: String) -> [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
        ]
    }

    private func read(_ account: String) throws -> String? {
        var q = query(account)
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw KeychainError.status(status) }
        guard let data = item as? Data else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    private func write(_ value: String?, _ account: String) throws {
        let status = SecItemDelete(query(account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError.status(status)
        }
        guard let value else { return }
        var q = query(account)
        q[kSecValueData] = Data(value.utf8)
        q[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(q as CFDictionary, nil)
        guard added == errSecSuccess else { throw KeychainError.status(added) }
    }

    public func token(for server: ServerID) throws -> String? {
        try read("token:\(server.raw)")
    }

    public func setToken(_ token: String?, for server: ServerID) throws {
        try write(token, "token:\(server.raw)")
    }

    public func pin(for server: ServerID) throws -> String? {
        try read("pin:\(server.raw)")
    }

    public func setPin(_ pin: String?, for server: ServerID) throws {
        try write(pin, "pin:\(server.raw)")
    }
}
