import Foundation
import SwitchboardProtocol

/// Where a host answers.
public struct Address: Hashable, Codable, Sendable, CustomStringConvertible {
    public let host: String
    public let port: UInt16

    public init(host: String, port: UInt16) {
        self.host = host
        self.port = port
    }

    /// `host:port`, as the QR code and the dialog print it.
    public init?(_ text: String) {
        guard let colon = text.lastIndex(of: ":"),
            let port = UInt16(text[text.index(after: colon)...])
        else { return nil }
        var host = String(text[..<colon])
        if host.hasPrefix("["), host.hasSuffix("]") {
            host = String(host.dropFirst().dropLast())
        }
        guard !host.isEmpty else { return nil }
        self.init(host: host, port: port)
    }

    public var description: String {
        host.contains(":") ? "[\(host)]:\(port)" : "\(host):\(port)"
    }

    public var baseURL: URL {
        URL(string: "https://\(description)/api/v1/")!
    }
}

public enum RemoteError: Error, CustomStringConvertible, Sendable {
    /// The host answered with an error body.
    case refused(status: Int, code: String, message: String)
    /// The handshake failed the pin; `seen` is the hash the host showed.
    case pinMismatch(expected: String, seen: String?)
    case transport(String)
    case badReply(String)

    public var description: String {
        switch self {
        case .refused(let status, let code, let message): return "\(status) \(code): \(message)"
        case .pinMismatch(let expected, let seen):
            return "the host's key \(seen ?? "?") is not the pinned \(expected)"
        case .transport(let text): return text
        case .badReply(let text): return "bad reply: \(text)"
        }
    }

    /// A refusal that means the token is no longer good.
    public var isUnauthorized: Bool {
        if case .refused(401, _, _) = self { return true }
        return false
    }
}

/// An open events socket: the frames as they arrive, a way to send a
/// text frame (`watch`, `unwatch`), and the close.
public struct EventSubscription: Sendable {
    public let frames: AsyncThrowingStream<EventFrame, Error>
    public let send: @Sendable (String) async throws -> Void
    public let close: @Sendable () -> Void

    public init(
        frames: AsyncThrowingStream<EventFrame, Error>,
        send: @escaping @Sendable (String) async throws -> Void = { _ in },
        close: @escaping @Sendable () -> Void
    ) {
        self.frames = frames
        self.send = send
        self.close = close
    }
}

/// What a per-host connection needs from the network, so the core is
/// tested against a fake and `HostClient` is the real one.
public protocol HostAPI: Sendable {
    func info() async throws -> Envelope<ServerInfo>
    /// Subscribe to the host's events; the first frame is `hello`.
    func events() async throws -> EventSubscription
    func pair(_ request: PairRequest) async throws -> Envelope<PairReply>
    func projects() async throws -> Envelope<ProjectsBody>
    func spaces() async throws -> Envelope<SpacesBody>
    func sets(space: String) async throws -> Envelope<SetsBody>
    func sessions(project: String) async throws -> Envelope<SessionsBody>
    func session(_ id: String) async throws -> Envelope<SessionBody>
    func waiting() async throws -> Envelope<SessionsBody>
    func screen(_ id: String, lines: Int?) async throws -> Envelope<ScreenBody>
    /// A page of the conversation: `from` is a turn number or `"last"`.
    /// A page anchored at `from` (a turn number or `last`) or ending
    /// before turn `before`; the two do not go together.
    func conversation(_ id: String, from: String?, before: Int?, limit: Int?) async throws
        -> Envelope<ConversationBody>
    func lastResponse(_ id: String) async throws -> Envelope<LastResponseBody>
    /// One turn whole, with the detail its page cut.
    func turn(_ id: String, n: Int) async throws -> Envelope<TurnBody>
    func runs(_ id: String) async throws -> Envelope<RunsBody>
    func runOutput(_ id: String, run: Int, lines: Int?) async throws -> Envelope<RunOutputBody>
    /// A command under the control scope; `op` is its idempotency key.
    func command(_ id: String, _ verb: CommandVerb, op: String, text: String?) async throws
        -> Envelope<CommandBody>
    /// What became of an op, after a command got no answer.
    func opStatus(_ op: String) async throws -> Envelope<OpStatusBody>
    /// A session's draft: the host's editor for it (§4.5).
    func draft(_ id: String) async throws -> Envelope<DraftBody>
    /// One of the editor's tools on the draft, or `send` with an `op`.
    func draftAction(_ id: String, _ body: DraftActionBody) async throws -> Envelope<DraftBody>
    /// The device's speech events into the draft.
    func dictate(_ id: String, _ body: DictationBody) async throws -> Envelope<DraftBody>
    /// A pinned file of a project, whole (§4.3).
    func file(project: String, path: String) async throws -> Envelope<FileBody>
    func tickets() async throws -> Envelope<TicketsBody>
    func ticket(_ id: String) async throws -> Envelope<TicketView>
    func decide(_ ticket: String, _ request: DecideRequest) async throws -> Envelope<DecidedBody>
    func resumeTicket(_ ticket: String) async throws -> Envelope<TicketResumedBody>
}

/// One host over HTTPS with the pinning delegate and a bearer token.
public final class HostClient: HostAPI, @unchecked Sendable {
    public let address: Address
    public let delegate: PinnedSessionDelegate
    private let session: URLSession
    private let lock = NSLock()
    private var bearer: String?

    public init(address: Address, pin: PinPolicy, token: String? = nil) {
        self.address = address
        self.delegate = PinnedSessionDelegate(policy: pin)
        self.bearer = token
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 10
        config.waitsForConnectivity = false
        config.httpAdditionalHeaders = ["Accept": "application/json"]
        self.session = URLSession(configuration: config, delegate: delegate, delegateQueue: nil)
    }

    deinit {
        session.invalidateAndCancel()
    }

    public var token: String? {
        get {
            lock.lock()
            defer { lock.unlock() }
            return bearer
        }
        set {
            lock.lock()
            bearer = newValue
            lock.unlock()
        }
    }

    public func info() async throws -> Envelope<ServerInfo> {
        try await get("info", query: nil)
    }

    /// The events socket over the same session and so the same pin;
    /// the bearer goes in the upgrade request's header, never the URL.
    public func events() async throws -> EventSubscription {
        guard let token else {
            throw RemoteError.refused(status: 401, code: "unauthorized", message: "no token")
        }
        var components = URLComponents(url: address.baseURL.appendingPathComponent("events"), resolvingAgainstBaseURL: false)!
        components.scheme = "wss"
        var request = URLRequest(url: components.url!)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let task = session.webSocketTask(with: request)
        task.resume()
        let frames = AsyncThrowingStream<EventFrame, Error> { continuation in
            let pump = Task {
                do {
                    while !Task.isCancelled {
                        let message = try await task.receive()
                        let data: Data
                        switch message {
                        case .string(let text): data = Data(text.utf8)
                        case .data(let bytes): data = bytes
                        @unknown default: continue
                        }
                        continuation.yield(try Wire.decode(EventFrame.self, from: data))
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: RemoteError.transport("events: \(error.localizedDescription)"))
                }
            }
            continuation.onTermination = { _ in
                pump.cancel()
                task.cancel(with: .normalClosure, reason: nil)
            }
        }
        return EventSubscription(
            frames: frames,
            send: { text in
                do {
                    try await task.send(.string(text))
                } catch {
                    throw RemoteError.transport("events: \(error.localizedDescription)")
                }
            },
            close: { task.cancel(with: .normalClosure, reason: nil) })
    }

    public func pair(_ request: PairRequest) async throws -> Envelope<PairReply> {
        guard case .expect = delegate.policy else {
            throw RemoteError.transport("a pairing needs the host's key pinned first")
        }
        var req = URLRequest(url: address.baseURL.appendingPathComponent("pair"))
        req.httpMethod = "POST"
        req.httpBody = try Wire.encoder().encode(request)
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        return try await send(req)
    }

    public func projects() async throws -> Envelope<ProjectsBody> {
        try await get("projects", query: nil)
    }

    public func spaces() async throws -> Envelope<SpacesBody> {
        try await get("spaces", query: nil)
    }

    public func sets(space: String) async throws -> Envelope<SetsBody> {
        try await get("spaces/\(space)/sets", query: nil)
    }

    public func sessions(project: String) async throws -> Envelope<SessionsBody> {
        try await get("projects/\(project)/sessions", query: nil)
    }

    public func session(_ id: String) async throws -> Envelope<SessionBody> {
        try await get("sessions/\(id)", query: nil)
    }

    public func waiting() async throws -> Envelope<SessionsBody> {
        try await get("waiting", query: nil)
    }

    public func screen(_ id: String, lines: Int?) async throws -> Envelope<ScreenBody> {
        try await get("sessions/\(id)/screen", query: lines.map { "lines=\($0)" })
    }

    public func conversation(_ id: String, from: String?, before: Int?, limit: Int?) async throws
        -> Envelope<ConversationBody>
    {
        var parts: [String] = []
        if let from { parts.append("from=\(from)") }
        if let before { parts.append("before=\(before)") }
        if let limit { parts.append("limit=\(limit)") }
        return try await get(
            "sessions/\(id)/conversation", query: parts.isEmpty ? nil : parts.joined(separator: "&"))
    }

    public func lastResponse(_ id: String) async throws -> Envelope<LastResponseBody> {
        try await get("sessions/\(id)/last-response", query: nil)
    }

    public func turn(_ id: String, n: Int) async throws -> Envelope<TurnBody> {
        try await get("sessions/\(id)/conversation/\(n)", query: nil)
    }

    public func command(_ id: String, _ verb: CommandVerb, op: String, text: String?) async throws
        -> Envelope<CommandBody>
    {
        var body: [String: String] = ["op": op]
        if let text { body["text"] = text }
        return try await post("sessions/\(id)/\(verb.rawValue)", json: body)
    }

    public func opStatus(_ op: String) async throws -> Envelope<OpStatusBody> {
        try await get("ops/\(op)", query: nil)
    }

    public func draft(_ id: String) async throws -> Envelope<DraftBody> {
        try await get("sessions/\(id)/draft", query: nil)
    }

    public func draftAction(_ id: String, _ body: DraftActionBody) async throws -> Envelope<DraftBody> {
        try await post("sessions/\(id)/draft", json: body)
    }

    public func dictate(_ id: String, _ body: DictationBody) async throws -> Envelope<DraftBody> {
        try await post("sessions/\(id)/dictation", json: body)
    }

    public func file(project: String, path: String) async throws -> Envelope<FileBody> {
        let escaped = path.split(separator: "/", omittingEmptySubsequences: false)
            .map { $0.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? String($0) }
            .joined(separator: "/")
        return try await get("projects/\(project)/files/\(escaped)", query: nil)
    }

    public func tickets() async throws -> Envelope<TicketsBody> {
        try await get("dispatch/tickets", query: nil)
    }

    public func ticket(_ id: String) async throws -> Envelope<TicketView> {
        try await get("dispatch/tickets/\(id)", query: nil)
    }

    public func decide(_ ticket: String, _ request: DecideRequest) async throws -> Envelope<DecidedBody> {
        try await post("dispatch/tickets/\(ticket)/decide", json: request)
    }

    public func resumeTicket(_ ticket: String) async throws -> Envelope<TicketResumedBody> {
        try await post("dispatch/tickets/\(ticket)/resume", json: [String: String]())
    }

    public func runs(_ id: String) async throws -> Envelope<RunsBody> {
        try await get("sessions/\(id)/runs", query: nil)
    }

    public func runOutput(_ id: String, run: Int, lines: Int?) async throws -> Envelope<RunOutputBody> {
        try await get("sessions/\(id)/runs/\(run)/output", query: lines.map { "lines=\($0)" })
    }

    private func get<T: Decodable & Sendable>(_ path: String, query: String?) async throws
        -> Envelope<T>
    {
        var url = address.baseURL.appendingPathComponent(path)
        if let query, var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) {
            parts.percentEncodedQuery = query
            url = parts.url ?? url
        }
        var req = URLRequest(url: url)
        req.httpMethod = "GET"
        return try await send(req)
    }

    private func post<T: Decodable & Sendable, B: Encodable>(_ path: String, json: B) async throws
        -> Envelope<T>
    {
        var req = URLRequest(url: address.baseURL.appendingPathComponent(path))
        req.httpMethod = "POST"
        req.httpBody = try JSONEncoder().encode(json)
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        return try await send(req)
    }

    /// What a failed request means. A key mismatch is declared only
    /// when the delegate saw a certificate under another key: a
    /// request cut short (the host relaunching, the app's own task
    /// cancelled, the network gone) is a transport failure to retry,
    /// never a reason to drop the pairing.
    static func classify(_ error: Error, policy: PinPolicy, mismatch: String?) -> RemoteError {
        if case .expect(let expected) = policy, let seen = mismatch {
            return .pinMismatch(expected: expected, seen: seen)
        }
        return .transport(error.localizedDescription)
    }

    private func send<T: Decodable & Sendable>(_ request: URLRequest) async throws -> Envelope<T> {
        var request = request
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw Self.classify(error, policy: delegate.policy, mismatch: delegate.mismatch)
        }
        guard let http = response as? HTTPURLResponse else {
            throw RemoteError.badReply("not HTTP")
        }
        if http.statusCode == 200 {
            do {
                return try Wire.decode(Envelope<T>.self, from: data)
            } catch {
                throw RemoteError.badReply("\(error)")
            }
        }
        if let failure = try? Wire.decode(ErrorEnvelope.self, from: data) {
            throw RemoteError.refused(
                status: http.statusCode, code: failure.error.code, message: failure.error.message)
        }
        throw RemoteError.refused(
            status: http.statusCode, code: "http",
            message: String(decoding: data.prefix(200), as: UTF8.self))
    }
}
