import CryptoKit
import Foundation
import Security

/// The SPKI hash the host's `/info` reports and the QR code carries:
/// SHA-256 over the certificate's `SubjectPublicKeyInfo`, hex. The host
/// signs with a P-256 key (rcgen's default) and the client pins nothing
/// weaker: a certificate over another key type does not hash at all.
public enum SPKI {
    /// The DER prefix of a P-256 `SubjectPublicKeyInfo`, ahead of the
    /// 65-byte uncompressed point `SecKeyCopyExternalRepresentation`
    /// gives back.
    static let p256Prefix: [UInt8] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ]

    public static func sha256Hex(ofCertificate certificate: SecCertificate) -> String? {
        guard let key = SecCertificateCopyKey(certificate),
            let attributes = SecKeyCopyAttributes(key) as? [CFString: Any],
            let type = attributes[kSecAttrKeyType] as? String,
            type == (kSecAttrKeyTypeECSECPrimeRandom as String),
            let bits = attributes[kSecAttrKeySizeInBits] as? Int, bits == 256,
            let point = SecKeyCopyExternalRepresentation(key, nil) as Data?,
            point.count == 65, point.first == 0x04
        else { return nil }
        var spki = Data(p256Prefix)
        spki.append(point)
        return SHA256.hash(data: spki).map { String(format: "%02x", $0) }.joined()
    }

    public static func sha256Hex(ofCertificateDER der: Data) -> String? {
        guard let certificate = SecCertificateCreateWithData(nil, der as CFData) else {
            return nil
        }
        return sha256Hex(ofCertificate: certificate)
    }
}

/// How a connection decides whether to trust the host's certificate.
public enum PinPolicy: Sendable, Equatable {
    /// Only a certificate over the pinned key, from the QR code or the
    /// Keychain; anything else fails the handshake and no request is sent.
    case expect(spkiSha256: String)
    /// Accept whatever key the host presents and remember its hash: for
    /// `/info` on a discovered, unpaired host. Never for a request that
    /// carries a token or a code.
    case observe
}

/// The `URLSession` delegate that enforces a `PinPolicy` on every
/// server-trust challenge; `URLSessionWebSocketTask` shares it.
public final class PinnedSessionDelegate: NSObject, URLSessionDelegate, @unchecked Sendable {
    public let policy: PinPolicy
    private let lock = NSLock()
    private var observedHash: String?
    private var mismatchHash: String?

    public init(policy: PinPolicy) {
        self.policy = policy
    }

    /// The hash the host presented under `.observe`, once a handshake ran.
    public var observed: String? {
        lock.lock()
        defer { lock.unlock() }
        return observedHash
    }

    /// The hash a refused host presented under `.expect`.
    public var mismatch: String? {
        lock.lock()
        defer { lock.unlock() }
        return mismatchHash
    }

    public func urlSession(
        _ session: URLSession, didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
            let trust = challenge.protectionSpace.serverTrust,
            let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
            let leaf = chain.first,
            let hash = SPKI.sha256Hex(ofCertificate: leaf)
        else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        switch policy {
        case .expect(let expected):
            if hash == expected {
                completionHandler(.useCredential, URLCredential(trust: trust))
            } else {
                lock.lock()
                mismatchHash = hash
                lock.unlock()
                completionHandler(.cancelAuthenticationChallenge, nil)
            }
        case .observe:
            lock.lock()
            observedHash = hash
            lock.unlock()
            completionHandler(.useCredential, URLCredential(trust: trust))
        }
    }
}
