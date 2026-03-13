import AppKit

@MainActor
class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?
    var tabVC: TabViewController?
    private let socketServer = SocketServer()
    private let eventBus = EventBus()

    func applicationDidFinishLaunching(_: Notification) {
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
        vc.openInitialTab()

        if let path = config.backgroundPath {
            vc.applyBackground(path: path, tint: config.backgroundTint)
        }
    }

    func applicationWillTerminate(_: Notification) {
        socketServer.stop()
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

        // View menu (zoom)
        let viewItem = NSMenuItem()
        mainMenu.addItem(viewItem)
        let viewMenu = NSMenu(title: "View")
        viewItem.submenu = viewMenu

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

    // MARK: - Socket Server

    private func startSocketServer() {
        socketServer.eventBus = eventBus
        socketServer.commandHandler = { [weak self] method, params, completion in
            self?.handleCommand(method: method, params: params, completion: completion)
        }
        socketServer.start()
    }

    private func handleCommand(method: String, params: [String: Any], completion: @escaping (Any?) -> Void) {
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

        case "background.set":
            guard let path = params["path"] as? String else { completion(nil); return }
            let tint = params["tint"] as? Double ?? 0.6
            vc.applyBackground(path: path, tint: tint)
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
            guard let urlString = params["url"] as? String else { completion(nil); return }
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.navigate(to: urlString)
            completion(["ok": true])

        case "webview.back":
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.goBack()
            completion(["ok": true])

        case "webview.forward":
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.goForward()
            completion(["ok": true])

        case "webview.reload":
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.reload()
            completion(["ok": true])

        case "webview.execute_js":
            guard let script = params["script"] as? String else { completion(nil); return }
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.executeJS(script) { result, error in
                if let error {
                    completion(["error": error.localizedDescription])
                } else {
                    completion(["result": result ?? NSNull()])
                }
            }

        case "webview.get_content":
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.getContent { html in
                completion(["html": html])
            }

        case "webview.devtools":
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            webVC.toggleDevTools()
            completion(["ok": true])

        case "webview.state":
            guard let webVC = vc.activeWebView else { completion(["error": "no active webview"]); return }
            completion([
                "url": webVC.currentURL,
                "title": webVC.currentTitle,
                "can_go_back": webVC.canGoBack,
                "can_go_forward": webVC.canGoForward,
                "is_loading": webVC.isLoading,
            ])

        default:
            completion(nil)
        }
    }
}
