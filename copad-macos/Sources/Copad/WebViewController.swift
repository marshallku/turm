import AppKit
import CopadCore
@preconcurrency import WebKit

// MARK: - WebViewController

@MainActor
final class WebViewController: NSViewController, CopadPanel {
    let panelID: String

    private(set) var webView: WKWebView!

    /// Focus target for `panel.focusTarget` — the WKWebView is the
    /// actual keyboard receiver; the controller's `view` is a layout
    /// container holding the URL bar + back/forward/reload + webView.
    var focusTarget: NSView {
        webView ?? view
    }

    private(set) var currentTitle: String = "Web"
    private var startURL: URL?
    private var started = false

    /// Stable identity for this pane's (currently sole) browser tab. Seeded
    /// from a restored snapshot so a tab keeps its identity — and therefore its
    /// future history blob — across restarts, rather than being reissued each
    /// launch. Work unit B3 turns this into a list.
    private(set) var tabID: String

    /// The URL this pane was restored with, held until a main-frame navigation
    /// supplies an authoritative replacement.
    ///
    /// Autosave runs on a timer, so it fires while a restored pane is still
    /// loading — and `webView.url` is nil until WebKit has one. Without this,
    /// a snapshot taken in that window would persist `""` and **erase the pane
    /// before it ever finished opening**. It also means a pane whose first load
    /// FAILED survives to be retried next launch.
    private var pendingURL: String?

    /// True once `pendingURL`'s navigation has been reported as failed.
    ///
    /// A failed destination is still worth persisting — it is where the user
    /// asked to go, and a restart should retry it. But it must not outrank a
    /// LATER same-document navigation: after a failed hop to B, an SPA route
    /// change or fragment on the page still showing A means the user is on A,
    /// and B is history. Without this flag nothing could ever clear B (the URL
    /// observer only clears on equality, and no commit is coming).
    private var pendingFailed = false

    /// Identity of the navigation `pendingURL` belongs to.
    ///
    /// The delegate callbacks are `nonisolated` and hop to the main actor, so
    /// they run some time AFTER WebKit called them. Without an identity check,
    /// page A committing and then the user navigating to B means A's queued
    /// callback clears B's pending destination — and autosave persists A while
    /// B is still loading, which is the exact bug `pendingURL` exists to
    /// prevent, reintroduced through the back door. `ObjectIdentifier` rather
    /// than the `WKNavigation` itself because only the former is `Sendable`.
    private var pendingNavigationID: ObjectIdentifier?

    /// Does a delegate callback belong to the navigation we are tracking?
    /// A callback we cannot identify is accepted only when we are not tracking
    /// one either, so an unidentifiable straggler can never clobber a live
    /// request.
    private func isCurrentNavigation(_ id: ObjectIdentifier?) -> Bool {
        guard let pendingNavigationID else { return id == nil || pendingURL == nil }
        return id == pendingNavigationID
    }

    /// Whether this tab currently admits automation. Always `.automation` in
    /// production until work unit B5 — the enforcement half of protected mode
    /// (rebuild the web view, purge the origin's service workers and code
    /// caches) does not exist yet, and a flag that *looks* like protection
    /// while only half of it is built would be worse than no flag at all.
    /// `webview.tab.protect` answers `unsupported_capability` until then.
    private(set) var tabMode: TabMode = .automation

    /// Mirrors `copad_core::browser::secrets::TabMode`.
    enum TabMode: String {
        case automation
        case protected
    }

    private var urlField: NSTextField!
    private var backButton: NSButton!
    private var forwardButton: NSButton!
    private var reloadButton: NSButton!
    private var observations: [NSKeyValueObservation] = []

    /// Set by AppDelegate after EventBus is created.
    weak var eventBus: EventBus?

    init(url: URL? = nil, restoreID: String? = nil, pane: BrowserPaneSnap? = nil) {
        self.panelID = restoreID ?? UUID().uuidString

        // Identity and URL both come from the ACTIVE tab, not the first one.
        // Taking `tabs.first` would hand tab A's identity to tab B's page for
        // any pane where the user had switched away from the first tab, and the
        // next autosave would persist that wrong pairing — quietly transplanting
        // one tab's history blob onto another's page once B2 lands.
        //
        // `active` is safe to index because a restored pane reaches here only
        // through `BrowserFFI.normalize`, which clamps it into range (and drops
        // every id that fails the Rust charset rule — so there is nothing left
        // for Swift to validate, and validating again would be the duplicated
        // security rule this whole layer exists to remove).
        let activeTab = pane.flatMap { p -> BrowserTabSnap? in
            p.tabs.indices.contains(p.active) ? p.tabs[p.active] : p.tabs.first
        }
        tabID = activeTab?.id ?? BrowserSnapshot.freshTabID()

        // The legacy `url` field is the fallback for a pane written before the
        // Workbench, or one whose tab list did not survive normalization.
        let restored = activeTab.flatMap { $0.url.isEmpty ? nil : URL(string: $0.url) } ?? url
        startURL = restored
        pendingURL = restored?.absoluteString
        pendingFailed = false
        super.init(nibName: nil, bundle: nil)
    }

    /// Regenerate this pane's persistable browser state.
    ///
    /// Regenerated rather than carried: `PaneManager` rebuilds `PaneContent`
    /// from the live panel on every autosave, so a stashed snapshot would go
    /// stale the moment the user navigated. `webView` is read optionally so a
    /// snapshot taken before `loadView` yields the pending URL instead of
    /// trapping on the implicitly-unwrapped property.
    func snapshotPane(policy: String) -> BrowserPaneSnap {
        let resolved = BrowserSnapshot.resolveURL(
            live: webView?.url?.absoluteString,
            pending: pendingURL,
        )
        let decision = BrowserFFI.canonicalize(resolved, policy: policy)
        return BrowserSnapshot.pane(
            tabID: tabID,
            url: decision.url,
            // A page title carries the same exposure as a URL. Whether it may be
            // persisted is Rust's call, not a string comparison here.
            title: decision.persistTitle ? currentTitle : "",
        )
    }

    /// WebKit now reports `live`. Retire the pending destination once it has
    /// been reached, and schedule a save either way.
    ///
    /// Clearing on equality is safe precisely because it is a no-op for the
    /// snapshot at that instant — `resolveURL` would return the same string
    /// from either source. What it buys is that a LATER same-document
    /// navigation is not shadowed by a stale pending value that nothing else
    /// would ever clear (fragment and SPA route changes never commit).
    ///
    /// The failed-load half of the contract is restored by
    /// `didFailProvisionalNavigation`, which re-arms pending when WebKit
    /// reverts to the previously committed URL.
    private func noteURLChanged(_ live: String?) {
        if let live, !live.isEmpty, live != "about:blank" {
            // Reached, or superseded: a live URL arriving after a FAILED
            // destination means the document moved on without it.
            if live == pendingURL || pendingFailed {
                pendingURL = nil
                pendingFailed = false
                pendingNavigationID = nil
            }
        }
        NotificationCenter.default.post(name: .webviewURLChanged, object: self)
    }

    /// The URL a snapshot would record, before canonicalisation. Used by
    /// `PaneManager` so the pane's own `url` field and its tab's `url` cannot
    /// disagree about which page this is.
    var snapshotSourceURL: String {
        BrowserSnapshot.resolveURL(live: webView?.url?.absoluteString, pending: pendingURL)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    override func loadView() {
        let config = WKWebViewConfiguration()
        // Enable Safari Web Inspector (right-click → Inspect Element)
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        let wv = WKWebView(frame: .zero, configuration: config)
        wv.navigationDelegate = self
        wv.translatesAutoresizingMaskIntoConstraints = false
        webView = wv

        let back = makeToolbarButton(symbol: "chevron.left", tooltip: "Back", action: #selector(backTapped))
        let forward = makeToolbarButton(symbol: "chevron.right", tooltip: "Forward", action: #selector(forwardTapped))
        let reload = makeToolbarButton(symbol: "arrow.clockwise", tooltip: "Reload", action: #selector(reloadTapped))
        let devtools = makeToolbarButton(symbol: "wrench.and.screwdriver", tooltip: "DevTools", action: #selector(devtoolsTapped))
        back.isEnabled = false
        forward.isEnabled = false
        backButton = back
        forwardButton = forward
        reloadButton = reload

        let field = NSTextField()
        field.placeholderString = "Enter URL or search…"
        field.bezelStyle = .roundedBezel
        field.font = .systemFont(ofSize: 12)
        field.usesSingleLineMode = true
        field.lineBreakMode = .byTruncatingTail
        field.cell?.sendsActionOnEndEditing = false
        field.target = self
        field.action = #selector(urlFieldSubmit(_:))
        field.translatesAutoresizingMaskIntoConstraints = false
        if let url = startURL { field.stringValue = url.absoluteString }
        urlField = field

        let toolbar = NSStackView(views: [back, forward, reload, field, devtools])
        toolbar.orientation = .horizontal
        toolbar.spacing = 4
        toolbar.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        toolbar.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.addSubview(toolbar)
        container.addSubview(wv)

        NSLayoutConstraint.activate([
            toolbar.topAnchor.constraint(equalTo: container.topAnchor),
            toolbar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            toolbar.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            wv.topAnchor.constraint(equalTo: toolbar.bottomAnchor),
            wv.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            wv.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            wv.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        view = container

        observations = [
            wv.observe(\.canGoBack, options: [.new, .initial]) { [weak self] wv, _ in
                Task { @MainActor in self?.backButton?.isEnabled = wv.canGoBack }
            },
            wv.observe(\.canGoForward, options: [.new, .initial]) { [weak self] wv, _ in
                Task { @MainActor in self?.forwardButton?.isEnabled = wv.canGoForward }
            },
            wv.observe(\.url, options: [.new]) { [weak self] wv, _ in
                Task { @MainActor in
                    guard let self else { return }
                    self.syncURLField(wv.url)
                    // `history.pushState` / `replaceState` / a fragment change
                    // never fire `didCommit`, and WebKit's same-document
                    // delegate callback is SPI, not part of the public
                    // `WKNavigationDelegate` — so this KVO is the ONLY
                    // supported signal for them. Under `restore = "url"` those
                    // are exactly the route changes a single-page app makes.
                    self.noteURLChanged(wv.url?.absoluteString)
                }
            },
        ]
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        if startURL == nil, urlField?.stringValue.isEmpty == true {
            view.window?.makeFirstResponder(urlField)
        }
    }

    private func makeToolbarButton(symbol: String, tooltip: String, action: Selector) -> NSButton {
        let btn = NSButton()
        btn.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        btn.bezelStyle = .regularSquare
        btn.isBordered = false
        btn.imageScaling = .scaleProportionallyDown
        btn.toolTip = tooltip
        btn.target = self
        btn.action = action
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.widthAnchor.constraint(equalToConstant: 24).isActive = true
        btn.heightAnchor.constraint(equalToConstant: 24).isActive = true
        return btn
    }

    private func syncURLField(_ url: URL?) {
        guard let urlField else { return }
        let s = url?.absoluteString ?? ""
        guard !s.isEmpty, s != "about:blank" else { return }
        // Don't clobber what the user is currently typing.
        if view.window?.firstResponder === urlField.currentEditor() { return }
        urlField.stringValue = s
    }

    @objc private func backTapped() {
        goBack()
    }

    @objc private func forwardTapped() {
        goForward()
    }

    @objc private func reloadTapped() {
        reload()
    }

    @objc private func devtoolsTapped() {
        toggleDevTools()
    }

    @objc private func urlFieldSubmit(_ sender: NSTextField) {
        let text = sender.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        navigate(to: text)
        view.window?.makeFirstResponder(webView)
    }

    // MARK: - CopadPanel

    func startIfNeeded() {
        guard !started else { return }
        started = true
        if let url = startURL {
            pendingNavigationID = webView.load(URLRequest(url: url)).map(ObjectIdentifier.init)
        } else {
            loadBlankPage()
        }
    }

    /// Background operations are no-ops for WebView panels.
    func applyBackground(path _: String, tint _: Double, opacity _: Double) {}
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
        // Record the destination BEFORE loading it. Otherwise a blank pane the
        // user typed an unreachable address into has no live URL and no pending
        // one either, and the debounced autosave writes "" — the same erasure
        // the restore path was fixed for, reached by a different route.
        pendingURL = url.absoluteString
        pendingFailed = false
        pendingNavigationID = webView.load(URLRequest(url: url)).map(ObjectIdentifier.init)

        // Persist the destination now rather than when the page happens to
        // load: an unreachable address never commits, and nothing else in the
        // app would schedule a save for it.
        NotificationCenter.default.post(name: .webviewURLChanged, object: self)
    }

    func goBack() {
        if webView.canGoBack { adoptNavigation(webView.goBack()) }
    }

    func goForward() {
        if webView.canGoForward { adoptNavigation(webView.goForward()) }
    }

    func reload() {
        adoptNavigation(webView.reload())
    }

    /// Take over the pending state for a navigation this controller did not
    /// choose a URL for — back, forward, reload.
    ///
    /// Two things go wrong without it. The identity check would reject the new
    /// navigation's `didCommit` (it still names the *previous* navigation), so
    /// nothing would ever clear the pending state. And a destination abandoned
    /// by the user pressing Back is no longer what should be persisted — start
    /// loading unreachable B from A, hit reload, and autosave would keep
    /// writing B while A is on screen.
    ///
    /// The pending URL is dropped only when there is a usable live URL to fall
    /// back on. Reloading a restored pane that has never managed to load is the
    /// case that would otherwise snap the snapshot to `""` and erase it.
    private func adoptNavigation(_ navigation: WKNavigation?) {
        pendingNavigationID = navigation.map(ObjectIdentifier.init)
        let live = webView?.url?.absoluteString
        if let live, !live.isEmpty, live != "about:blank" {
            pendingURL = nil
            pendingFailed = false
        }
    }

    func executeJS(_ script: String, completion: @escaping (Any?, Error?) -> Void) {
        // WKWebView's completionHandler is @Sendable in the Swift 6 SDK; the socket
        // command chain that ultimately owns `completion` is not @Sendable-typed yet.
        // Box the callback so the @Sendable closure literal we pass in only captures
        // a Sendable wrapper. WebKit invokes the callback on the main thread, so the
        // unchecked-sendable bridge is sound.
        let box = SendableBox(completion)
        webView.evaluateJavaScript(script) { result, error in
            box.value(result, error)
        }
    }

    func getContent(completion: @escaping (String) -> Void) {
        let box = SendableBox(completion)
        webView.evaluateJavaScript("document.documentElement.outerHTML") { result, _ in
            box.value(result as? String ?? "")
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
            self.currentTitle = (title?.isEmpty == false ? title! : host) ?? "Web"
            NotificationCenter.default.post(name: .terminalTitleChanged, object: self)
            let id = self.panelID
            eventBus?.broadcast(event: "webview.loaded", data: ["panel_id": id])
            eventBus?.broadcast(event: "webview.title_changed", data: ["panel_id": id, "title": self.currentTitle])
            eventBus?.broadcast(event: "panel.title_changed", data: ["panel_id": id, "title": self.currentTitle])
        }
    }

    /// Any navigation starting — including one the PAGE began by itself: a link
    /// click, a `<meta refresh>`, a JS redirect.
    ///
    /// Those never pass through `navigate`/`goBack`/`reload`, so without
    /// adopting them here the controller would still be tracking an older
    /// navigation and would reject the new one's `didCommit` — leaving the
    /// pending destination stuck forever and autosaving a page the user left
    /// long ago.
    ///
    /// The equality guard is what keeps this from eating our OWN navigations:
    /// `navigate` assigns `pendingNavigationID` synchronously from `load()`'s
    /// return value, before this callback can run, so a navigation we initiated
    /// matches and its freshly-set pending URL survives.
    nonisolated func webView(
        _ webView: WKWebView,
        didStartProvisionalNavigation navigation: WKNavigation!,
    ) {
        let navID = navigation.map(ObjectIdentifier.init)
        Task { @MainActor in
            guard self.pendingNavigationID != navID else { return }
            self.pendingNavigationID = navID
            // Same rule as `adoptNavigation`: an abandoned destination is only
            // dropped when there is a live URL to fall back on, or the snapshot
            // would go empty and erase the pane.
            let live = webView.url?.absoluteString
            if let live, !live.isEmpty, live != "about:blank" {
                self.pendingURL = nil
                self.pendingFailed = false
            }
        }
    }

    /// A load that never got off the ground — an unreachable host, DNS
    /// failure, a refused connection.
    ///
    /// WebKit reverts `url` to the previously committed page here, so without
    /// re-arming the pending destination the pane would autosave the OLD page
    /// and silently discard where the user asked to go. That is the same
    /// erasure `pendingURL` exists to prevent, reached from the other end.
    nonisolated func webView(
        _: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error,
    ) {
        let navID = navigation.map(ObjectIdentifier.init)
        let ns = error as NSError
        // A cancellation is not a failed load — it is THIS navigation being
        // superseded by a newer one. Re-arming from it would resurrect the
        // abandoned destination.
        let cancelled = ns.domain == NSURLErrorDomain && ns.code == NSURLErrorCancelled
        let failing = ns.userInfo[NSURLErrorFailingURLStringErrorKey] as? String
        Task { @MainActor in
            guard self.isCurrentNavigation(navID) else { return }
            if !cancelled, let failing, !failing.isEmpty, failing != "about:blank",
               // Never overwrite a NEWER pending destination with an older
               // one's failure: start loading A, navigate to B, then A's
               // callback arrives — B is what the user asked for.
               self.pendingURL == nil || self.pendingURL == failing
            {
                self.pendingURL = failing
                self.pendingFailed = true
            }
            NotificationCenter.default.post(name: .webviewURLChanged, object: self)
        }
    }

    nonisolated func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        let navID = navigation.map(ObjectIdentifier.init)
        Task { @MainActor in
            guard self.isCurrentNavigation(navID) else { return }
            let urlStr = webView.url?.absoluteString ?? ""
            // The live document is now authoritative; stop preferring the
            // restore URL. A committed document supersedes the pending
            // destination whether or not the two strings match (a redirect
            // lands elsewhere), which is why this does not go through
            // `noteURLChanged`'s equality check.
            if !urlStr.isEmpty, urlStr != "about:blank" {
                self.pendingURL = nil
                self.pendingFailed = false
                self.pendingNavigationID = nil
            }
            NotificationCenter.default.post(name: .webviewURLChanged, object: self)
            let id = self.panelID
            eventBus?.broadcast(event: "webview.navigated", data: ["panel_id": id, "url": urlStr])
        }
    }
}

// SendableBox lives in Sources/Copad/SendableBox.swift now — same need shows
// up in PluginSupervisor (PR 3), so the type was hoisted out of this file.
