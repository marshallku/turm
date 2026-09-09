import AppKit
import CopadCore
@preconcurrency import WebKit

/// One tab inside a browser pane: a `WKWebView` plus every piece of state that
/// belongs to *that page* rather than to the pane around it.
///
/// Extracted from `WebViewController` when the pane gained a tab strip (work
/// unit B3). All of it — the navigation-lifecycle bookkeeping, the history
/// blob, the scroll offset, the automation mode — was already per-page; it just
/// happened to live on the controller while there was only ever one page.
///
/// **Background tabs are plain detached views.** The spike behind B3 measured
/// `takeSnapshot` on a web view in a hidden window, an off-screen window, a
/// zero-alpha window, and no window at all: every one rendered correctly at the
/// right size. So a background tab needs no window trick — which is why this
/// type holds a `WKWebView` and nothing else, and why an inactive tab is simply
/// not in the view hierarchy.
///
/// What does NOT work off-screen is `requestAnimationFrame`: an occluded web
/// view never gets an animation frame. Anything injected here must assume that
/// (see `scrollReporterJS`).
@MainActor
final class BrowserTab: NSObject {
    /// Stable across restarts, and the key its history blob is filed under.
    let id: String
    private(set) var webView: WKWebView!
    private(set) var title: String = "Web"

    /// Whether this tab currently admits automation. Always `.automation` in
    /// production until work unit B5 — the enforcement half of protected mode
    /// (rebuild the web view, purge the origin's service workers and code
    /// caches) does not exist yet, and a flag that *looks* like protection
    /// while only half of it is built would be worse than no flag at all.
    private(set) var tabMode: TabMode = .automation

    /// Bumped on every main-frame commit AND every mode change. This is what
    /// binds a fill authorization to a DOCUMENT rather than to an origin: an
    /// origin check taken before an async keychain read is stale by the time
    /// the read returns.
    private(set) var documentGeneration: UInt64 = 0

    /// Origin whose executable caches were purged for the CURRENT protected
    /// session, or nil if the purge did not complete.
    ///
    /// A fill is refused unless this matches the page's origin. Two holes it
    /// closes: a purge that TIMED OUT still entered protected mode (so an
    /// agent-planted service worker could survive and intercept the page), and
    /// only the origin present at protect-time was ever purged (so protecting a
    /// blank tab and then navigating somewhere pre-poisoned bypassed it
    /// entirely).
    private(set) var purgedOrigin: String?

    /// Mirrors `copad_core::browser::secrets::TabMode`.
    enum TabMode: String {
        case automation
        case protected
    }

    private weak var owner: WebViewController?
    private var startURL: URL?
    private var started = false
    private var observations: [NSKeyValueObservation] = []

    // MARK: Navigation lifecycle
    //
    // See `BrowserSnapshot.resolveURL` for why a pending destination outranks
    // the live URL. The short version: `WKWebView.url` is nil until WebKit has
    // one and stale after a failed navigation, and the session save is on a
    // timer, so trusting it persisted `""` (or the page before last) and lost
    // the user's page.

    /// Where this tab was asked to go, held until a navigation resolves.
    private var pendingURL: String?
    /// True once `pendingURL`'s navigation was reported as failed. A failed
    /// destination is still worth persisting — it is where the user asked to go
    /// — but must not outrank a later same-document navigation.
    private var pendingFailed = false
    /// Identity of the navigation `pendingURL` belongs to. The delegate
    /// callbacks hop to the main actor, so they run after WebKit called them;
    /// without this, page A committing and the user then navigating to B means
    /// A's queued callback clears B's destination.
    private var pendingNavigationID: ObjectIdentifier?

    // MARK: History + scroll

    private var historyGeneration: UInt64 = 0
    private var restoredHistory: Data?
    private var restoredHistoryDepth: Int?
    private var restoredScrollY: Double?
    private var lastKnownScrollY: Double = 0
    private var lastNotifiedScrollY: Double = 0

    /// Size a tab's web view starts at, and falls back to when the pane has no
    /// laid-out size yet. Roughly a browser window, so a page a background tab
    /// renders is laid out at a plausible viewport rather than something
    /// degenerate.
    static let defaultFrame = NSRect(x: 0, y: 0, width: 1280, height: 800)

    /// Resize while OUT of the view hierarchy. Only meaningful for a background
    /// tab; the active one is driven by autolayout.
    func setBackgroundSize(_ size: NSSize) {
        guard size.width > 1, size.height > 1, webView?.superview == nil else { return }
        webView?.frame = NSRect(origin: .zero, size: size)
    }

    /// Distance the page must scroll before it is worth re-persisting. The
    /// offset is a restore hint, not a measurement, so paying a debounced save
    /// (and a history-blob write) for every few pixels would be pure cost.
    private static let scrollPersistDelta: Double = 50

    // MARK: - Lifecycle

    init(id: String, url: URL?, restored: BrowserTabSnap?, owner: WebViewController) {
        self.id = id
        self.owner = owner
        super.init()

        let restoredURL = restored.flatMap { $0.url.isEmpty ? nil : URL(string: $0.url) } ?? url
        startURL = restoredURL
        pendingURL = restoredURL?.absoluteString
        if let restored, !restored.title.isEmpty { title = restored.title }

        // Read the opaque history blob now, while the snapshot is in hand. A
        // read failure — absent, unreadable, oversize, a symlink planted at its
        // name — is not an error: it simply means this tab restores as a plain
        // URL load.
        if let restored, let generation = restored.historyGeneration {
            historyGeneration = generation
            restoredHistory = BrowserFFI.readHistory(tabID: id, generation: generation)
            restoredHistoryDepth = restored.historyDepth
            restoredScrollY = restored.scrollY
        }

        buildWebView()
    }

    /// Enter or leave protected mode.
    ///
    /// Entering DESTROYS and rebuilds the web view. That is the entire
    /// mechanism: a brand-new `WKWebView` — new content world, new web content
    /// process — cannot be carrying a script the agent installed into the old
    /// document. Sealing the existing document instead would guarantee nothing,
    /// because the agent could have installed a listener before the fill and
    /// read its copy the moment the lock lifted (decision #100).
    ///
    /// Leaving destroys it too. There is no "unseal this document" transition,
    /// because that is exactly the transition that cannot be made safe: a
    /// cancelled submit, an SPA that swallows the event, or a back-navigation
    /// all leave the password recoverable. The session survives — cookies live
    /// in the data store, not in the view — while `sessionStorage` and in-page
    /// JS state do not.
    func setProtected(_ on: Bool, completion: (() -> Void)? = nil) {
        guard (tabMode == .protected) != on else { completion?(); return }
        let url = webView?.url ?? startURL
        tabMode = on ? .protected : .automation
        documentGeneration &+= 1

        // Tear the old view down FIRST, so nothing from the previous document
        // is still running while the purge is in flight.
        observations = []
        webView?.removeFromSuperview()
        webView?.navigationDelegate = nil
        webView = nil

        let rebuild = { [weak self] (purged: String?) in
            guard let self else { return }
            self.purgedOrigin = purged
            self.buildWebView()
            self.started = false
            self.startURL = url
            self.pendingURL = url?.absoluteString
            self.pendingFailed = false
            self.pendingNavigationID = nil
            // Never restore history into a protected document, and never carry
            // one out of it.
            self.restoredHistory = nil
            self.restoredHistoryDepth = nil
            self.restoredScrollY = nil
            self.startIfNeeded()
            self.owner?.tabModeChanged(self)
            completion?()
        }

        guard on else {
            rebuild(nil)
            return
        }
        // A fresh view is not enough on its own: an agent that previously ran
        // `execute_js` on this origin could have poisoned persistent,
        // script-writable state — a Cache API entry or a service worker — which
        // the rebuilt view would then load and execute.
        //
        // The purge is ASYNCHRONOUS, so the load has to WAIT for it. Kicking
        // the purge off and loading immediately (the first version of this)
        // left a live service worker free to serve the protected document
        // before it was removed — the exact attack the purge exists to stop.
        //
        // Cookies and localStorage are deliberately KEPT: purging them signs
        // the user out and breaks every "remember me" flow, and neither is
        // executable.
        purgeExecutableCaches(for: url) { purgedOrigin in rebuild(purgedOrigin) }
    }

    /// Make sure this origin has been purged AND that the document currently
    /// loaded from it post-dates the purge.
    ///
    /// Purging after the fact is not enough. Protect a blank tab, navigate to an
    /// origin with an agent-planted service worker, and that worker has already
    /// served the document — removing its registration afterwards leaves the
    /// page it produced running, with the fill proceeding into it. So a
    /// not-yet-purged origin is purged and the tab is then REBUILT, which
    /// reloads the destination through a fresh view with the caches gone.
    func ensurePurged(for origin: String, then done: @escaping (Bool) -> Void) {
        if let purgedOrigin, purgedOrigin.caseInsensitiveCompare(origin) == .orderedSame {
            done(true)
            return
        }
        purgeExecutableCaches(for: URL(string: origin)) { [weak self] purged in
            guard let self else { done(false); return }
            self.purgedOrigin = purged
            guard purged != nil else { done(false); return }
            self.rebuildForPurge { done(true) }
        }
    }

    /// Rebuild the web view in place, keeping protected mode, and reload the
    /// current URL. Used after a late purge so the document the credential goes
    /// into was produced with the caches already gone.
    private func rebuildForPurge(then done: @escaping () -> Void) {
        let url = webView?.url ?? startURL
        documentGeneration &+= 1
        observations = []
        webView?.removeFromSuperview()
        webView?.navigationDelegate = nil
        webView = nil
        buildWebView()
        started = false
        startURL = url
        pendingURL = url?.absoluteString
        pendingFailed = false
        pendingNavigationID = nil
        restoredHistory = nil
        restoredHistoryDepth = nil
        restoredScrollY = nil
        startIfNeeded()
        owner?.tabModeChanged(self)
        // Give the reload a beat to commit before the caller validates against
        // `documentGeneration` again.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { done() }
    }

    /// Purge the origin's script-writable code caches, reporting the origin
    /// that was actually cleared — or nil if it was not.
    ///
    /// A timeout reports FAILURE rather than proceeding quietly. The earlier
    /// version logged and carried on, which meant an unanswered WebKit callback
    /// silently downgraded protected mode to "a fresh view over whatever the
    /// agent had already planted".
    private func purgeExecutableCaches(for url: URL?, then done: @escaping (String?) -> Void) {
        guard let host = url?.host, let origin = url.map({ u in
            var c = URLComponents()
            c.scheme = u.scheme
            c.host = u.host
            c.port = u.port
            return c.string ?? ""
        }), !origin.isEmpty else {
            // Nothing to purge (a blank tab). Recorded as "no origin purged",
            // so the first navigation to a real origin has to purge before a
            // credential may be used there.
            done(nil)
            return
        }
        let types: Set<String> = [
            WKWebsiteDataTypeServiceWorkerRegistrations,
            WKWebsiteDataTypeFetchCache,
            WKWebsiteDataTypeDiskCache,
            WKWebsiteDataTypeMemoryCache,
            WKWebsiteDataTypeOfflineWebApplicationCache,
        ]
        let store = WKWebsiteDataStore.default()
        let box = SendableBox<(String?) -> Void>(done)
        var finished = false
        let finish = { [box] (purged: String?) in
            guard !finished else { return }
            finished = true
            if purged == nil {
                FileHandle.standardError.write(Data(
                    "[copad-browser] cache purge did not complete; credentials stay refused for this origin\n".utf8,
                ))
            }
            box.value(purged)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { finish(nil) }
        store.fetchDataRecords(ofTypes: types) { records in
            let mine = records.filter { $0.displayName == host || host.hasSuffix($0.displayName) }
            guard !mine.isEmpty else { finish(origin); return }
            store.removeData(ofTypes: types, for: mine) { finish(origin) }
        }
    }

    private func buildWebView() {
        let config = WKWebViewConfiguration()
        // Assert the DEFAULT data store rather than relying on it.
        //
        // `WKWebViewConfiguration()` already defaults to it, which is the only
        // reason logins survive a restart — nothing stated that, so a future
        // edit could have swapped in a non-persistent or identified store and
        // silently signed the user out of every site. It must stay
        // `.default()`: an identified store is a DISTINCT store, so adopting
        // one is a one-way sign-out with no migration path (decision #100).
        config.websiteDataStore = .default()
        config.userContentController.addUserScript(WKUserScript(
            source: Self.scrollReporterJS,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: true,
        ))
        config.userContentController.add(ScrollReporter(owner: self), name: Self.scrollHandlerName)

        // Capture is suppressed AT THE SOURCE in protected mode, not filtered
        // at read time. Refusing the `webview.net` read while the JSONL file
        // kept filling would be no defence at all — the log is deliberately
        // agent-readable, so a `console.log(password)` would simply be read
        // from the file instead. Not installing the script means it has
        // nowhere to go.
        if tabMode != .protected {
            config.userContentController.addUserScript(WKUserScript(
                source: Self.captureJS,
                injectionTime: .atDocumentStart,
                forMainFrameOnly: false,
            ))
            config.userContentController.add(
                CaptureReporter(owner: self),
                name: Self.captureHandlerName,
            )
        }
        // Enable Safari Web Inspector (right-click → Inspect Element)
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")

        // A real starting frame, not `.zero`.
        //
        // A background tab is not in the view hierarchy, so autolayout never
        // gives it a size — and `takeSnapshot` on a zero-sized view produces an
        // image that cannot be encoded at all ("could not encode PNG"). The B3
        // spike missed this because it handed every probe view an explicit
        // frame; the pane resizes background tabs to match the container
        // (`syncBackgroundTabSizes`), and this is the size before the first
        // layout has happened.
        let wv = WKWebView(frame: Self.defaultFrame, configuration: config)
        wv.navigationDelegate = self
        wv.translatesAutoresizingMaskIntoConstraints = false
        webView = wv

        observations = [
            wv.observe(\.canGoBack, options: [.new, .initial]) { [weak self] _, _ in
                Task { @MainActor in self?.owner?.refreshChrome(for: self) }
            },
            wv.observe(\.canGoForward, options: [.new, .initial]) { [weak self] _, _ in
                Task { @MainActor in self?.owner?.refreshChrome(for: self) }
            },
            wv.observe(\.url, options: [.new]) { [weak self] wv, _ in
                Task { @MainActor in
                    guard let self else { return }
                    self.owner?.refreshChrome(for: self)
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

    func startIfNeeded() {
        guard !started, webView != nil else { return }
        started = true
        if restoreHistoryIfPossible() { return }
        if let url = startURL {
            pendingNavigationID = webView.load(URLRequest(url: url)).map(ObjectIdentifier.init)
        } else {
            loadBlankPage()
        }
    }

    // MARK: - Navigation

    /// True while a protected-mode transition is rebuilding the web view.
    /// Every command that would touch it checks this rather than trusting the
    /// implicitly-unwrapped property.
    var isRebuilding: Bool { webView == nil }

    func navigate(to urlString: String) {
        guard let webView else { return }
        let finalString: String = if urlString.hasPrefix("http://")
            || urlString.hasPrefix("https://") || urlString.hasPrefix("file://")
        {
            urlString
        } else {
            "https://" + urlString
        }
        guard let url = URL(string: finalString) else { return }
        // Record the destination BEFORE loading it. Otherwise a blank tab the
        // user typed an unreachable address into has no live URL and no pending
        // one either, and the debounced autosave writes "".
        pendingURL = url.absoluteString
        pendingFailed = false
        pendingNavigationID = webView.load(URLRequest(url: url)).map(ObjectIdentifier.init)
        NotificationCenter.default.post(name: .webviewStateChanged, object: owner)
    }

    func goBack() {
        guard let webView else { return }
        if webView.canGoBack { adoptNavigation(webView.goBack()) }
    }

    func goForward() {
        guard let webView else { return }
        if webView.canGoForward { adoptNavigation(webView.goForward()) }
    }

    func reload() {
        guard let webView else { return }
        adoptNavigation(webView.reload())
    }

    /// Take over the pending state for a navigation this tab did not choose a
    /// URL for — back, forward, reload.
    ///
    /// Without it the identity check would reject the new navigation's
    /// `didCommit` (it still names the previous navigation), so nothing would
    /// ever clear the pending state. And a destination abandoned by pressing
    /// Back is no longer what should be persisted.
    ///
    /// The pending URL is dropped only when there is a usable live URL to fall
    /// back on: reloading a restored tab that has never managed to load is the
    /// case that would otherwise snap the snapshot to `""` and erase it.
    private func adoptNavigation(_ navigation: WKNavigation?) {
        pendingNavigationID = navigation.map(ObjectIdentifier.init)
        let live = webView?.url?.absoluteString
        if let live, !live.isEmpty, live != "about:blank" {
            pendingURL = nil
            pendingFailed = false
        }
    }

    private func isCurrentNavigation(_ navID: ObjectIdentifier?) -> Bool {
        guard let pendingNavigationID else { return navID == nil || pendingURL == nil }
        return navID == pendingNavigationID
    }

    /// WebKit now reports `live`. Retire the pending destination once it has
    /// been reached, and schedule a save either way.
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
        NotificationCenter.default.post(name: .webviewStateChanged, object: owner)
    }

    // MARK: - Snapshot

    /// URL a snapshot would record, before canonicalisation.
    var snapshotSourceURL: String {
        BrowserSnapshot.resolveURL(live: webView?.url?.absoluteString, pending: pendingURL)
    }

    var currentURL: String { webView?.url?.absoluteString ?? "" }
    var canGoBack: Bool { webView?.canGoBack ?? false }
    var canGoForward: Bool { webView?.canGoForward ?? false }
    var isLoading: Bool { webView?.isLoading ?? false }

    func snapshot(policy: String) -> BrowserTabSnap {
        // A protected tab persists NOTHING page-derived.
        //
        // Suppressing capture and refusing reads while still writing the URL,
        // the title and the interaction-state blob into session.json would have
        // been a boundary with the back door left open: an OAuth redirect or a
        // "Reset password for …" title lands in an agent-readable file, and the
        // blob contains the full history besides. The tab keeps its identity so
        // the layout survives; the page does not.
        guard !underProtection else {
            return BrowserTabSnap(id: id, url: "", title: "")
        }
        let decision = BrowserFFI.canonicalize(snapshotSourceURL, policy: policy)
        var snap = BrowserTabSnap(
            id: id,
            url: decision.url,
            // A page title carries the same exposure as a URL. Whether it may
            // be persisted is Rust's call, not a string comparison here.
            title: decision.persistTitle ? title : "",
        )
        if let captured = captureHistory(policy: policy) {
            snap.historyGeneration = captured.generation
            snap.historyDepth = captured.depth
            snap.scrollY = captured.scrollY
        }
        return snap
    }

    /// Persist the opaque back/forward + scroll blob, if the policy keeps it.
    ///
    /// Returns nil — and therefore records NO generation — whenever the blob is
    /// unavailable or the write failed. A generation referencing a blob that was
    /// never written would turn a write failure now into a silent restore
    /// failure later, which is much harder to diagnose than simply not
    /// restoring history this time.
    private func captureHistory(policy: String)
        -> (generation: UInt64, depth: Int, scrollY: Double)?
    {
        guard BrowserFFI.keepsHistory(policy: policy), let webView else { return nil }
        // The blob describes the document WebKit currently holds. When a
        // pending destination is outstanding — a navigation asked for but not
        // committed, typically because it is unreachable — the URL being
        // persisted is that destination, and the blob is the PREVIOUS page. On
        // restore the blob would pass the depth check, the URL load would be
        // skipped, and the user would silently land back on the page they
        // navigated away from. Persist the URL alone in that case.
        guard pendingURL == nil else { return nil }
        // `interactionState` is documented as `Any?`, not `Data` — take it only
        // when it really is `Data`, rather than trusting current practice.
        guard let blob = webView.interactionState as? Data, !blob.isEmpty else { return nil }

        let next = historyGeneration &+ 1
        guard BrowserFFI.writeHistory(tabID: id, generation: next, data: blob) else { return nil }
        historyGeneration = next
        return (next, webView.backForwardList.backList.count, lastKnownScrollY)
    }

    /// Restore the opaque back/forward list instead of re-loading the URL.
    ///
    /// **Assigning `interactionState` is not evidence that it worked.** A blob
    /// from another WebKit version, or a corrupt one, fails SILENTLY — and the
    /// resulting view can still report the right URL, because the fallback
    /// would produce exactly that URL anyway. So success is judged on the one
    /// thing a plain `load(url)` could not fake: the back-list depth.
    @discardableResult
    private func restoreHistoryIfPossible() -> Bool {
        guard let blob = restoredHistory, let expectedDepth = restoredHistoryDepth else {
            return false
        }
        restoredHistory = nil
        webView.interactionState = blob

        // A depth of zero proves nothing on its own: a view that restored
        // NOTHING also has an empty back list, so a corrupt blob for a
        // single-page history would "verify" and leave a blank tab with no URL
        // load to fall back on. For that case the evidence has to be the
        // current item existing at all.
        if expectedDepth == 0, webView.backForwardList.currentItem == nil {
            FileHandle.standardError.write(Data(
                "[copad-browser] history blob restored no current item — loading the URL instead\n".utf8,
            ))
            restoredHistoryDepth = nil
            restoredScrollY = nil
            return false
        }

        let actualDepth = webView.backForwardList.backList.count
        guard actualDepth == expectedDepth else {
            FileHandle.standardError.write(Data(
                "[copad-browser] history blob rejected (depth \(actualDepth) != \(expectedDepth)) — loading the URL instead\n".utf8,
            ))
            restoredHistoryDepth = nil
            restoredScrollY = nil
            return false
        }
        return true
    }

    /// Put the page back where the user left it, once the restored document has
    /// laid out. Deliberately does not verify: a page whose content changed
    /// clamps the offset, and discarding a working back/forward stack over that
    /// would be a bug.
    private func applyRestoredScroll() {
        guard let y = restoredScrollY, y > 0 else { return }
        restoredScrollY = nil
        webView.evaluateJavaScript("window.scrollTo(0, \(y)); 1") { _, _ in }
    }

    // MARK: - Scroll reporting

    static let scrollHandlerName = "copadScroll"

    /// Trailing-edge throttle on a TIMER, deliberately not `requestAnimationFrame`.
    ///
    /// rAF was the obvious choice and it silently does not work: a WKWebView in
    /// an occluded or off-screen window never gets an animation frame, so the
    /// callback never runs and every scroll report is lost. That is not an edge
    /// case — a copad window behind another app is occluded, and a background
    /// tab is off-screen by construction. `setTimeout` is throttled in the
    /// background but never stopped.
    static let scrollReporterJS = """
    (() => {
        let timer = null;
        const report = () => {
            timer = null;
            try {
                window.webkit.messageHandlers.\(scrollHandlerName).postMessage(window.scrollY);
            } catch (e) {}
        };
        const onScroll = () => {
            if (timer !== null) return;
            timer = setTimeout(report, 150);
        };
        window.addEventListener('scroll', onScroll, { passive: true });
        window.addEventListener('load', report);
    })();
    """

    static let captureHandlerName = "copadCapture"

    /// Patch `fetch`, `XMLHttpRequest` and `console.*` at document start.
    ///
    /// **Declared coverage, not implied.** This sees JS-initiated requests and
    /// nothing else — no subresources, no `sendBeacon`, no WebSocket frames, no
    /// service-worker traffic. Navigations are captured natively instead. Every
    /// `webview.net` response carries that coverage string so an empty list is
    /// never read as "no request was made".
    ///
    /// Everything is wrapped so a page that has frozen `console` or replaced
    /// `fetch` with a throwing stub cannot break its own capture — and, more
    /// importantly, cannot break the page by our being here.
    static let captureJS = """
    (() => {
        const post = (payload) => {
            try {
                window.webkit.messageHandlers.\(captureHandlerName).postMessage(payload);
            } catch (e) {}
        };
        const now = () => Math.floor(Date.now() / 1000);

        // --- console ---
        for (const level of ['debug', 'log', 'info', 'warn', 'error']) {
            const original = console[level];
            if (typeof original !== 'function') continue;
            console[level] = function (...args) {
                try {
                    const text = args.map((a) => {
                        if (typeof a === 'string') return a;
                        try { return JSON.stringify(a); } catch (e) { return String(a); }
                    }).join(' ');
                    post({ kind: 'console', ts: now(), level, text });
                } catch (e) {}
                return original.apply(this, args);
            };
        }
        window.addEventListener('error', (e) => {
            post({ kind: 'console', ts: now(), level: 'error',
                   text: String(e.message || e), source: String(e.filename || '') });
        });
        window.addEventListener('unhandledrejection', (e) => {
            post({ kind: 'console', ts: now(), level: 'error',
                   text: 'unhandled rejection: ' + String(e.reason) });
        });

        // --- fetch ---
        const originalFetch = window.fetch;
        if (typeof originalFetch === 'function') {
            window.fetch = function (input, init) {
                const started = Date.now();
                const url = (typeof input === 'string') ? input : (input && input.url) || '';
                const method = (init && init.method) || (input && input.method) || 'GET';
                return originalFetch.apply(this, arguments).then((res) => {
                    post({ kind: 'net', ts: now(), source: 'script', method, url,
                           status: res.status, duration_ms: Date.now() - started });
                    return res;
                }, (err) => {
                    post({ kind: 'net', ts: now(), source: 'script', method, url,
                           duration_ms: Date.now() - started,
                           error: String(err) });
                    throw err;
                });
            };
        }

        // --- XMLHttpRequest ---
        // Still patched even though fetch is everywhere: plenty of libraries
        // use XHR underneath, and a log that quietly missed them would be worse
        // than no log.
        const XHR = window.XMLHttpRequest;
        if (XHR && XHR.prototype) {
            const open = XHR.prototype.open;
            const send = XHR.prototype.send;
            XHR.prototype.open = function (method, url) {
                this.__copad = { method, url, started: 0 };
                return open.apply(this, arguments);
            };
            XHR.prototype.send = function () {
                const meta = this.__copad;
                if (meta) {
                    meta.started = Date.now();
                    this.addEventListener('loadend', () => {
                        post({ kind: 'net', ts: now(), source: 'script',
                               method: meta.method || 'GET', url: meta.url || '',
                               status: this.status || undefined,
                               duration_ms: Date.now() - meta.started });
                    });
                }
                return send.apply(this, arguments);
            };
        }
    })();
    """

    /// Record a natively-observed navigation. The user script cannot see these
    /// — a document load is not a `fetch` — so without this the log would claim
    /// a page made no requests at all when it was simply navigated to.
    func recordNavigation(url: String, status: Int?) {
        // Capture is suppressed at the SOURCE under protection, and this is a
        // source: the navigation record is written natively, so not installing
        // the user script does nothing to stop it. A protected login redirect
        // carries exactly the token the whole mode exists to contain, and the
        // JSONL log is deliberately agent-readable.
        guard !captureSuppressed else { return }
        guard let panelID = owner?.panelID, !url.isEmpty else { return }
        BrowserFFI.appendLog(
            panelID: panelID,
            kind: "net",
            record: [
                "ts": UInt64(Date().timeIntervalSince1970),
                "tab_id": id,
                "source": "navigation",
                "method": "GET",
                "url": url,
                "status": status as Any,
            ].compactMapValues { $0 is NSNull ? nil : $0 },
            captureBodies: owner?.captureBodies ?? false,
        )
    }

    /// Is capture off right now, for this tab OR anything else in its profile?
    ///
    /// Protection freezes the PROFILE — a sibling automation tab is another
    /// window onto the same shared storage and the same logged-in session — and
    /// that has to hold at capture time, not only at RPC-read time. A sibling
    /// kept its capture script (it was built before the protection began), so
    /// without this it would go on writing that session's requests into a file
    /// the agent can simply read, while the RPC path politely refused.
    /// Does protection apply to this tab right now — because of its own mode,
    /// or because anything sharing its session is protected?
    ///
    /// Used for events, persistence and capture alike. Checking only the
    /// emitting tab left every one of those open: an agent could navigate an
    /// automation SIBLING to an authenticated endpoint and receive its URL
    /// through `webview.navigated`, or read it out of session.json under
    /// `url`/`full`, while the RPC gate correctly refused the same read.
    var underProtection: Bool {
        tabMode == .protected || owner?.profileHasProtectedTab == true
    }

    private var captureSuppressed: Bool {
        // Asked of the WINDOW, not just this pane: two browser panes share
        // `WKWebsiteDataStore.default()`, so a second pane's capture script
        // goes on logging the same authenticated session while the first is
        // protected — into a file the agent reads directly, while the RPC gate
        // politely refuses. Protection freezes the profile; the profile is the
        // window.
        tabMode == .protected || owner?.profileHasProtectedTab == true
    }

    /// Bridge for the capture script. Separate object because
    /// `WKUserContentController` retains its handlers.
    private final class CaptureReporter: NSObject, WKScriptMessageHandler {
        private weak var owner: BrowserTab?
        init(owner: BrowserTab) { self.owner = owner }

        func userContentController(
            _: WKUserContentController,
            didReceive message: WKScriptMessage,
        ) {
            guard var body = message.body as? [String: Any],
                  let kind = body.removeValue(forKey: "kind") as? String
            else { return }
            Task { @MainActor [weak owner] in
                guard let owner, let panelID = owner.owner?.panelID else { return }
                // Checked at DELIVERY, not only at injection: this record may
                // have been queued before protection began.
                guard !owner.captureSuppressed else { return }
                body["tab_id"] = owner.id
                BrowserFFI.appendLog(
                    panelID: panelID,
                    kind: kind,
                    record: body,
                    captureBodies: owner.owner?.captureBodies ?? false,
                )
            }
        }
    }

    func noteScroll(_ y: Double) {
        lastKnownScrollY = y
        // Scrolling has to schedule a save of its own. Only layout changes and
        // navigations did, so the offset updated in memory and the file kept
        // whatever it held from the last navigation.
        guard abs(y - lastNotifiedScrollY) >= Self.scrollPersistDelta else { return }
        lastNotifiedScrollY = y
        NotificationCenter.default.post(name: .webviewStateChanged, object: owner)
    }

    /// Separate object rather than conforming the tab itself:
    /// `WKUserContentController` RETAINS its message handlers, so a tab
    /// registering itself would never deallocate.
    private final class ScrollReporter: NSObject, WKScriptMessageHandler {
        private weak var owner: BrowserTab?
        init(owner: BrowserTab) { self.owner = owner }

        func userContentController(
            _: WKUserContentController,
            didReceive message: WKScriptMessage,
        ) {
            guard let y = message.body as? Double else { return }
            Task { @MainActor [weak owner] in owner?.noteScroll(y) }
        }
    }

    // MARK: - Credentials

    /// Fill a credential into this tab's page.
    ///
    /// The secret's entire lifetime is inside this function: keychain → a local
    /// `String` → a JS assignment. It is in no parameter, no return value, no
    /// event, and no log. The caller learns only which selector was filled.
    ///
    /// **The order matters and a first version had it wrong.** That version
    /// probed the DOM, validated, THEN read the keychain, then injected — so
    /// every check was stale by the time the value was written, and the page had
    /// the whole keychain read to swap the input or navigate. Now the keychain
    /// read happens FIRST, and the probe + validation + injection are a single
    /// JS evaluation: the element that is checked is the element that is
    /// written, with nothing in between.
    ///
    /// The preconditions themselves come from `copad-core`, not from Swift —
    /// they decide whether a password reaches a page, and a second
    /// implementation is a second thing that can be subtly wrong.
    func fillCredential(
        _ credential: CredentialRef,
        completion: @escaping (Result<String, RPCError>) -> Void,
    ) {
        // Cheap refusals first, so an unprotected tab never reaches the
        // keychain at all.
        guard tabMode == .protected else {
            completion(.failure(RPCError(
                code: "requires_protected",
                message: "a credential may only be filled into a protected tab",
            )))
            return
        }
        let generationAtStart = documentGeneration
        let origin = BrowserFFI.canonicalize(currentURL, policy: "origin").url

        BrowserKeychain.read(id: credential.id) { [weak self] result in
            guard let self else { return }
            switch result {
            case let .failure(err):
                completion(.failure(Self.keychainError(err)))
            case let .success(stored):
                // Re-validate against the document as it is NOW: the keychain
                // read is the one long-running step, and it is exactly when a
                // page could have navigated.
                guard self.documentGeneration == generationAtStart else {
                    completion(.failure(RPCError(
                        code: "document_changed",
                        message: "the tab navigated during the keychain read — fill aborted",
                    )))
                    return
                }
                // The ORIGIN and SLOT come from the keychain, never from the
                // index: the index is a plain JSON file anything running as the
                // user can rewrite, and pointing an entry at an attacker-chosen
                // origin would otherwise keep its keychain account while making
                // every downstream check agree.
                guard stored.origin.caseInsensitiveCompare(origin) == .orderedSame else {
                    completion(.failure(RPCError(
                        code: "origin_mismatch",
                        message: "this credential is bound to \(stored.origin), not \(origin)",
                    )))
                    return
                }
                // And the purge is a PRECONDITION, re-established for whatever
                // origin the tab is on now — not just the one it was on when
                // protection started.
                self.ensurePurged(for: origin) { purged in
                    guard purged else {
                        completion(.failure(RPCError(
                            code: "purge_incomplete",
                            message: "this origin's service workers and code caches could not be cleared; refusing to fill",
                        )))
                        return
                    }
                    // The generation is re-read HERE, not compared to the value
                    // from before: a late purge deliberately rebuilds the view
                    // and bumps it, so an equality check against the earlier
                    // value would reject exactly the case the purge exists to
                    // make safe. What must still hold is the origin, which the
                    // injection re-asserts inside the page.
                    let generationNow = self.documentGeneration
                    guard self.tabMode == .protected else {
                        completion(.failure(RPCError(
                            code: "requires_protected",
                            message: "the tab left protected mode during the purge",
                        )))
                        return
                    }
                    self.injectAfterValidating(
                        credential, stored: stored, origin: origin,
                        generation: generationNow, completion: completion,
                    )
                }
            }
        }
    }

    /// Probe, validate and write in ONE evaluation.
    ///
    /// The probe result is handed to core's validator, and the injection script
    /// re-asserts the same element identity and type before assigning — so an
    /// element swapped between the probe and the write is caught by the page
    /// itself, not merely by a check that ran a moment earlier. The script also
    /// REPORTS whether it wrote, because a fill that silently did nothing must
    /// not be reported as success.
    private func injectAfterValidating(
        _ credential: CredentialRef,
        stored: BrowserKeychain.Stored,
        origin: String,
        generation: UInt64,
        completion: @escaping (Result<String, RPCError>) -> Void,
    ) {
        let wantsPassword = stored.slot == "password"
        // A nonce stamped on the element during the probe and required by the
        // write. Re-querying a selector is not element identity: a same-origin
        // navigation, or the page swapping in a different password input,
        // yields a NEW element that the same selector matches perfectly. The
        // nonce is what makes the element that was validated the element that
        // is written.
        let nonce = UUID().uuidString
        let probe = """
        (() => {
            const el = \(wantsPassword)
                ? document.querySelector('input[type="password"]')
                : document.querySelector('input[type="text"], input:not([type])');
            if (!el) return JSON.stringify(null);
            const sel = el.id ? '#' + CSS.escape(el.id)
                : (el.name ? 'input[name="' + CSS.escape(el.name) + '"]' : null);
            if (!sel) return JSON.stringify(null);
            el.dataset.copadFillNonce = \(Self.jsString(nonce));
            return JSON.stringify({ selector: sel, is_password_input: el.type === 'password' });
        })()
        """
        let box = SendableBox<(Result<String, RPCError>) -> Void>(completion)
        webView.evaluateJavaScript(probe) { [weak self] result, _ in
            Task { @MainActor in
                guard let self else { return }
                guard let raw = result as? String,
                      let data = raw.data(using: .utf8),
                      let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let selector = obj["selector"] as? String
                else {
                    box.value(.failure(RPCError(
                        code: "no_target",
                        message: "no fillable input element on this page",
                    )))
                    return
                }
                let isPassword = obj["is_password_input"] as? Bool ?? false

                if let problem = BrowserFFI.validateFill(
                    request: [
                        "credential_id": credential.id,
                        "profile": BrowserPaneSnap.defaultProfile,
                        "tab_id": self.id,
                        "origin": stored.origin,
                        "document_generation": generation,
                        "slot": stored.slot,
                    ],
                    // The credential as the TRUSTED store describes it, with the
                    // index contributing only the id and the display name.
                    credential: CredentialRef(
                        id: credential.id,
                        origin: stored.origin,
                        username: credential.username,
                        label: credential.label,
                        slot: stored.slot,
                        createdAt: credential.createdAt,
                        lastUsed: credential.lastUsed,
                    ),
                    live: [
                        "tab_id": self.id,
                        "profile": BrowserPaneSnap.defaultProfile,
                        "mode": self.tabMode.rawValue,
                        "origin": origin,
                        "document_generation": self.documentGeneration,
                    ],
                    target: ["selector": selector, "is_password_input": isPassword],
                ) {
                    box.value(.failure(problem))
                    return
                }

                // The injection re-asserts the ORIGIN and the element inside
                // the page itself. Re-querying a selector is not enough: a
                // navigation between validation and execution lands on a
                // different document that may well have its own matching
                // password field, and the value would be delivered there. The
                // page can only answer for the document it is actually in.
                let js = """
                (() => {
                    if (location.origin !== \(Self.jsString(stored.origin))) return 'wrong-origin';
                    // By NONCE, not by selector: the element that was validated
                    // is the element that gets written, even if the page has
                    // since grown another one the selector would also match.
                    const el = document.querySelector('[data-copad-fill-nonce=' +
                        JSON.stringify(\(Self.jsString(nonce))) + ']');
                    if (!el) return 'gone';
                    delete el.dataset.copadFillNonce;
                    if (\(wantsPassword) && el.type !== 'password') return 'retyped';
                    el.focus({ preventScroll: true });
                    el.value = \(Self.jsString(stored.secret));
                    el.dispatchEvent(new Event('input', { bubbles: true }));
                    el.dispatchEvent(new Event('change', { bubbles: true }));
                    return 'ok';
                })()
                """
                guard let webView = self.webView, self.documentGeneration == generation else {
                    box.value(.failure(RPCError(
                        code: "document_changed",
                        message: "the tab changed between validation and injection",
                    )))
                    return
                }
                webView.evaluateJavaScript(js) { outcome, error in
                    Task { @MainActor in
                        // A fill that did nothing must not report success.
                        switch outcome as? String {
                        case "ok":
                            box.value(.success(selector))
                        case "wrong-origin":
                            box.value(.failure(RPCError(
                                code: "origin_mismatch",
                                message: "the document navigated to another origin before the write",
                            )))
                        case "gone":
                            box.value(.failure(RPCError(
                                code: "document_changed",
                                message: "the target element disappeared before the value was written",
                            )))
                        case "retyped":
                            box.value(.failure(RPCError(
                                code: "not_a_password_input",
                                message: "the target element stopped being a password input",
                            )))
                        default:
                            // A FIXED message. `el.value = …` runs a setter the
                            // page can define, and a setter that throws
                            // `new Error(value)` would put the password into
                            // `localizedDescription` and straight out through
                            // the RPC response — a leak by way of the error
                            // path, which nothing else was checking.
                            box.value(.failure(RPCError(
                                code: "fill_failed",
                                message: "the page did not accept the value",
                            )))
                        }
                    }
                }
            }
        }
    }

    static func keychainError(_ failure: BrowserKeychain.Failure) -> RPCError {
        switch failure {
        case .notFound:
            RPCError(code: "not_found", message: "no secret stored for that credential")
        case let .unavailable(detail):
            RPCError(
                code: "secret_backend_unavailable",
                message: "platform keychain unavailable, password features are disabled: \(detail)",
            )
        }
    }

    /// JSON-encode a string so it can be embedded literally in JS source. The
    /// secret goes through this, so a password containing a quote or a
    /// backslash cannot break out into executable JS.
    static func jsString(_ s: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: s, options: [.fragmentsAllowed]),
              let str = String(data: data, encoding: .utf8)
        else { return "\"\"" }
        return str
    }

    // MARK: - Misc

    func executeJS(_ script: String, completion: @escaping (Any?, Error?) -> Void) {
        guard let webView else {
            completion(nil, NSError(
                domain: "copad.browser", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "the tab is rebuilding"],
            ))
            return
        }
        // WKWebView's completionHandler is @Sendable in the Swift 6 SDK; the
        // socket command chain that ultimately owns `completion` is not
        // @Sendable-typed yet. Box the callback so the @Sendable closure literal
        // only captures a Sendable wrapper. WebKit invokes the callback on the
        // main thread, so the unchecked-sendable bridge is sound.
        let box = SendableBox(completion)
        webView.evaluateJavaScript(script) { result, error in
            box.value(result, error)
        }
    }

    /// Re-apply `developerExtrasEnabled` on THIS tab's configuration, so
    /// right-click → "Inspect Element" is available for the tab the caller
    /// named.
    func toggleDevTools() {
        guard let webView else { return }
        let current = webView.configuration.preferences
            .value(forKey: "developerExtrasEnabled") as? Bool ?? false
        webView.configuration.preferences.setValue(!current, forKey: "developerExtrasEnabled")
    }

    func getContent(completion: @escaping (String) -> Void) {
        guard let webView else { completion(""); return }
        let box = SendableBox(completion)
        webView.evaluateJavaScript("document.documentElement.outerHTML") { result, _ in
            box.value(result as? String ?? "")
        }
    }

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

extension BrowserTab: WKNavigationDelegate {
    nonisolated func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
        Task { @MainActor in
            self.applyRestoredScroll()
            let pageTitle = webView.title
            let host = webView.url?.host
            self.title = (pageTitle?.isEmpty == false ? pageTitle! : host) ?? "Web"
            self.owner?.tabDidFinishLoading(self)
        }
    }

    /// Any navigation starting — including one the PAGE began by itself: a link
    /// click, a `<meta refresh>`, a JS redirect.
    ///
    /// Those never pass through `navigate`/`goBack`/`reload`, so without
    /// adopting them here the tab would still be tracking an older navigation
    /// and would reject the new one's `didCommit` — leaving the pending
    /// destination stuck forever and autosaving a page the user left long ago.
    ///
    /// The equality guard keeps this from eating our OWN navigations:
    /// `navigate` assigns `pendingNavigationID` synchronously from `load()`'s
    /// return value, before this callback can run.
    nonisolated func webView(
        _ webView: WKWebView,
        didStartProvisionalNavigation navigation: WKNavigation!,
    ) {
        let navID = navigation.map(ObjectIdentifier.init)
        Task { @MainActor in
            guard self.pendingNavigationID != navID else { return }
            self.pendingNavigationID = navID
            let live = webView.url?.absoluteString
            if let live, !live.isEmpty, live != "about:blank" {
                self.pendingURL = nil
                self.pendingFailed = false
            }
        }
    }

    /// A load that never got off the ground — an unreachable host, DNS failure,
    /// a refused connection. WebKit reverts `url` to the previously committed
    /// page here, so without re-arming the pending destination the tab would
    /// autosave the OLD page and silently discard where the user asked to go.
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
               // one's failure.
               self.pendingURL == nil || self.pendingURL == failing
            {
                self.pendingURL = failing
                self.pendingFailed = true
            }
            NotificationCenter.default.post(name: .webviewStateChanged, object: self.owner)
        }
    }

    nonisolated func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        let navID = navigation.map(ObjectIdentifier.init)
        Task { @MainActor in
            guard self.isCurrentNavigation(navID) else { return }
            let urlStr = webView.url?.absoluteString ?? ""
            // A committed document supersedes the pending destination whether
            // or not the two strings match (a redirect lands elsewhere), which
            // is why this does not go through `noteURLChanged`'s equality check.
            if !urlStr.isEmpty, urlStr != "about:blank" {
                self.pendingURL = nil
                self.pendingFailed = false
                self.pendingNavigationID = nil
                self.documentGeneration &+= 1
            }
            NotificationCenter.default.post(name: .webviewStateChanged, object: self.owner)
            self.recordNavigation(url: urlStr, status: nil)
            self.owner?.tabDidCommit(self, url: urlStr)
        }
    }
}
