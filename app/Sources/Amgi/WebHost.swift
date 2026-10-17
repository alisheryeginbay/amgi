import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// The one web view behind every detail pane, kept for the life of the app so
/// switching panes never reloads the page. Swift owns the state and sends it to the
/// page; the page sends back what the user did.
@MainActor
final class WebHost: NSObject {
    let webView: AmgiWebView
    /// Called for everything the page sends except `ready`.
    var onMessage: (WebMessage) -> Void = { _ in }
    /// Items for a right-click on browse rows, given the cards it acts on.
    var contextMenu: ([String]) -> [NSMenuItem] = { _ in [] }

    /// The open collection, served at `amgi://collection/`.
    var collectionRoot: URL? {
        get { schemeHandler.collectionRoot }
        set { schemeHandler.collectionRoot = newValue }
    }

    private let schemeHandler = SchemeHandler()
    private var ready = false
    private var state: WebState?
    // What the page has, to send only what changed.
    private var sentPane: WebPane?
    private var sentEnvironment: EnvironmentPayload?
    private var sentIcons: [String: String]?
    private var sentStudy: StudyPayload?
    private var sentDashboard: DashboardPayload?
    private var sentBrowse: BrowseSource?
    private var sentSearch: String?

    override init() {
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(schemeHandler, forURLScheme: SchemeHandler.scheme)
        // Card sounds play when a card appears, without a click first.
        configuration.mediaTypesRequiringUserActionForPlayback = []
        webView = AmgiWebView(frame: .zero, configuration: configuration)
        super.init()

        configuration.userContentController.add(WeakMessageHandler(self), name: "amgi")
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsMagnification = false
        webView.allowsBackForwardNavigationGestures = false
        // Transparent, so the window's background shows through as behind native views.
        webView.setValue(false, forKey: "drawsBackground")
        webView.underPageBackgroundColor = .clear
        #if DEBUG
        webView.isInspectable = true
        #endif
        webView.contextMenu = { [weak self] ids in self?.contextMenu(ids) ?? [] }
        webView.onMenuClosed = { [weak self] in self?.command("contextMenuClosed") }
        monitorStudyKeys()
        load()
    }

    /// The bundled page, or a development server given in `AMGI_WEB_URL`
    /// (`cd web && npm run dev`, then `AMGI_WEB_URL=http://localhost:5173 swift run`).
    private func load() {
        ready = false
        if let dev = ProcessInfo.processInfo.environment["AMGI_WEB_URL"], let url = URL(string: dev) {
            webView.load(URLRequest(url: url))
        } else {
            webView.load(URLRequest(url: SchemeHandler.pageURL))
        }
    }

    /// Send the parts of `state` the page doesn't have yet.
    func apply(_ state: WebState) {
        self.state = state
        guard ready else { return }
        send("environment", state.environment, &sentEnvironment)
        send("icons", state.icons, &sentIcons)
        send("search", state.search, &sentSearch)
        send("study", state.study, &sentStudy)
        if let dashboard = state.dashboard {
            send("dashboard", dashboard, &sentDashboard)
        }
        if let browse = state.browse, browse != sentBrowse {
            sentBrowse = browse
            post("browse", BrowsePayload(browse))
        }
        send("pane", state.pane, &sentPane)
    }

    /// Study keys reach the card wherever focus is, so selecting Review leaves the sidebar
    /// focused: arrows still move through it, and these keys answer the card instead of
    /// jumping to rows. Text being typed and the page's own key handling are left alone.
    private static let studyKeys: Set<String> = [" ", "1", "2", "3", "4", "r"]
    private var keyMonitor: Any?

    private func monitorStudyKeys() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            // Monitors run on the main thread.
            let window = event.window
            let key = event.charactersIgnoringModifiers?.lowercased() ?? ""
            let plain = event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty
            let repeated = event.isARepeat
            let forwarded = MainActor.assumeIsolated { () -> Bool in
                guard let self, plain, Self.studyKeys.contains(key),
                      self.ready, self.state?.pane == .study,
                      let window, window === self.webView.window,
                      window.firstResponder !== self.webView,
                      !(window.firstResponder is NSText)
                else { return false }
                // Swallowed but not acted on, as the page ignores held keys.
                if !repeated {
                    self.webView.callAsyncJavaScript("window.amgi.key(key)", arguments: ["key": key], in: nil, in: .page)
                }
                return true
            }
            return forwarded ? nil : event
        }
    }

    /// A command for the page, such as `replay`.
    func command(_ name: String) {
        guard ready else { return }
        webView.callAsyncJavaScript("window.amgi.command(name)", arguments: ["name": name], in: nil, in: .page)
    }

    private func send<Value: Encodable & Equatable>(_ part: String, _ value: Value, _ sent: inout Value?) {
        guard value != sent else { return }
        sent = value
        post(part, value)
    }

    private func post(_ part: String, _ value: some Encodable) {
        guard let data = try? JSONEncoder().encode(value), let json = String(data: data, encoding: .utf8) else { return }
        webView.callAsyncJavaScript(
            "window.amgi.receive(part, json)",
            arguments: ["part": part, "json": json],
            in: nil,
            in: .page
        )
    }

    fileprivate func receive(_ body: Any) {
        guard let message = WebMessage(body) else { return }
        switch message {
        case .ready:
            // A fresh page has nothing; send it everything.
            ready = true
            sentPane = nil
            sentEnvironment = nil
            sentIcons = nil
            sentStudy = nil
            sentDashboard = nil
            sentBrowse = nil
            sentSearch = nil
            if let state {
                self.state = nil
                apply(state)
            }
        case .contextCards(let ids):
            webView.contextCardIDs = ids
        default:
            onMessage(message)
        }
    }
}

extension WebHost: WKNavigationDelegate, WKUIDelegate {
    func webView(
        _ webView: WKWebView,
        decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        // Links in cards open in the browser; the page itself never navigates away.
        if action.navigationType == .linkActivated, let url = action.request.url {
            NSWorkspace.shared.open(url)
            decisionHandler(.cancel)
        } else {
            decisionHandler(.allow)
        }
    }

    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if let url = action.request.url { NSWorkspace.shared.open(url) }
        return nil
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        load()
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        #if DEBUG
        print("Web page failed to load:", error)
        #endif
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        #if DEBUG
        print("Web page failed:", error)
        #endif
    }
}

/// The page's message handler, held weakly: the content controller keeps its handlers alive.
private final class WeakMessageHandler: NSObject, WKScriptMessageHandler {
    weak var host: WebHost?

    init(_ host: WebHost) {
        self.host = host
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        host?.receive(message.body)
    }
}

/// The web view, with a context menu that keeps only text actions and adds the app's
/// own items for the browse rows clicked.
final class AmgiWebView: WKWebView {
    /// Set by the page while handling the right-click, just before the menu opens.
    var contextCardIDs: [String] = []
    var contextMenu: ([String]) -> [NSMenuItem] = { _ in [] }
    var onMenuClosed: () -> Void = {}

    private static let kept: Set<String> = {
        var kept: Set<String> = [
            "WKMenuItemIdentifierCopy", "WKMenuItemIdentifierCopyImage", "WKMenuItemIdentifierCopyLink",
            "WKMenuItemIdentifierLookUp", "WKMenuItemIdentifierTranslate", "WKMenuItemIdentifierSearchWeb",
            "WKMenuItemIdentifierShareMenu", "WKMenuItemIdentifierSpeechMenu",
        ]
        #if DEBUG
        kept.insert("WKMenuItemIdentifierInspectElement")
        #endif
        return kept
    }()

    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        for item in menu.items.reversed() where !item.isSeparatorItem && !Self.kept.contains(item.identifier?.rawValue ?? "") {
            menu.removeItem(item)
        }
        // Separators left at either end or next to each other.
        var previousWasSeparator = true
        for item in menu.items {
            if item.isSeparatorItem && previousWasSeparator { menu.removeItem(item) }
            previousWasSeparator = item.isSeparatorItem
        }
        if menu.items.last?.isSeparatorItem == true { menu.removeItem(at: menu.items.count - 1) }

        let items = contextCardIDs.isEmpty ? [] : contextMenu(contextCardIDs)
        if !items.isEmpty && !menu.items.isEmpty { menu.insertItem(.separator(), at: 0) }
        for item in items.reversed() { menu.insertItem(item, at: 0) }
        contextCardIDs = []
        super.willOpenMenu(menu, with: event)
    }

    override func didCloseMenu(_ menu: NSMenu, with event: NSEvent?) {
        super.didCloseMenu(menu, with: event)
        onMenuClosed()
    }

}

/// A menu item that runs a closure.
final class ActionMenuItem: NSMenuItem {
    private let handler: () -> Void

    init(_ title: String, handler: @escaping () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
    }

    required init(coder: NSCoder) { fatalError("not used") }

    @objc private func run() { handler() }
}

/// Serves `amgi://app/…` from the bundled page and `amgi://collection/…` from the open
/// collection folder, never anything outside it. Byte ranges are honoured, which media
/// elements ask for.
private final class SchemeHandler: NSObject, WKURLSchemeHandler {
    static let scheme = "amgi"
    static let pageURL = URL(string: "amgi://app/index.html")!

    var collectionRoot: URL?
    private let pageRoot = Bundle.module.url(forResource: "Web", withExtension: nil)
    /// Tasks still running; WebKit forbids answering one it has stopped.
    private var running: Set<ObjectIdentifier> = []

    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        let id = ObjectIdentifier(task)
        running.insert(id)
        let request = task.request
        let root = request.url?.host() == "collection" ? collectionRoot : pageRoot
        Task {
            let response = await Task.detached { Self.respond(to: request, root: root) }.value
            guard running.remove(id) != nil else { return }
            switch response {
            case .success(let response, let data):
                task.didReceive(response)
                task.didReceive(data)
                task.didFinish()
            case .failure(let response):
                task.didReceive(response)
                task.didFinish()
            }
        }
    }

    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {
        running.remove(ObjectIdentifier(task))
    }

    private enum Response: Sendable {
        case success(HTTPURLResponse, Data)
        case failure(HTTPURLResponse)
    }

    private nonisolated static func respond(to request: URLRequest, root: URL?) -> Response {
        guard let url = request.url else { return .failure(status(400, for: URL(string: "amgi://app/")!)) }
        let path = url.path(percentEncoded: false).trimmingPrefix("/")
        guard let root, !path.isEmpty else { return .failure(status(404, for: url)) }

        let base = root.standardizedFileURL.resolvingSymlinksInPath()
        let file = base.appending(path: String(path)).standardizedFileURL
        // A folder's path may or may not end in `/`.
        let folder = base.path(percentEncoded: false).trimmingSuffix("/") + "/"
        guard file.path(percentEncoded: false).hasPrefix(folder),
              let handle = try? FileHandle(forReadingFrom: file),
              let size = try? handle.seekToEnd()
        else { return .failure(status(404, for: url)) }
        defer { try? handle.close() }

        var headers = [
            "Content-Type": UTType(filenameExtension: file.pathExtension)?.preferredMIMEType ?? "application/octet-stream",
            "Accept-Ranges": "bytes",
            "Access-Control-Allow-Origin": "*",
            // Files can change on disk while the app runs.
            "Cache-Control": "no-cache",
        ]
        if request.httpMethod == "HEAD" {
            headers["Content-Length"] = "\(size)"
            return .success(HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: headers)!, Data())
        }

        var range: ClosedRange<UInt64> = 0...max(size, 1) - 1
        var code = 200
        if let header = request.value(forHTTPHeaderField: "Range"), let requested = byteRange(header, size: size) {
            range = requested
            code = 206
            headers["Content-Range"] = "bytes \(range.lowerBound)-\(range.upperBound)/\(size)"
        }
        guard size > 0 else {
            headers["Content-Length"] = "0"
            return .success(HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: headers)!, Data())
        }
        do {
            try handle.seek(toOffset: range.lowerBound)
            let data = try handle.read(upToCount: Int(range.upperBound - range.lowerBound + 1)) ?? Data()
            headers["Content-Length"] = "\(data.count)"
            return .success(HTTPURLResponse(url: url, statusCode: code, httpVersion: "HTTP/1.1", headerFields: headers)!, data)
        } catch {
            return .failure(status(500, for: url))
        }
    }

    /// `bytes=a-b`, `bytes=a-` or `bytes=-n`, clamped to the file.
    private nonisolated static func byteRange(_ header: String, size: UInt64) -> ClosedRange<UInt64>? {
        guard size > 0, header.hasPrefix("bytes="), !header.contains(",") else { return nil }
        let parts = header.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
        guard parts.count == 2 else { return nil }
        let last = size - 1
        if parts[0].isEmpty, let suffix = UInt64(parts[1]), suffix > 0 {
            return (size - min(suffix, size))...last
        }
        guard let start = UInt64(parts[0]), start <= last else { return nil }
        let end = parts[1].isEmpty ? last : min(UInt64(parts[1]) ?? last, last)
        return start <= end ? start...end : nil
    }

    private nonisolated static func status(_ code: Int, for url: URL) -> HTTPURLResponse {
        HTTPURLResponse(url: url, statusCode: code, httpVersion: "HTTP/1.1", headerFields: ["Access-Control-Allow-Origin": "*"])!
    }
}

/// Shows the shared web view in the detail column.
struct WebContent: NSViewRepresentable {
    let host: WebHost
    let state: WebState

    func makeNSView(context: Context) -> AmgiWebView {
        host.apply(state)
        return host.webView
    }

    func updateNSView(_ view: AmgiWebView, context: Context) {
        host.apply(state)
    }
}

private extension String {
    func trimmingSuffix(_ suffix: String) -> String {
        hasSuffix(suffix) ? String(dropLast(suffix.count)) : self
    }
}
