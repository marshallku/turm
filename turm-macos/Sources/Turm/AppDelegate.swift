import AppKit

@MainActor
class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?
    var tabVC: TabViewController?
    private let socketServer = SocketServer()
    private let eventBus = EventBus()
    private let actionRegistry = ActionRegistry()
    private var configWatcher: ConfigWatcher?

    func applicationDidFinishLaunching(_: Notification) {
        // PR 1 (Tier 2.1) FFI smoke test. Proves the Rust staticlib linked
        // correctly and a JSON round-trip survives the C-ABI boundary.
        // Remove once Tier 2.4 (TriggerEngine via FFI) replaces it with real
        // engine startup.
        TurmFFI.runSmokeTest()

        // PR 2 (Tier 2.3) registry seam — register first-party actions so the
        // socket dispatcher can hand off to them via tryDispatch BEFORE the
        // legacy switch fires. Plugin host (PR 3) and trigger engine (PR 5)
        // will register additional actions through this same path.
        registerBuiltinActions()

        let config = TurmConfig.load()
        let theme = TurmTheme.byName(config.themeName) ?? .catppuccinMocha

        setupMenuBar()

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false,
        )
        window.title = "turm"
        window.center()
        window.isRestorable = false
        window.backgroundColor = theme.background.nsColor

        let vc = TabViewController(config: config, theme: theme)
        window.contentViewController = vc
        window.setContentSize(NSSize(width: 1200, height: 800))
        window.center()

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        self.window = window
        tabVC = vc
        vc.eventBus = eventBus
        startSocketServer()
        startConfigWatcher()
        vc.openInitialTab()

        if let path = config.backgroundPath {
            vc.applyBackground(path: path, tint: config.backgroundTint, opacity: config.backgroundOpacity)
        }
    }

    func applicationWillTerminate(_: Notification) {
        socketServer.stop()
        configWatcher?.stop()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
        true
    }

    // MARK: - Menu Bar

    private func setupMenuBar() {
        let mainMenu = NSMenu()

        // App menu
        let appItem = NSMenuItem()
        mainMenu.addItem(appItem)
        let appMenu = NSMenu()
        appItem.submenu = appMenu
        appMenu.addItem(withTitle: "Quit turm", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")

        // Shell menu (tab management)
        let shellItem = NSMenuItem()
        mainMenu.addItem(shellItem)
        let shellMenu = NSMenu(title: "Shell")
        shellItem.submenu = shellMenu

        let newTabItem = NSMenuItem(title: "New Tab", action: #selector(newTab), keyEquivalent: "t")
        newTabItem.target = self
        shellMenu.addItem(newTabItem)

        let newWebTabItem = NSMenuItem(title: "New Web Tab", action: #selector(newWebTab), keyEquivalent: "t")
        newWebTabItem.keyEquivalentModifierMask = [.command, .shift]
        newWebTabItem.target = self
        shellMenu.addItem(newWebTabItem)

        let closePaneItem = NSMenuItem(title: "Close Pane", action: #selector(closePane), keyEquivalent: "w")
        closePaneItem.target = self
        shellMenu.addItem(closePaneItem)

        shellMenu.addItem(.separator())

        let splitHItem = NSMenuItem(title: "Split Pane Horizontally", action: #selector(splitHorizontal), keyEquivalent: "d")
        splitHItem.target = self
        shellMenu.addItem(splitHItem)

        let splitVItem = NSMenuItem(title: "Split Pane Vertically", action: #selector(splitVertical), keyEquivalent: "D")
        splitVItem.keyEquivalentModifierMask = [.command, .shift]
        splitVItem.target = self
        shellMenu.addItem(splitVItem)

        shellMenu.addItem(.separator())

        for i in 1 ... 9 {
            let item = NSMenuItem(title: "Tab \(i)", action: #selector(switchTabByNumber(_:)), keyEquivalent: "\(i)")
            item.target = self
            item.tag = i
            shellMenu.addItem(item)
        }

        // Find menu
        let findItem = NSMenuItem()
        mainMenu.addItem(findItem)
        let findMenu = NSMenu(title: "Find")
        findItem.submenu = findMenu

        let findAction = NSMenuItem(title: "Find…", action: #selector(performFindPanelAction(_:)), keyEquivalent: "f")
        findAction.tag = Int(NSFindPanelAction.showFindPanel.rawValue)
        findMenu.addItem(findAction)

        let findNextAction = NSMenuItem(title: "Find Next", action: #selector(performFindPanelAction(_:)), keyEquivalent: "g")
        findNextAction.tag = Int(NSFindPanelAction.next.rawValue)
        findMenu.addItem(findNextAction)

        let findPrevAction = NSMenuItem(title: "Find Previous", action: #selector(performFindPanelAction(_:)), keyEquivalent: "G")
        findPrevAction.keyEquivalentModifierMask = NSEvent.ModifierFlags([.command, .shift])
        findPrevAction.tag = Int(NSFindPanelAction.previous.rawValue)
        findMenu.addItem(findPrevAction)

        // View menu (zoom + tab bar toggle)
        let viewItem = NSMenuItem()
        mainMenu.addItem(viewItem)
        let viewMenu = NSMenu(title: "View")
        viewItem.submenu = viewMenu

        let toggleTabBarItem = NSMenuItem(title: "Toggle Tab Bar", action: #selector(toggleTabBar), keyEquivalent: "b")
        toggleTabBarItem.keyEquivalentModifierMask = [.command, .shift]
        toggleTabBarItem.target = self
        viewMenu.addItem(toggleTabBarItem)

        viewMenu.addItem(.separator())

        let zoomIn = NSMenuItem(title: "Zoom In", action: #selector(zoomIn), keyEquivalent: "=")
        zoomIn.target = self
        viewMenu.addItem(zoomIn)

        let zoomOut = NSMenuItem(title: "Zoom Out", action: #selector(zoomOut), keyEquivalent: "-")
        zoomOut.target = self
        viewMenu.addItem(zoomOut)

        let zoomReset = NSMenuItem(title: "Actual Size", action: #selector(zoomReset), keyEquivalent: "0")
        zoomReset.target = self
        viewMenu.addItem(zoomReset)

        NSApp.mainMenu = mainMenu
    }

    // MARK: - Tab / Pane Actions

    @objc private func newTab() {
        tabVC?.newTab()
    }

    @objc private func newWebTab() {
        tabVC?.newWebViewTab()
    }

    @objc private func closePane() {
        tabVC?.closeActivePane()
    }

    @objc private func splitHorizontal() {
        tabVC?.splitActivePane(orientation: .horizontal)
    }

    @objc private func splitVertical() {
        tabVC?.splitActivePane(orientation: .vertical)
    }

    @objc private func switchTabByNumber(_ sender: NSMenuItem) {
        tabVC?.switchTab(to: sender.tag - 1)
    }

    // MARK: - Tab Bar Toggle

    @objc func toggleTabBar() {
        tabVC?.toggleTabBar(userInitiated: true)
    }

    // MARK: - Find

    @objc func performFindPanelAction(_ sender: NSMenuItem) {
        tabVC?.activeTerminal?.view.perform(#selector(performFindPanelAction(_:)), with: sender)
    }

    // MARK: - Zoom Actions

    @objc private func zoomIn() {
        tabVC?.activeTerminal?.zoomIn()
    }

    @objc private func zoomOut() {
        tabVC?.activeTerminal?.zoomOut()
    }

    @objc private func zoomReset() {
        tabVC?.activeTerminal?.zoomReset()
    }

    // MARK: - Action Registry

    /// First-party `system.*` actions that should be reachable through
    /// the registry from day one. Plugin host (PR 3) and trigger engine
    /// (PR 5) will register their own handlers via the same registry.
    private func registerBuiltinActions() {
        // system.ffi_test — proxy to TurmFFI.callJSON. Two purposes:
        //   1. Proves the registry seam is reachable from `turmctl call`,
        //      end-to-end through SocketServer.dispatch.
        //   2. Gives PR 5 (trigger engine via FFI) a smoke test target it
        //      can dispatch as a registered action — same code path as a
        //      plugin will use.
        actionRegistry.register("system.ffi_test") { params, completion in
            // Pass through whatever the caller sent; if absent, send an
            // empty object so the FFI side still gets a valid JSON object.
            let payload = params.isEmpty ? ["caller": "system.ffi_test"] : params
            if let echoed = TurmFFI.callJSON(payload) {
                completion(["echoed": echoed, "ffi_version": TurmFFI.version()])
            } else {
                completion(RPCError(
                    code: "ffi_error",
                    message: TurmFFI.lastError() ?? "TurmFFI.callJSON returned nil",
                ))
            }
        }

        // system.list_actions — introspection for diagnostics. Returns
        // every name registered through the action registry. Mirrors
        // Linux's debug behavior of being able to query "what actions
        // exist right now".
        actionRegistry.register("system.list_actions") { [weak self] _, completion in
            guard let self else {
                completion(RPCError(code: "no_app", message: "AppDelegate gone"))
                return
            }
            completion([
                "count": actionRegistry.count,
                "names": actionRegistry.names(),
            ])
        }
    }

    // MARK: - Socket Server

    // MARK: - Config Watcher

    private func startConfigWatcher() {
        let configURL = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/turm/config.toml")
        let watcher = ConfigWatcher(url: configURL)
        watcher.onChange = { [weak self] in self?.handleConfigChange() }
        watcher.start()
        configWatcher = watcher
    }

    private func handleConfigChange() {
        let newConfig = TurmConfig.load()
        let newTheme = TurmTheme.byName(newConfig.themeName) ?? .catppuccinMocha
        tabVC?.applyConfig(newConfig, theme: newTheme)
        eventBus.broadcast(event: "config.reloaded", data: ["theme": newTheme.name])
    }

    private func startSocketServer() {
        socketServer.eventBus = eventBus
        socketServer.commandHandler = { [weak self] method, params, completion in
            self?.handleCommand(method: method, params: params, completion: completion)
        }
        socketServer.start()
    }

    private func handleCommand(method: String, params: [String: Any], completion: @escaping (Any?) -> Void) {
        // Registry takes precedence over the legacy switch — PR 3 (plugin
        // supervisor) and PR 5 (trigger engine) register their handlers
        // here. tryDispatch returns false when nothing's registered under
        // `method`, in which case completion is untouched and we fall
        // through to the hardcoded handlers below.
        if actionRegistry.tryDispatch(method, params: params, completion: completion) {
            return
        }

        guard let vc = tabVC else { completion(nil); return }

        switch method {
        case "system.ping":
            completion(["status": "ok"])

        case "terminal.exec":
            guard let command = params["command"] as? String else { completion(nil); return }
            vc.execCommand(command)
            completion(["ok": true])

        case "terminal.feed":
            guard let text = params["text"] as? String else { completion(nil); return }
            vc.feedText(text)
            completion(["ok": true])

        case "terminal.state":
            completion(vc.terminalState())

        case "terminal.read":
            completion(vc.readScreen())

        case "terminal.history":
            let lines = params["lines"] as? Int ?? 100
            completion(vc.activeTerminal?.history(lines: lines))

        case "terminal.context":
            let historyLines = params["history_lines"] as? Int ?? 50
            completion(vc.activeTerminal?.context(historyLines: historyLines))

        case "tab.new":
            vc.newTab()
            completion(["ok": true])

        case "tab.close":
            vc.closeActivePane()
            completion(["ok": true])

        case "tab.switch":
            guard let index = params["index"] as? Int else { completion(nil); return }
            vc.switchTab(to: index)
            completion(["ok": true])

        case "tab.list":
            completion(vc.tabList())

        case "tab.info":
            completion(vc.tabInfo())

        case "tab.rename":
            guard let title = params["title"] as? String else { completion(nil); return }
            let index = params["index"] as? Int ?? vc.activeIndex
            vc.renameTab(at: index, title: title)
            completion(["ok": true])

        case "split.horizontal":
            vc.splitActivePane(orientation: .horizontal)
            completion(["ok": true])

        case "split.vertical":
            vc.splitActivePane(orientation: .vertical)
            completion(["ok": true])

        case "session.list":
            completion(vc.sessionList())

        case "session.info":
            let index = params["index"] as? Int ?? vc.activeIndex
            completion(vc.sessionInfo(index: index))

        case "terminal.shell_precmd":
            let panelID = params["panel_id"] as? String ?? vc.activeTerminal?.panelID ?? ""
            eventBus.broadcast(event: "terminal.shell_precmd", data: ["panel_id": panelID])
            completion(["ok": true])

        case "terminal.shell_preexec":
            let panelID = params["panel_id"] as? String ?? vc.activeTerminal?.panelID ?? ""
            eventBus.broadcast(event: "terminal.shell_preexec", data: ["panel_id": panelID])
            completion(["ok": true])

        case "agent.approve":
            guard let message = params["message"] as? String else { completion(nil); return }
            let title = params["title"] as? String ?? "Agent Action"
            let actions = params["actions"] as? [String] ?? ["Approve", "Deny"]
            guard let win = window else { completion(["error": "no window"]); return }
            let alert = NSAlert()
            alert.messageText = title
            alert.informativeText = message
            for action in actions {
                alert.addButton(withTitle: action)
            }
            alert.beginSheetModal(for: win) { response in
                // NSApplication.ModalResponse.alertFirstButtonReturn = 1000
                let idx = response.rawValue - 1000
                let chosen = actions.indices.contains(idx) ? actions[idx] : actions.last ?? "Deny"
                completion(["action": chosen, "index": idx])
            }
            // completion called async from sheet modal callback above — do not call here

        case "tabs.toggle_bar":
            vc.toggleTabBar(userInitiated: true)
            completion(["ok": true, "collapsed": vc.isTabBarCollapsed])

        case "background.set":
            guard let path = params["path"] as? String else { completion(nil); return }
            let tint = params["tint"] as? Double ?? 0.6
            let opacity = params["opacity"] as? Double ?? 1.0
            vc.applyBackground(path: path, tint: tint, opacity: opacity)
            completion(["ok": true])

        case "background.set_tint":
            guard let tint = params["tint"] as? Double else { completion(nil); return }
            vc.setTint(tint)
            completion(["ok": true])

        case "background.clear":
            vc.clearBackground()
            completion(["ok": true])

        // MARK: WebView commands

        case "webview.open":
            let urlString = params["url"] as? String
            let url = urlString.flatMap { s -> URL? in
                let final = s.hasPrefix("http://") || s.hasPrefix("https://") || s.hasPrefix("file://") ? s : "https://" + s
                return URL(string: final)
            }
            let mode = params["mode"] as? String ?? "tab"
            switch mode {
            case "split_h":
                vc.splitActivePaneWithWebView(url: url, orientation: .horizontal)
            case "split_v":
                vc.splitActivePaneWithWebView(url: url, orientation: .vertical)
            default: // "tab"
                vc.newWebViewTab(url: url)
            }
            completion(["ok": true])

        case "webview.navigate":
            guard let urlString = params["url"] as? String else {
                completion(RPCError(code: "invalid_params", message: "Missing 'url' param"))
                return
            }
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                webVC.navigate(to: urlString)
                completion(["status": "ok"])
            }

        case "webview.back":
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                webVC.goBack()
                completion(["status": "ok"])
            }

        case "webview.forward":
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                webVC.goForward()
                completion(["status": "ok"])
            }

        case "webview.reload":
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                webVC.reload()
                completion(["status": "ok"])
            }

        case "webview.execute_js":
            // Param name is `code` (Linux + turm-cli convention). Older callers that
            // sent `script` get a fallback so existing macOS-only consumers don't break.
            guard let code = (params["code"] as? String) ?? (params["script"] as? String) else {
                completion(RPCError(code: "invalid_params", message: "Missing 'code' param"))
                return
            }
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                webVC.executeJS(code) { result, error in
                    if let error {
                        completion(RPCError(code: "js_error", message: error.localizedDescription))
                    } else {
                        completion(["result": result ?? NSNull()])
                    }
                }
            }

        case "webview.get_content":
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                webVC.getContent { html in
                    completion(["html": html])
                }
            }

        case "webview.devtools":
            // Linux accepts `action: show/close/attach/detach`. macOS WKWebView
            // exposes no public API to programmatically open the inspector
            // window — `developerExtrasEnabled` only enables the right-click
            // → "Inspect Element" menu. We accept the action verb for protocol
            // parity but treat show/attach/detach as "ensure enabled" and
            // close as "no-op" (the user closes the inspector window manually).
            let action = (params["action"] as? String) ?? "show"
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                switch action {
                case "show", "attach", "detach", "toggle":
                    webVC.toggleDevTools()
                    completion(["status": "ok"])
                case "close":
                    completion(["status": "ok"])
                default:
                    completion(RPCError(
                        code: "invalid_params",
                        message: "Unknown action: \(action). Use show/close/attach/detach/toggle",
                    ))
                }
            }

        case "webview.state":
            switch resolveWebView(params, in: vc) {
            case let .failure(err): completion(err)
            case let .success(webVC):
                completion([
                    "url": webVC.currentURL,
                    "title": webVC.currentTitle,
                    "can_go_back": webVC.canGoBack,
                    "can_go_forward": webVC.canGoForward,
                    "is_loading": webVC.isLoading,
                ])
            }

        default:
            completion(nil)
        }
    }

    /// Resolves the target WebViewController for an `id`-aware webview command.
    ///
    /// - If `params["id"]` is a non-empty string, look it up across all tabs and
    ///   return the panel (errors out on `not_found` / `wrong_panel_type`,
    ///   matching Linux `socket.rs` codes).
    /// - If `id` is absent, fall back to the active webview. Linux's handlers
    ///   require `id`; macOS keeps the lenient default per the parity plan
    ///   (Tier 1.6) so existing turmctl-without-id calls keep working.
    private func resolveWebView(
        _ params: [String: Any],
        in vc: TabViewController,
    ) -> Result<WebViewController, RPCError> {
        if let id = params["id"] as? String, !id.isEmpty {
            guard let panel = vc.panel(id: id) else {
                return .failure(RPCError(code: "not_found", message: "Panel not found: \(id)"))
            }
            guard let webVC = panel as? WebViewController else {
                return .failure(RPCError(code: "wrong_panel_type", message: "Panel is not a webview"))
            }
            return .success(webVC)
        }
        guard let webVC = vc.activeWebView else {
            return .failure(RPCError(
                code: "no_active_webview",
                message: "No active webview and no 'id' provided",
            ))
        }
        return .success(webVC)
    }
}
