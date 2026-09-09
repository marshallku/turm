import AppKit
import CopadCore
@preconcurrency import WebKit

// MARK: - WebViewController

/// A browser pane: a tab strip, a toolbar, and one visible `BrowserTab`.
///
/// Everything that belongs to a *page* lives on `BrowserTab`; this owns the
/// chrome and the tab list. Inactive tabs stay alive but out of the view
/// hierarchy — the B3 spike measured `takeSnapshot` on a web view in no window
/// at all and it rendered correctly, so a background tab needs no hidden-window
/// trick to remain screenshottable.
@MainActor
final class WebViewController: NSViewController, CopadPanel {
    let panelID: String

    private(set) var tabs: [BrowserTab] = []
    private(set) var activeIndex = 0

    var activeTab: BrowserTab { tabs[activeIndex] }

    /// The active tab's web view. Kept as a property name so the socket layer,
    /// which reaches for `webVC.webView.takeSnapshot`, did not have to change
    /// when the pane gained tabs.
    var webView: WKWebView! { tabs.indices.contains(activeIndex) ? tabs[activeIndex].webView : nil }

    /// Focus target for `panel.focusTarget` — the WKWebView is the actual
    /// keyboard receiver; the controller's `view` is a layout container holding
    /// the tab strip + URL bar + navigation buttons + webView.
    var focusTarget: NSView { webView ?? view }

    private(set) var currentTitle: String = "Web"

    private var urlField: NSTextField!
    private var backButton: NSButton!
    private var forwardButton: NSButton!
    private var reloadButton: NSButton!
    private var tabStrip: NSStackView!
    private var tabStripScroll: NSScrollView!
    private var webContainer: NSView!
    private var webConstraints: [NSLayoutConstraint] = []

    /// Set by AppDelegate after EventBus is created.
    weak var eventBus: EventBus?

    /// `[browser] capture_bodies`. Off by default: bodies are the likeliest
    /// place for a secret to end up in a file the agent can read.
    var captureBodies = false

    /// Restore state held until `startIfNeeded`, so the pane's tabs are built
    /// once rather than half-built in `init` and half in `loadView`.
    private var pendingRestore: BrowserPaneSnap?
    private var pendingInitialURL: URL?
    private var started = false

    // MARK: - Lifecycle

    init(url: URL? = nil, restoreID: String? = nil, pane: BrowserPaneSnap? = nil) {
        panelID = restoreID ?? UUID().uuidString
        pendingInitialURL = url
        // A restored pane reaches here only through `BrowserFFI.normalize`,
        // which has already clamped `active` into range and dropped every id
        // that fails the Rust charset rule — so there is nothing left for Swift
        // to validate, and validating again would be the duplicated security
        // rule this layer exists to remove.
        pendingRestore = (pane?.tabs.isEmpty == false) ? pane : nil
        super.init(nibName: nil, bundle: nil)
        WebViewController.livePanes.add(self)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    override func loadView() {
        let back = makeToolbarButton(symbol: "chevron.left", tooltip: "Back", action: #selector(backTapped))
        let forward = makeToolbarButton(symbol: "chevron.right", tooltip: "Forward", action: #selector(forwardTapped))
        let reload = makeToolbarButton(symbol: "arrow.clockwise", tooltip: "Reload", action: #selector(reloadTapped))
        let devtools = makeToolbarButton(symbol: "wrench.and.screwdriver", tooltip: "DevTools", action: #selector(devtoolsTapped))
        let newTab = makeToolbarButton(symbol: "plus", tooltip: "New tab", action: #selector(newTabTapped))
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
        urlField = field

        let toolbar = NSStackView(views: [back, forward, reload, field, newTab, devtools])
        toolbar.orientation = .horizontal
        toolbar.spacing = 4
        toolbar.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        toolbar.translatesAutoresizingMaskIntoConstraints = false

        // Tab strip. Horizontally scrollable rather than shrinking chips to
        // nothing: a pane can be narrow, and an unreadable 12-pixel chip is
        // worse than one that scrolls.
        let strip = NSStackView()
        strip.orientation = .horizontal
        strip.spacing = 4
        strip.edgeInsets = NSEdgeInsets(top: 0, left: 6, bottom: 0, right: 6)
        strip.translatesAutoresizingMaskIntoConstraints = false
        tabStrip = strip

        let scroll = NSScrollView()
        scroll.hasHorizontalScroller = false
        scroll.hasVerticalScroller = false
        scroll.drawsBackground = false
        scroll.documentView = strip
        scroll.translatesAutoresizingMaskIntoConstraints = false
        tabStripScroll = scroll

        let container = NSView()
        let host = NSView()
        host.translatesAutoresizingMaskIntoConstraints = false
        webContainer = host

        container.addSubview(scroll)
        container.addSubview(toolbar)
        container.addSubview(host)

        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: container.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 28),
            strip.heightAnchor.constraint(equalTo: scroll.heightAnchor),

            toolbar.topAnchor.constraint(equalTo: scroll.bottomAnchor),
            toolbar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            toolbar.trailingAnchor.constraint(equalTo: container.trailingAnchor),

            host.topAnchor.constraint(equalTo: toolbar.bottomAnchor),
            host.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            host.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            host.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        view = container
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        // Only when the pane opened with nothing to show. Never on a
        // programmatic creation, which must not steal the keyboard from
        // whatever the user is typing into.
        if tabs.count == 1, activeTab.snapshotSourceURL.isEmpty,
           urlField?.stringValue.isEmpty == true
        {
            view.window?.makeFirstResponder(urlField)
        }
    }

    // MARK: - CopadPanel

    func startIfNeeded() {
        guard !started else { return }
        started = true

        if let restore = pendingRestore {
            for snap in restore.tabs {
                tabs.append(BrowserTab(id: snap.id, url: nil, restored: snap, owner: self))
            }
            activeIndex = min(restore.active, tabs.count - 1)
        } else {
            tabs.append(BrowserTab(
                id: BrowserSnapshot.freshTabID(),
                url: pendingInitialURL,
                restored: nil,
                owner: self,
            ))
            activeIndex = 0
        }
        pendingRestore = nil
        pendingInitialURL = nil

        // Every tab starts loading, not just the visible one: a restored
        // background tab that only loaded when first selected would lose its
        // history blob on the next save (nothing to capture from an empty view)
        // and come back as a bare URL.
        for tab in tabs { tab.startIfNeeded() }
        showActiveTab()
        refreshTabStrip()
    }

    /// Background operations are no-ops for WebView panels.
    func applyBackground(path _: String, tint _: Double, opacity _: Double) {}
    func clearBackground() {}
    func setTint(_: Double) {}

    // MARK: - Tabs

    /// Open a tab. `background: true` leaves the user where they are — the
    /// default for agent-driven opens, which must never yank the visible page
    /// out from under someone.
    /// Mirrors `copad_core::browser::tabs::MAX_TABS_PER_PANE`.
    ///
    /// Enforced at CREATION, not only on restore. Without it the 101st tab
    /// opened happily and then vanished after a restart, because core's
    /// normalizer truncates the list on the way back in — a tab that worked
    /// until you quit is worse than one that was refused.
    static let maxTabs = 100

    /// Returns the new tab's id, or nil when the pane is full.
    @discardableResult
    func newTab(url: URL?, background: Bool) -> String? {
        guard tabs.count < Self.maxTabs else { return nil }
        let tab = BrowserTab(id: BrowserSnapshot.freshTabID(), url: url, restored: nil, owner: self)
        tabs.append(tab)
        tab.startIfNeeded()
        if !background {
            activeIndex = tabs.count - 1
            showActiveTab()
        }
        refreshTabStrip()
        NotificationCenter.default.post(name: .webviewStateChanged, object: self)
        return tab.id
    }

    func tab(id: String) -> BrowserTab? {
        tabs.first { $0.id == id }
    }

    @discardableResult
    func selectTab(id: String) -> Bool {
        guard let index = tabs.firstIndex(where: { $0.id == id }),
              !tabs[index].isRebuilding
        else { return false }
        activeIndex = index
        showActiveTab()
        refreshTabStrip()
        NotificationCenter.default.post(name: .webviewStateChanged, object: self)
        return true
    }

    /// Close a tab. Refuses the pane's LAST tab rather than leaving an empty
    /// pane — closing the pane itself is the tab manager's job, not this one's,
    /// and a pane with no tabs has no coherent snapshot.
    ///
    /// Returns an error message, or nil on success.
    func closeTab(id: String) -> String? {
        guard tabs.count > 1 else { return "cannot close the pane's last tab" }
        guard let index = tabs.firstIndex(where: { $0.id == id }) else {
            return "no such tab: \(id)"
        }
        // A tab mid-protection-transition has no web view at all. Both this
        // and `showActiveTab` used to dereference it unconditionally, so a
        // second RPC connection could crash the app during the rebuild.
        guard !tabs[index].isRebuilding else {
            return "the tab is changing protection mode; try again"
        }
        let wasActive = index == activeIndex
        tabs[index].webView?.removeFromSuperview()
        tabs.remove(at: index)
        // Keep the SAME tab selected when one before it closes; only fall to a
        // neighbour when the active tab itself went away.
        if wasActive {
            activeIndex = min(index, tabs.count - 1)
            showActiveTab()
        } else if index < activeIndex {
            activeIndex -= 1
        }
        refreshTabStrip()
        NotificationCenter.default.post(name: .webviewStateChanged, object: self)
        return nil
    }

    /// Returns an error message, or nil on success.
    func moveTab(id: String, to destination: Int) -> String? {
        guard let from = tabs.firstIndex(where: { $0.id == id }) else {
            return "no such tab: \(id)"
        }
        let to = max(0, min(destination, tabs.count - 1))
        guard to != from else { return nil }
        // Track the ACTIVE TAB, not its index: reordering shifts indices, and
        // carrying the number through would silently reselect a different tab.
        let activeID = activeTab.id
        let tab = tabs.remove(at: from)
        tabs.insert(tab, at: to)
        activeIndex = tabs.firstIndex { $0.id == activeID } ?? 0
        refreshTabStrip()
        NotificationCenter.default.post(name: .webviewStateChanged, object: self)
        return nil
    }

    /// True when ANY tab in this pane is protected. Protection freezes the
    /// whole profile, not one tab — a sibling tab is another window onto the
    /// same shared storage.
    var hasProtectedTab: Bool {
        tabs.contains { $0.tabMode == .protected }
    }

    /// Is ANY live browser pane protected?
    ///
    /// Every pane shares `WKWebsiteDataStore.default()`, so they share the
    /// logged-in session; protection has to mean the same thing across all of
    /// them or it means very little.
    ///
    /// Answered from a REGISTRY, not by walking up to the window. A pane in an
    /// inactive copad tab has been detached from the view hierarchy — its
    /// `view.window` is nil — so a window-based lookup silently fell back to
    /// "just this pane", and that pane's capture script went on writing the
    /// shared session's activity into an agent-readable file while another pane
    /// was protected.
    var profileHasProtectedTab: Bool {
        WebViewController.anyProtected
    }

    /// Every live browser pane, weakly held.
    ///
    /// `NSHashTable.weakObjects` so a closed pane drops out without anything
    /// having to remember to deregister it — a pane can be torn down from
    /// several paths and a missed deregistration would pin protection on
    /// forever.
    private static let livePanes = NSHashTable<WebViewController>.weakObjects()

    static var anyProtected: Bool {
        livePanes.allObjects.contains { $0.hasProtectedTab }
    }

    private func showActiveTab() {
        guard let host = webContainer, tabs.indices.contains(activeIndex) else { return }
        NSLayoutConstraint.deactivate(webConstraints)
        for subview in host.subviews { subview.removeFromSuperview() }
        // Nil while a protected-mode transition rebuilds it; `tabModeChanged`
        // calls back in once the new view exists.
        guard let wv = tabs[activeIndex].webView else { return }
        host.addSubview(wv)
        webConstraints = [
            wv.topAnchor.constraint(equalTo: host.topAnchor),
            wv.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            wv.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            wv.bottomAnchor.constraint(equalTo: host.bottomAnchor),
        ]
        NSLayoutConstraint.activate(webConstraints)
        syncBackgroundTabSizes()
        refreshChrome(for: tabs[activeIndex])
    }

    /// Keep background tabs the same size as the visible one.
    ///
    /// A tab that is not in the view hierarchy gets no layout, so without this
    /// it keeps whatever frame it was born with — and a screenshot of it would
    /// be laid out at a viewport the user never had. (A ZERO frame is worse
    /// still: `takeSnapshot` then returns an image that cannot be encoded at
    /// all, which is how this was found.)
    private func syncBackgroundTabSizes() {
        guard let host = webContainer else { return }
        let size = host.bounds.size.width > 1 ? host.bounds.size : BrowserTab.defaultFrame.size
        for (index, tab) in tabs.enumerated() where index != activeIndex {
            tab.setBackgroundSize(size)
        }
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        syncBackgroundTabSizes()
    }

    private func refreshTabStrip() {
        guard let strip = tabStrip else { return }
        for chip in strip.arrangedSubviews { strip.removeArrangedSubview(chip); chip.removeFromSuperview() }
        // A single tab gets no strip at all: a lone chip is chrome that tells
        // the user nothing they cannot already see in the URL bar.
        tabStripScroll?.isHidden = tabs.count < 2
        guard tabs.count > 1 else { return }
        for (index, tab) in tabs.enumerated() {
            strip.addArrangedSubview(makeTabChip(tab: tab, index: index, active: index == activeIndex))
        }
    }

    private func makeTabChip(tab: BrowserTab, index: Int, active: Bool) -> NSView {
        let button = NSButton()
        let label = tab.title.isEmpty ? "New tab" : tab.title
        button.title = label.count > 24 ? String(label.prefix(23)) + "…" : label
        button.bezelStyle = .recessed
        button.setButtonType(.pushOnPushOff)
        button.state = active ? .on : .off
        button.font = .systemFont(ofSize: 11)
        button.toolTip = tab.currentURL.isEmpty ? label : tab.currentURL
        button.tag = index
        button.target = self
        button.action = #selector(tabChipClicked(_:))
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }

    @objc private func tabChipClicked(_ sender: NSButton) {
        guard tabs.indices.contains(sender.tag) else { return }
        selectTab(id: tabs[sender.tag].id)
    }

    // MARK: - Callbacks from BrowserTab

    /// Keep the toolbar and strip in step with whichever tab changed. A
    /// background tab's navigation must update its CHIP but must not touch the
    /// URL bar, which belongs to the visible page.
    func refreshChrome(for tab: BrowserTab?) {
        guard let tab else { return }
        if tab === tabs[safe: activeIndex] {
            backButton?.isEnabled = tab.canGoBack
            forwardButton?.isEnabled = tab.canGoForward
            syncURLField(tab.currentURL)
        }
        refreshTabStrip()
    }

    func tabDidFinishLoading(_ tab: BrowserTab) {
        if tab === tabs[safe: activeIndex] {
            // A protected page's TITLE is page-derived and travels on the event
            // bus, which anything subscribed can read. "Reset password for …"
            // is the case that matters.
            currentTitle = tab.underProtection ? "Protected" : tab.title
            NotificationCenter.default.post(name: .terminalTitleChanged, object: self)
            eventBus?.broadcast(event: "webview.loaded", data: ["panel_id": panelID])
            eventBus?.broadcast(
                event: "webview.title_changed",
                data: ["panel_id": panelID, "title": currentTitle],
            )
            eventBus?.broadcast(
                event: "panel.title_changed",
                data: ["panel_id": panelID, "title": currentTitle],
            )
        }
        refreshTabStrip()
    }

    /// A tab entered or left protected mode. Its web view was replaced, so the
    /// visible one has to be re-installed and the chrome re-synced.
    func tabModeChanged(_ tab: BrowserTab) {
        if tab === tabs[safe: activeIndex] { showActiveTab() }
        refreshTabStrip()
        eventBus?.broadcast(event: "webview.tab_mode_changed", data: [
            "panel_id": panelID,
            "tab_id": tab.id,
            "mode": tab.tabMode.rawValue,
        ])
    }

    func tabDidCommit(_ tab: BrowserTab, url: String) {
        // Same reasoning as the log: an event carrying a protected page's URL
        // is a channel out, whoever is listening.
        guard !tab.underProtection else { return }
        eventBus?.broadcast(
            event: "webview.navigated",
            data: ["panel_id": panelID, "tab_id": tab.id, "url": url],
        )
    }

    // MARK: - Snapshot

    func snapshotPane(policy: String) -> BrowserPaneSnap {
        BrowserPaneSnap(
            tabs: tabs.map { $0.snapshot(policy: policy) },
            active: activeIndex,
        )
    }

    /// URL a snapshot would record for the pane as a whole — the active tab's.
    ///
    /// Blanked for a protected tab. `PaneManager.paneContent` persists this
    /// INDEPENDENTLY as `PaneContent.webview.url`, so blanking only
    /// `BrowserTabSnap.url` left a protected `/reset/<token>` URL reaching
    /// session.json through the legacy field under `url`/`full`.
    var snapshotSourceURL: String {
        guard let tab = tabs[safe: activeIndex], !tab.underProtection else { return "" }
        return tab.snapshotSourceURL
    }

    // MARK: - Toolbar

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

    private func syncURLField(_ urlString: String) {
        guard let urlField else { return }
        guard !urlString.isEmpty, urlString != "about:blank" else { return }
        // Don't clobber what the user is currently typing.
        if view.window?.firstResponder === urlField.currentEditor() { return }
        urlField.stringValue = urlString
    }

    @objc private func backTapped() { goBack() }
    @objc private func forwardTapped() { goForward() }
    @objc private func reloadTapped() { reload() }
    @objc private func devtoolsTapped() { toggleDevTools() }

    /// The toolbar's new-tab button switches to the new tab, unlike
    /// `webview.tab.new` which defaults to background — a human clicking `+`
    /// means "take me there", an agent calling it does not.
    @objc private func newTabTapped() {
        guard newTab(url: nil, background: false) != nil else {
            NSSound.beep()
            return
        }
        view.window?.makeFirstResponder(urlField)
    }

    @objc private func urlFieldSubmit(_ sender: NSTextField) {
        let text = sender.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        navigate(to: text)
        view.window?.makeFirstResponder(webView)
    }

    // MARK: - Navigation (active tab)

    func navigate(to urlString: String) { activeTab.navigate(to: urlString) }
    func goBack() { activeTab.goBack() }
    func goForward() { activeTab.goForward() }
    func reload() { activeTab.reload() }

    func executeJS(_ script: String, completion: @escaping (Any?, Error?) -> Void) {
        activeTab.executeJS(script, completion: completion)
    }

    func getContent(completion: @escaping (String) -> Void) {
        activeTab.getContent(completion: completion)
    }

    // MARK: - State

    func toggleDevTools() {
        // Enables right-click → "Inspect Element" via Safari Web Inspector.
        // Already set at tab creation; this re-applies it so a caller can
        // toggle at runtime.
        guard let webView else { return }
        let current = webView.configuration.preferences
            .value(forKey: "developerExtrasEnabled") as? Bool ?? false
        webView.configuration.preferences.setValue(!current, forKey: "developerExtrasEnabled")
    }

    var currentURL: String { tabs[safe: activeIndex]?.currentURL ?? "" }
    var canGoBack: Bool { tabs[safe: activeIndex]?.canGoBack ?? false }
    var canGoForward: Bool { tabs[safe: activeIndex]?.canGoForward ?? false }
    var isLoading: Bool { tabs[safe: activeIndex]?.isLoading ?? false }
}

private extension Array {
    /// Bounds-checked access. The tab list is mutated from socket commands, so
    /// an index that was valid when a callback was queued may not be by the
    /// time it runs.
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
