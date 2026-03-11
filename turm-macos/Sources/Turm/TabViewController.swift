import AppKit

/// Manages multiple terminal tabs.
/// Mirrors the role of TabManager + tabs.rs in turm-linux.
@MainActor
final class TabViewController: NSViewController {
    private let config: TurmConfig
    private let theme: TurmTheme

    private var tabBar: TabBarView!
    private var contentArea: NSView!
    private var terminals: [TerminalViewController] = []
    private(set) var activeIndex: Int = -1

    var activeTerminal: TerminalViewController? {
        terminals.indices.contains(activeIndex) ? terminals[activeIndex] : nil
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
        tabBar.onCloseTab = { [weak self] i in self?.closeTab(at: i) }
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

    /// Called by AppDelegate after makeKeyAndOrderFront.
    /// Sets view.frame from the window's content rect so Auto Layout resolves
    /// subviews at the real size before the first shell starts.
    func openInitialTab() {
        newTab()
    }

    // MARK: - Tab Operations

    func newTab() {
        let termVC = TerminalViewController(config: config, theme: theme)
        addChild(termVC)
        terminals.append(termVC)

        termVC.onProcessTerminated = { [weak self, weak termVC] in
            guard let self, let termVC else { return }
            if let index = terminals.firstIndex(where: { $0 === termVC }) {
                closeTab(at: index)
            }
        }

        // Observe title changes to refresh tab bar
        NotificationCenter.default.addObserver(
            forName: .terminalTitleChanged,
            object: termVC,
            queue: .main,
        ) { [weak self] _ in
            Task { @MainActor in self?.refreshTabBar() }
        }

        switchTab(to: terminals.count - 1)
    }

    func closeTab(at index: Int) {
        guard terminals.indices.contains(index) else { return }

        // Remove child VC
        let termVC = terminals[index]
        termVC.view.removeFromSuperview()
        termVC.removeFromParent()
        terminals.remove(at: index)

        if terminals.isEmpty {
            view.window?.close()
            return
        }

        let nextIndex = min(activeIndex, terminals.count - 1)
        // Force update even if index didn't change (the tab at that index changed)
        activeIndex = -1
        switchTab(to: nextIndex)
    }

    func switchTab(to index: Int) {
        guard terminals.indices.contains(index), index != activeIndex else { return }

        // Remove current terminal view
        if let current = activeTerminal {
            current.view.removeFromSuperview()
        }

        activeIndex = index
        let termVC = terminals[index]

        // Embed new terminal view
        termVC.view.translatesAutoresizingMaskIntoConstraints = false
        contentArea.addSubview(termVC.view)
        NSLayoutConstraint.activate([
            termVC.view.topAnchor.constraint(equalTo: contentArea.topAnchor),
            termVC.view.leadingAnchor.constraint(equalTo: contentArea.leadingAnchor),
            termVC.view.trailingAnchor.constraint(equalTo: contentArea.trailingAnchor),
            termVC.view.bottomAnchor.constraint(equalTo: contentArea.bottomAnchor),
        ])

        view.layoutSubtreeIfNeeded()
        termVC.startShellIfNeeded()

        termVC.view.window?.makeFirstResponder(termVC.view)

        refreshTabBar()
    }

    private func refreshTabBar() {
        let titles = terminals.map(\.currentTitle)
        tabBar.setTabs(titles: titles, activeIndex: activeIndex)
    }

    // MARK: - Terminal tab title updates

    func terminalTitleChanged(for termVC: TerminalViewController) {
        guard let index = terminals.firstIndex(where: { $0 === termVC }) else { return }
        tabBar.updateTitle(termVC.currentTitle, at: index)
        if index == activeIndex {
            view.window?.title = termVC.currentTitle
        }
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
        terminals.enumerated().map { i, t in
            ["index": i, "title": t.currentTitle, "active": i == activeIndex]
        }
    }
}
