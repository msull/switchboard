import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

#if os(visionOS)
import WebKit
#endif

/// Web addresses as the user types them and as the app keeps them.
public enum WebAddress {
    /// Whether `url` is a page a web pane shows.
    public static func isWeb(_ url: URL) -> Bool {
        (url.scheme == "http" || url.scheme == "https") && !(url.host ?? "").isEmpty
    }

    /// The page `typed` names, or nil when it is not an address. One
    /// with no scheme gets `http` when its host is on the local network
    /// (`localhost`, a `.local` name, a bare name, or numbers) and
    /// `https` otherwise; any other scheme is refused.
    public static func parse(_ typed: String) -> URL? {
        let text = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !text.contains(where: \.isWhitespace) else { return nil }
        if text.contains("://") {
            guard let url = URL(string: text), isWeb(url) else { return nil }
            return url
        }
        let authority = text.prefix { $0 != "/" && $0 != "?" && $0 != "#" }
        let host = authority.split(separator: ":", omittingEmptySubsequences: false).first.map(String.init) ?? ""
        let ported = authority.contains(":")
        // A word alone is a search for a name, not a page.
        guard !host.isEmpty, host.contains(".") || ported || host == "localhost" else { return nil }
        let numbers = host.allSatisfy { $0.isNumber || $0 == "." }
        let local = host == "localhost" || host.hasSuffix(".local") || numbers || !host.contains(".")
        guard let url = URL(string: (local ? "http://" : "https://") + text), isWeb(url) else { return nil }
        return url
    }

    /// A short name for a page: its host, with the port and path when it
    /// has them, which is what tells two local pages apart.
    public static func label(_ url: URL) -> String {
        var out = url.host ?? url.absoluteString
        if let port = url.port { out += ":\(port)" }
        if url.path.count > 1 { out += url.path }
        return out
    }
}

/// What reaches a web window: one page, in a window of its own.
public struct WebWindowRequest: Codable, Hashable, Sendable {
    public let id: UUID
    public let url: URL

    public init(id: UUID = UUID(), url: URL) {
        self.id = id
        self.url = url
    }
}

/// Where a web view is, for the bar over it.
@MainActor
@Observable
final class WebPageState {
    var url: URL?
    var title = ""
    var canGoBack = false
    var canGoForward = false
    var loading = false
    /// Why the page did not load, until the next one starts.
    var failure: String?

    #if os(visionOS)
    @ObservationIgnored weak var view: WKWebView?

    func back() { view?.goBack() }
    func forward() { view?.goForward() }
    func reload() { if loading { view?.stopLoading() } else { view?.reload() } }
    #else
    func back() {}
    func forward() {}
    func reload() {}
    #endif
}

/// A web page with a bar over it: back, forward, reload, where it is,
/// and where else to put it. In a side pane, on a board tile, or filling
/// a window of its own. `navigated` hears each page the view moves to,
/// so a pane comes back where it was left.
struct WebPaneView: View {
    @Bindable var model: BrowserModel
    let url: URL
    /// In a window of its own, which has no window to pop out to.
    var windowed = false
    var navigated: ((URL) -> Void)?
    /// Empties the pane it is in.
    var close: (() -> Void)?
    @State private var page = WebPageState()
    @Environment(\.openURL) private var openURL
    @Environment(\.openWindow) private var openWindow

    private var current: URL { page.url ?? url }

    var body: some View {
        VStack(spacing: 6) {
            bar
            surface
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay(alignment: .top) {
                    if let failure = page.failure {
                        Text(failure).font(.callout).foregroundStyle(.secondary)
                            .padding(10)
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
                            .padding(12)
                    }
                }
        }
        .onChange(of: page.url) { _, new in
            if let new, new != url { navigated?(new) }
        }
    }

    @ViewBuilder
    private var surface: some View {
        #if os(visionOS)
        WebSurface(state: page, start: url)
        #else
        Text(url.absoluteString).frame(maxWidth: .infinity, maxHeight: .infinity)
        #endif
    }

    private var bar: some View {
        HStack(spacing: 6) {
            button("chevron.backward", "Back", on: page.canGoBack) { page.back() }
            button("chevron.forward", "Forward", on: page.canGoForward) { page.forward() }
            button(page.loading ? "xmark" : "arrow.clockwise", page.loading ? "Stop" : "Reload", on: true) {
                page.reload()
            }
            VStack(alignment: .leading, spacing: 0) {
                Text(page.title.isEmpty ? WebAddress.label(current) : page.title)
                    .font(.callout.bold()).lineLimit(1)
                if !page.title.isEmpty {
                    Text(WebAddress.label(current)).font(.caption2).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            PinToBoardMenu(model: model, item: .web(current), name: WebAddress.label(current))
            Menu {
                if !windowed {
                    Button {
                        openWindow(value: WebWindowRequest(url: current))
                    } label: {
                        Label("Open in its own window", systemImage: "arrow.up.forward.square")
                    }
                }
                Button {
                    openURL(current)
                } label: {
                    Label("Open in Safari", systemImage: "safari")
                }
            } label: {
                Image(systemName: "ellipsis").font(.caption)
            }
            .buttonStyle(.bordered)
            .buttonBorderShape(.circle)
            .controlSize(.mini)
            .accessibilityLabel("More")
            if let close {
                button("xmark.circle", "Empty the pane", on: true, action: close)
            }
        }
    }

    private func button(_ symbol: String, _ label: String, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).font(.caption)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .controlSize(.mini)
        .disabled(!on)
        .accessibilityLabel(label)
    }
}

/// A web page in a window of its own.
public struct WebWindowView: View {
    @Bindable var model: BrowserModel
    let request: WebWindowRequest

    public init(model: BrowserModel, request: WebWindowRequest) {
        self.model = model
        self.request = request
    }

    public var body: some View {
        WebPaneView(model: model, url: request.url, windowed: true)
            .padding(12)
            .navigationTitle(WebAddress.label(request.url))
    }
}

#if os(visionOS)
/// The system's web view, the engine Safari uses, reporting where it is
/// to a `WebPageState`.
private struct WebSurface: UIViewRepresentable {
    let state: WebPageState
    /// The page asked for; the view's own moves do not come back here.
    let start: URL

    func makeCoordinator() -> Coordinator { Coordinator(state: state, asked: start) }

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: WKWebViewConfiguration())
        view.navigationDelegate = context.coordinator
        view.uiDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = true
        context.coordinator.watch(view)
        state.view = view
        view.load(URLRequest(url: start))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        // A pane hands back each page the view moved to, so `start`
        // changes on every link followed: only a page the view is not
        // already on is one someone asked for.
        guard context.coordinator.asked != start else { return }
        context.coordinator.asked = start
        if view.url != start { view.load(URLRequest(url: start)) }
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        let state: WebPageState
        var asked: URL
        private var watches: [NSKeyValueObservation] = []

        init(state: WebPageState, asked: URL) {
            self.state = state
            self.asked = asked
        }

        /// The view's own properties are the truth about where it is; a
        /// page that changes its address without a navigation still
        /// moves them.
        func watch(_ view: WKWebView) {
            let state = state
            watches = [
                view.observe(\.url, options: [.initial, .new]) { view, _ in
                    Task { @MainActor in state.url = view.url }
                },
                view.observe(\.title, options: [.initial, .new]) { view, _ in
                    Task { @MainActor in state.title = view.title ?? "" }
                },
                view.observe(\.canGoBack, options: [.initial, .new]) { view, _ in
                    Task { @MainActor in state.canGoBack = view.canGoBack }
                },
                view.observe(\.canGoForward, options: [.initial, .new]) { view, _ in
                    Task { @MainActor in state.canGoForward = view.canGoForward }
                },
                view.observe(\.isLoading, options: [.initial, .new]) { view, _ in
                    Task { @MainActor in state.loading = view.isLoading }
                },
            ]
        }

        func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
            state.failure = nil
        }

        func webView(
            _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
        ) {
            failed(error)
        }

        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            failed(error)
        }

        private func failed(_ error: Error) {
            // A load cut short by the next one is not a failure to show.
            guard (error as NSError).code != NSURLErrorCancelled else { return }
            state.failure = error.localizedDescription
        }

        /// A link that asks for a new window opens here: a pane has one page.
        func webView(
            _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
            for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures
        ) -> WKWebView? {
            if navigationAction.targetFrame == nil { webView.load(navigationAction.request) }
            return nil
        }
    }
}
#endif
