import Foundation
import Network
import SwitchboardProtocol

/// One `_switchboard._tcp` record as browsed: the instance name and the
/// TXT entries the host publishes (`v`, `name`, `id`). A candidate, not
/// a trusted server: nothing here is believed until the pinned key
/// checks out on a connection.
public struct DiscoveredServer: Hashable, Sendable, Identifiable {
    public let instance: String
    public let txt: [String: String]
    public let endpoint: NWEndpoint

    public var id: String { instance }
    public var serverID: ServerID? { txt["id"].map(ServerID.init) }
    public var name: String? { txt["name"] }
    public var protocolVersion: Int? { txt["v"].flatMap(Int.init) }

    public init(instance: String, txt: [String: String], endpoint: NWEndpoint) {
        self.instance = instance
        self.txt = txt
        self.endpoint = endpoint
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(instance)
    }

    public static func == (lhs: DiscoveredServer, rhs: DiscoveredServer) -> Bool {
        lhs.instance == rhs.instance && lhs.txt == rhs.txt
    }
}

/// Bonjour browsing through `Network.framework`. The handler gets the
/// whole current set on every change.
public final class Discovery: @unchecked Sendable {
    private let browser: NWBrowser
    private let queue = DispatchQueue(label: "switchboard.discovery")

    public init() {
        let parameters = NWParameters()
        parameters.includePeerToPeer = true
        browser = NWBrowser(
            for: .bonjourWithTXTRecord(type: RemoteProtocol.service, domain: nil),
            using: parameters)
    }

    public func start(_ handler: @escaping @Sendable (Set<DiscoveredServer>) -> Void) {
        browser.browseResultsChangedHandler = { results, _ in
            handler(Set(results.compactMap(Discovery.server(from:))))
        }
        browser.start(queue: queue)
    }

    public func stop() {
        browser.cancel()
    }

    static func server(from result: NWBrowser.Result) -> DiscoveredServer? {
        guard case .service(let name, _, _, _) = result.endpoint else { return nil }
        var txt: [String: String] = [:]
        if case .bonjour(let record) = result.metadata {
            txt = record.dictionary
        }
        return DiscoveredServer(instance: name, txt: txt, endpoint: result.endpoint)
    }

    /// The address behind a browsed endpoint: a connection is opened to
    /// learn the path's remote host and port, then closed. The host sees
    /// a TCP connection with no handshake, which its 5 s timeout drops.
    public static func resolve(_ endpoint: NWEndpoint) async throws -> Address {
        let connection = NWConnection(to: endpoint, using: .tcp)
        let queue = DispatchQueue(label: "switchboard.resolve")
        return try await withCheckedThrowingContinuation { continuation in
            let done = Resolved()
            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    guard let remote = connection.currentPath?.remoteEndpoint,
                        case .hostPort(let host, let port) = remote
                    else {
                        done.finish { continuation.resume(throwing: RemoteError.transport("no address")) }
                        connection.cancel()
                        return
                    }
                    // An address prints with its interface ("%en0")
                    // when it has one, which a URL cannot carry.
                    let text: String
                    switch host {
                    case .ipv4(let v4): text = Discovery.bare("\(v4)")
                    case .ipv6(let v6): text = Discovery.bare("\(v6)")
                    case .name(let name, _): text = name
                    @unknown default: text = Discovery.bare("\(host)")
                    }
                    done.finish {
                        continuation.resume(returning: Address(host: text, port: port.rawValue))
                    }
                    connection.cancel()
                case .failed(let error):
                    done.finish { continuation.resume(throwing: RemoteError.transport("\(error)")) }
                    connection.cancel()
                case .cancelled:
                    done.finish { continuation.resume(throwing: RemoteError.transport("cancelled")) }
                default:
                    break
                }
            }
            connection.start(queue: queue)
        }
    }

    static func bare(_ address: String) -> String {
        address.split(separator: "%").first.map(String.init) ?? address
    }

    /// A continuation resumed once.
    private final class Resolved: @unchecked Sendable {
        private var finished = false
        private let lock = NSLock()
        func finish(_ body: () -> Void) {
            lock.lock()
            defer { lock.unlock() }
            guard !finished else { return }
            finished = true
            body()
        }
    }
}
