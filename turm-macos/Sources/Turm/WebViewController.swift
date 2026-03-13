import AppKit
import WebKit

// MARK: - WebViewController

@MainActor
final class WebViewController: NSViewController, TurmPanel {
    let panelID: String = UUID().uuidString

    private(set) var webView: WKWebView!
    private(set) var currentTitle: String = "Web"
    private var startURL: URL?
    private var started = false

    /// Set by AppDelegate after EventBus is created.
    weak var eventBus: EventBus?

    init(url: URL? = nil) {
        startURL = url
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    override func loadView() {
        let config = WKWebViewConfiguration()
        // Enable Safari Web Inspector (right-click → Inspect Element)
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        let wv = WKWebView(frame: NSRect(x: 0, y: 0, width: 1200, height: 800), configuration: config)
        wv.navigationDelegate = self
        webView = wv
        view = wv
    }

    // MARK: - TurmPanel

    func startIfNeeded() {
        guard !started else { return }
        started = true
        if let url = startURL {
            webView.load(URLRequest(url: url))
        } else {
            loadBlankPage()
        }
    }

    /// Background operations are no-ops for WebView panels.
    func applyBackground(path _: String, tint _: Double) {}
    func clearBackground() {}
    func setTint(_: Double) {}

    // MARK: - Navigation

    func navigate(to urlString: String) {
        let finalString: String = if urlString.hasPrefix("http://") || urlString.hasPrefix("https://") || urlString.hasPrefix("file://") {
            urlString
        } else {
            "https://" + urlString
        }
        guard let url = URL(string: finalString) else { return }
        webView.load(URLRequest(url: url))
    }

    func goBack() {
        if webView.canGoBack { webView.goBack() }
    }

    func goForward() {
        if webView.canGoForward { webView.goForward() }
    }

    func reload() {
        webView.reload()
    }

    func executeJS(_ script: String, completion: @escaping (Any?, Error?) -> Void) {
        webView.evaluateJavaScript(script, completionHandler: completion)
    }

    func getContent(completion: @escaping (String) -> Void) {
        webView.evaluateJavaScript("document.documentElement.outerHTML") { result, _ in
            completion(result as? String ?? "")
        }
    }

    // MARK: - State

    func toggleDevTools() {
        // Enables right-click → "Inspect Element" via Safari Web Inspector.
        // developerExtrasEnabled is already set in loadView(); this re-applies it
        // in case the caller wants to toggle the state at runtime.
        let current = webView.configuration.preferences.value(forKey: "developerExtrasEnabled") as? Bool ?? false
        webView.configuration.preferences.setValue(!current, forKey: "developerExtrasEnabled")
    }

    var currentURL: String {
        webView.url?.absoluteString ?? ""
    }

    var canGoBack: Bool {
        webView.canGoBack
    }

    var canGoForward: Bool {
        webView.canGoForward
    }

    var isLoading: Bool {
        webView.isLoading
    }

    // MARK: - Private

    private func loadBlankPage() {
        let html = """
        <html>
        <body style="background:#1e1e2e;color:#cdd6f4;font-family:system-ui;
                     display:flex;align-items:center;justify-content:center;
                     height:100vh;margin:0">
          <p style="opacity:0.4">Open a URL to get started</p>
        </body>
        </html>
        """
        webView.loadHTMLString(html, baseURL: nil)
    }
}

// MARK: - WKNavigationDelegate

extension WebViewController: WKNavigationDelegate {
    nonisolated func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
        Task { @MainActor in
            let title = webView.title
            let host = webView.url?.host
            let urlStr = webView.url?.absoluteString ?? ""
            self.currentTitle = (title?.isEmpty == false ? title! : host) ?? "Web"
            NotificationCenter.default.post(name: .terminalTitleChanged, object: self)
            let id = self.panelID
            eventBus?.broadcast(event: "webview.loaded", data: ["panel_id": id])
            eventBus?.broadcast(event: "webview.title_changed", data: ["panel_id": id, "title": self.currentTitle])
            eventBus?.broadcast(event: "panel.title_changed", data: ["panel_id": id, "title": self.currentTitle])
        }
    }

    nonisolated func webView(_ webView: WKWebView, didCommit _: WKNavigation!) {
        Task { @MainActor in
            let urlStr = webView.url?.absoluteString ?? ""
            let id = self.panelID
            eventBus?.broadcast(event: "webview.navigated", data: ["panel_id": id, "url": urlStr])
        }
    }
}
