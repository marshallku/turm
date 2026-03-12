import AppKit

/// Manages multiple terminal tabs, each backed by a PaneManager (split-pane tree).
@MainActor
final class TabViewController: NSViewController {
    private let config: TurmConfig
    private let theme: TurmTheme

    private var tabBar: TabBarView!
    private var contentArea: NSView!
    private var paneManagers: [PaneManager] = []
    private(set) var activeIndex: Int = -1

    // Retained so new tabs inherit the current background state
    private var currentBackgroundPath: String?
    private var currentBackgroundTint: Double = 0.6

    var activePaneManager: PaneManager? {
        paneManagers.indices.contains(activeIndex) ? paneManagers[activeIndex] : nil
    }

    var activeTerminal: TerminalViewController? {
        activePaneManager?.activePane
    }

    init(config: TurmConfig, theme: TurmTheme) {
        self.config = config
        self.theme = theme
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    // MARK: - View Lifecycle

    override func loadView() {
        let root = NSView()
        root.wantsLayer = true
        root.layer?.backgroundColor = theme.background.nsColor.cgColor

        tabBar = TabBarView(theme: theme)
        tabBar.translatesAutoresizingMaskIntoConstraints = false
        tabBar.onSelectTab = { [weak self] i in self?.switchTab(to: i) }
        tabBar.onCloseTab = { [weak self] i in self?.closeTabByButton(at: i) }
        tabBar.onNewTab = { [weak self] in self?.newTab() }
        root.addSubview(tabBar)

        contentArea = NSView()
        contentArea.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(contentArea)

        NSLayoutConstraint.activate([
            tabBar.topAnchor.constraint(equalTo: root.topAnchor),
            tabBar.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            tabBar.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            tabBar.heightAnchor.constraint(equalToConstant: TabBarView.height),

            contentArea.topAnchor.constraint(equalTo: tabBar.bottomAnchor),
            contentArea.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            contentArea.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            contentArea.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])

        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
    }

    func openInitialTab() {
        newTab()
    }

    // MARK: - Tab Operations

    func newTab() {
        let manager = PaneManager(config: config, theme: theme)
        manager.onLastPaneClosed = { [weak self, weak manager] in
            guard let self, let manager else { return }
            if let index = paneManagers.firstIndex(where: { $0 === manager }) {
                closeTab(at: index)
            }
        }
        manager.onActivePaneChanged = { [weak self] in
            self?.refreshTabBar()
        }

        // Observe title changes from any terminal in this manager
        NotificationCenter.default.addObserver(
            forName: .terminalTitleChanged,
            object: nil,
            queue: .main,
        ) { [weak self] _ in
            Task { @MainActor in self?.refreshTabBar() }
        }

        paneManagers.append(manager)
        switchTab(to: paneManagers.count - 1)
        // Inherit current background state
        if let path = currentBackgroundPath {
            manager.applyBackground(path: path, tint: currentBackgroundTint)
        }
    }

    func closeTab(at index: Int) {
        guard paneManagers.indices.contains(index) else { return }

        let manager = paneManagers[index]
        manager.containerView.removeFromSuperview()
        paneManagers.remove(at: index)

        if paneManagers.isEmpty {
            view.window?.close()
            return
        }

        let nextIndex = min(activeIndex, paneManagers.count - 1)
        activeIndex = -1
        switchTab(to: nextIndex)
    }

    /// Called from tab bar close button — closes all panes in the tab.
    private func closeTabByButton(at index: Int) {
        guard paneManagers.indices.contains(index) else { return }
        let manager = paneManagers[index]
        // Terminate all shells before closing
        manager.allTerminals().forEach { $0.view.removeFromSuperview(); $0.removeFromParent() }
        manager.containerView.removeFromSuperview()
        paneManagers.remove(at: index)

        if paneManagers.isEmpty {
            view.window?.close()
            return
        }

        let nextIndex = min(activeIndex, paneManagers.count - 1)
        activeIndex = -1
        switchTab(to: nextIndex)
    }

    func switchTab(to index: Int) {
        guard paneManagers.indices.contains(index), index != activeIndex else { return }

        // Remove current manager's container
        if let current = activePaneManager {
            current.containerView.removeFromSuperview()
        }

        activeIndex = index
        let manager = paneManagers[index]

        contentArea.addSubview(manager.containerView)
        NSLayoutConstraint.activate([
            manager.containerView.topAnchor.constraint(equalTo: contentArea.topAnchor),
            manager.containerView.leadingAnchor.constraint(equalTo: contentArea.leadingAnchor),
            manager.containerView.trailingAnchor.constraint(equalTo: contentArea.trailingAnchor),
            manager.containerView.bottomAnchor.constraint(equalTo: contentArea.bottomAnchor),
        ])

        view.layoutSubtreeIfNeeded()
        manager.allTerminals().forEach { $0.startShellIfNeeded() }
        manager.activePane.view.window?.makeFirstResponder(manager.activePane.view)

        refreshTabBar()
    }

    // MARK: - Split Operations

    func splitActivePane(orientation: SplitOrientation) {
        activePaneManager?.splitActive(orientation: orientation)
    }

    func closeActivePane() {
        activePaneManager?.closeActive()
    }

    // MARK: - Tab Bar

    private func refreshTabBar() {
        let titles = paneManagers.map(\.activePane.currentTitle)
        tabBar.setTabs(titles: titles, activeIndex: activeIndex)
    }

    // MARK: - Background

    func applyBackground(path: String, tint: Double) {
        currentBackgroundPath = path
        currentBackgroundTint = tint
        paneManagers.forEach { $0.applyBackground(path: path, tint: tint) }
    }

    func clearBackground() {
        currentBackgroundPath = nil
        paneManagers.forEach { $0.clearBackground() }
    }

    func setTint(_ alpha: Double) {
        currentBackgroundTint = alpha
        paneManagers.forEach { $0.setTint(alpha) }
    }

    // MARK: - Socket Commands

    func execCommand(_ command: String) {
        activeTerminal?.execCommand(command)
    }

    func feedText(_ text: String) {
        activeTerminal?.feedText(text)
    }

    func terminalState() -> [String: Any] {
        activeTerminal?.terminalState() ?? [:]
    }

    func readScreen() -> [String: Any] {
        activeTerminal?.readScreen() ?? [:]
    }

    func tabList() -> [[String: Any]] {
        paneManagers.enumerated().map { i, m in
            ["index": i, "title": m.activePane.currentTitle, "active": i == activeIndex]
        }
    }

    /// Extended tab info including pane count (tab.info).
    func tabInfo() -> [[String: Any]] {
        paneManagers.enumerated().map { i, m in
            [
                "index": i,
                "title": m.activePane.currentTitle,
                "active": i == activeIndex,
                "pane_count": m.allTerminals().count,
            ]
        }
    }

    /// Rename a tab by overriding its title (tab.rename).
    func renameTab(at index: Int, title: String) {
        guard paneManagers.indices.contains(index) else { return }
        // Store the override title on the active pane of that tab
        paneManagers[index].setCustomTitle(title)
        refreshTabBar()
        if index == activeIndex {
            view.window?.title = title
        }
    }

    /// Session-level info: all tabs (session.list).
    func sessionList() -> [[String: Any]] {
        tabList()
    }

    /// Info for a specific tab by index (session.info).
    func sessionInfo(index: Int) -> [String: Any]? {
        guard paneManagers.indices.contains(index) else { return nil }
        let m = paneManagers[index]
        let state = m.activePane.terminalState()
        return [
            "index": index,
            "title": m.activePane.currentTitle,
            "active": index == activeIndex,
            "pane_count": m.allTerminals().count,
            "cols": state["cols"] ?? 0,
            "rows": state["rows"] ?? 0,
        ]
    }
}
