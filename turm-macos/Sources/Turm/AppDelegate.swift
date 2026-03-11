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

        let closeTabItem = NSMenuItem(title: "Close Tab", action: #selector(closeTab), keyEquivalent: "w")
        closeTabItem.target = self
        shellMenu.addItem(closeTabItem)

        shellMenu.addItem(.separator())

        for i in 1 ... 9 {
            let item = NSMenuItem(title: "Tab \(i)", action: #selector(switchTabByNumber(_:)), keyEquivalent: "\(i)")
            item.target = self
            item.tag = i
            shellMenu.addItem(item)
        }

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

    // MARK: - Tab Actions

    @objc private func newTab() {
        tabVC?.newTab()
    }

    @objc private func closeTab() {
        tabVC?.closeTab(at: tabVC?.activeIndex ?? 0)
    }

    @objc private func switchTabByNumber(_ sender: NSMenuItem) {
        tabVC?.switchTab(to: sender.tag - 1)
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
            vc.closeTab(at: vc.activeIndex)
            return ["ok": true]

        case "tab.list":
            return vc.tabList()

        default:
            return nil
        }
    }
}
