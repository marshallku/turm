import AppKit

@MainActor
class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?
    var tabVC: TabViewController?
    private let socketServer = SocketServer()

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
        // contentViewController causes the window to resize to the view's Auto Layout
        // minimum size. Restore the intended 1200×800 and re-center afterwards.
        window.setContentSize(NSSize(width: 1200, height: 800))
        window.center()

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        self.window = window
        tabVC = vc
        startSocketServer()
        vc.openInitialTab()

        // Apply background from config if set
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

        // Find menu — action is forwarded through the responder chain to SwiftTerm's MacTerminalView
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

    /// Forwards find panel actions to SwiftTerm's MacTerminalView, which implements
    /// performFindPanelAction(_:) with a built-in find bar (case/regex/whole-word options).
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
        socketServer.commandHandler = { [weak self] method, params in
            self?.handleCommand(method: method, params: params)
        }
        socketServer.start()
    }

    private func handleCommand(method: String, params: [String: Any]) -> Any? {
        guard let vc = tabVC else { return nil }
        switch method {
        case "system.ping":
            return ["status": "ok"]

        case "terminal.exec":
            guard let command = params["command"] as? String else { return nil }
            vc.execCommand(command)
            return ["ok": true]

        case "terminal.feed":
            guard let text = params["text"] as? String else { return nil }
            vc.feedText(text)
            return ["ok": true]

        case "terminal.state":
            return vc.terminalState()

        case "terminal.read":
            return vc.readScreen()

        case "tab.new":
            vc.newTab()
            return ["ok": true]

        case "tab.close":
            vc.closeActivePane()
            return ["ok": true]

        case "split.horizontal":
            vc.splitActivePane(orientation: .horizontal)
            return ["ok": true]

        case "split.vertical":
            vc.splitActivePane(orientation: .vertical)
            return ["ok": true]

        case "tab.switch":
            guard let index = params["index"] as? Int else { return nil }
            vc.switchTab(to: index)
            return ["ok": true]

        case "tab.list":
            return vc.tabList()

        case "tab.info":
            return vc.tabInfo()

        case "tab.rename":
            guard let title = params["title"] as? String else { return nil }
            let index = params["index"] as? Int ?? vc.activeIndex
            vc.renameTab(at: index, title: title)
            return ["ok": true]

        case "terminal.history":
            let lines = params["lines"] as? Int ?? 100
            return vc.activeTerminal?.history(lines: lines)

        case "terminal.context":
            let historyLines = params["history_lines"] as? Int ?? 50
            return vc.activeTerminal?.context(historyLines: historyLines)

        case "session.list":
            return vc.sessionList()

        case "session.info":
            let index = params["index"] as? Int ?? vc.activeIndex
            return vc.sessionInfo(index: index)

        case "background.set":
            guard let path = params["path"] as? String else { return nil }
            let tint = params["tint"] as? Double ?? 0.6
            vc.applyBackground(path: path, tint: tint)
            return ["ok": true]

        case "background.set_tint":
            guard let tint = params["tint"] as? Double else { return nil }
            vc.setTint(tint)
            return ["ok": true]

        case "background.clear":
            vc.clearBackground()
            return ["ok": true]

        default:
            return nil
        }
    }
}
