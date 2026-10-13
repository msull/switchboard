// A command-line browser for the Mac: discovers hosts, pairs with the
// QR code's text, and prints the unified view on a timer. Its registry
// and credentials live under ~/Library/Application Support/switchboard-remote
// unless `SWITCHBOARD_REMOTE_DIR` says otherwise.
import Foundation
import SwitchboardClientCore
import SwitchboardProtocol
import SwitchboardTransport

let usage = """
    switchboard-remote discover [seconds]       browse _switchboard._tcp and print what answers
    switchboard-remote pair '<invite json>'     pair with a host's open window (the QR code's text)
    switchboard-remote pair <host:port> <code> <spki_sha256>
    switchboard-remote hosts                    the known hosts and their state
    switchboard-remote forget <server id>
    switchboard-remote watch [seconds]          the unified view, refreshed on a timer
    """

// Line-buffered even into a file, so a `watch` log reads as it goes.
setvbuf(stdout, nil, _IOLBF, 0)

// `SWITCHBOARD_REMOTE_DIR` keeps a run's registry and credentials out
// of the real ones (Application Support resolves the home directory
// itself, so `HOME` alone does not move it).
let support = ProcessInfo.processInfo.environment["SWITCHBOARD_REMOTE_DIR"].map {
    URL(fileURLWithPath: $0)
} ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    .appendingPathComponent("switchboard-remote")
let manager = ConnectionManager(
    registry: FileRegistry(url: support.appendingPathComponent("servers.json")),
    credentials: FileCredentials(url: support.appendingPathComponent("credentials.json")),
    deviceName: "\(ProcessInfo.processInfo.hostName) (cli)")

func stateWord(_ host: HostStatus) -> String {
    switch host.state {
    case .discovered: return "discovered"
    case .unpaired: return host.identityChanged ? "key changed; pair again" : "needs pairing"
    case .pairing: return "pairing"
    case .connecting: return "connecting"
    case .syncing: return "syncing"
    case .connected: return "connected"
    case .reconnecting(let n): return "reconnecting (\(n))"
    case .offline(let reason): return "offline: \(reason)"
    }
}

func printStore(_ store: UnifiedStore) {
    print("hosts")
    for host in store.hosts {
        let addr = host.server.addresses.first.map { "\($0)" } ?? "no address"
        print("  \(host.server.name)  \(host.id.short)  \(addr)  \(stateWord(host))")
        if let error = host.lastError, !host.state.isConnected { print("    \(error)") }
    }
    print("sessions")
    for item in store.sessions {
        let mark = item.stale ? " (offline)" : ""
        print("  [\(item.serverName)\(mark)] \(item.session.name)  \(item.session.kind.rawValue)  \(item.session.card)")
    }
    print("sets")
    for item in store.sets {
        let mark = item.stale ? " (offline)" : ""
        print("  [\(item.serverName)\(mark)] \(item.set.name)  \(item.set.items.count) pins")
    }
}

func discover(seconds: Double) async {
    let discovery = Discovery()
    let seen = Seen()
    discovery.start { set in seen.update(set) }
    try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    discovery.stop()
    let found = seen.current
    if found.isEmpty { print("nothing advertised _switchboard._tcp in \(Int(seconds)) s") }
    for record in found.sorted(by: { $0.instance < $1.instance }) {
        let address = (try? await Discovery.resolve(record.endpoint)).map { "\($0)" } ?? "unresolved"
        print("\(record.instance)  id=\(record.serverID?.raw ?? "?")  v=\(record.protocolVersion ?? 0)  \(address)")
    }
}

final class Seen: @unchecked Sendable {
    private let lock = NSLock()
    private var set: Set<DiscoveredServer> = []
    func update(_ new: Set<DiscoveredServer>) {
        lock.lock()
        set = new
        lock.unlock()
    }
    var current: Set<DiscoveredServer> {
        lock.lock()
        defer { lock.unlock() }
        return set
    }
}

func run() async -> Int32 {
    let args = Array(CommandLine.arguments.dropFirst())
    guard let command = args.first else {
        print(usage)
        return 2
    }
    do {
        try await manager.load()
        switch command {
        case "discover":
            await discover(seconds: Double(args.dropFirst().first ?? "") ?? 3)
        case "pair":
            let rest = Array(args.dropFirst())
            let invite: PairingInvite
            if rest.count == 1 {
                invite = try PairingInvite.parse(rest[0])
            } else if rest.count == 3 {
                // The id is learned from the pairing reply; a placeholder
                // until then is refused, so read /info unpinned first.
                guard let address = Address(rest[0]) else { throw RemoteError.transport("bad address") }
                let probe = HostClient(address: address, pin: .expect(spkiSha256: rest[2]))
                let info = try await probe.info()
                invite = PairingInvite(
                    serverID: info.body.serverID, spkiSha256: rest[2], code: rest[1],
                    addresses: [rest[0]])
            } else {
                print(usage)
                return 2
            }
            try await manager.pair(invite: invite)
            await manager.rebuild()
            printStore(await manager.store)
        case "hosts":
            await manager.connectAll()
            printStore(await manager.store)
        case "forget":
            guard let id = args.dropFirst().first else {
                print(usage)
                return 2
            }
            await manager.forget(ServerID(id))
            print("forgotten")
        case "watch":
            let interval = Double(args.dropFirst().first ?? "") ?? 5
            await manager.start(interval: interval)
            while true {
                try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
                print("\u{1B}[2J\u{1B}[H\(Date())")
                printStore(await manager.store)
            }
        default:
            print(usage)
            return 2
        }
    } catch {
        FileHandle.standardError.write(Data("error: \(error)\n".utf8))
        return 1
    }
    return 0
}

exit(await run())
