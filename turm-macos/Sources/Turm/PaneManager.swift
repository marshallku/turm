import AppKit

/// NSSplitView subclass that distributes all subviews equally on the first resize pass.
/// Works for any number of subviews (N panes → each gets 1/N of available space).
/// After the initial layout the user can freely drag dividers to any position.
///
/// Using NSSplitViewDelegate.splitView(_:resizeSubviewsWithOldSize:) rather than
/// layout() because NSSplitView sets subview frames via resizeSubviews, which runs
/// *before* layout(). By the time layout() fires, the (wrong) frames are already
/// committed. The delegate method intercepts at exactly the right moment.
private class EqualSplitView: NSSplitView, NSSplitViewDelegate {
    private var initialSizeSet = false

    override init(frame: NSRect) {
        super.init(frame: frame)
        delegate = self
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    func splitView(_ splitView: NSSplitView, resizeSubviewsWithOldSize _: NSSize) {
        let total = isVertical ? splitView.frame.width : splitView.frame.height
        guard total > 0, splitView.subviews.count >= 2 else {
            splitView.adjustSubviews()
            return
        }

        if initialSizeSet {
            // After initial sizing: let NSSplitView handle normal proportional resize.
            splitView.adjustSubviews()
            return
        }
        initialSizeSet = true

        let n = splitView.subviews.count
        let eachSize = (total - dividerThickness * CGFloat(n - 1)) / CGFloat(n)
        if isVertical {
            var x: CGFloat = 0
            for sub in splitView.subviews {
                sub.frame = NSRect(x: x, y: 0, width: eachSize, height: splitView.frame.height)
                x += eachSize + dividerThickness
            }
        } else {
            var y: CGFloat = 0
            for sub in splitView.subviews {
                sub.frame = NSRect(x: 0, y: y, width: splitView.frame.width, height: eachSize)
                y += eachSize + dividerThickness
            }
        }
    }
}

/// Manages the split-pane tree for a single tab.
/// TabViewController embeds `containerView` once; PaneManager rebuilds its
/// contents on every split/close using fresh NSSplitView instances.
@MainActor
final class PaneManager {
    private let config: TurmConfig
    private let theme: TurmTheme

    private(set) var root: SplitNode
    private(set) var activePane: TerminalViewController

    /// Stable container — TabViewController pins this to contentArea once and never re-embeds.
    let containerView: NSView

    var onLastPaneClosed: (() -> Void)?
    var onActivePaneChanged: (() -> Void)?

    private nonisolated(unsafe) var clickMonitor: Any?
    /// Tracks the fill constraints added to containerView so they can be
    /// deactivated before the next rebuild.
    private var rootConstraints: [NSLayoutConstraint] = []

    // MARK: - Init

    init(config: TurmConfig, theme: TurmTheme) {
        self.config = config
        self.theme = theme

        let termVC = TerminalViewController(config: config, theme: theme)
        root = .leaf(termVC)
        activePane = termVC

        containerView = NSView()
        containerView.translatesAutoresizingMaskIntoConstraints = false

        wireTerminal(termVC)
        rebuildViewHierarchy()
        installClickMonitor()
    }

    deinit {
        if let m = clickMonitor { NSEvent.removeMonitor(m) }
    }

    // MARK: - Public API

    func splitActive(orientation: SplitOrientation) {
        let newTermVC = TerminalViewController(config: config, theme: theme)
        wireTerminal(newTermVC)

        root = root.splitting(activePane, with: .leaf(newTermVC), orientation: orientation)

        rebuildViewHierarchy()

        setActive(newTermVC)
        newTermVC.startShellIfNeeded()
        newTermVC.view.window?.makeFirstResponder(newTermVC.view)
    }

    func closeActive() {
        let closing = activePane
        guard let newRoot = root.removing(closing) else {
            closing.view.removeFromSuperview()
            closing.removeFromParent()
            onLastPaneClosed?()
            return
        }

        root = newRoot
        closing.view.removeFromSuperview()
        closing.removeFromParent()
        rebuildViewHierarchy()

        let next = root.allLeaves().first!
        setActive(next)
        next.view.window?.makeFirstResponder(next.view)
    }

    func setActive(_ terminal: TerminalViewController) {
        activePane = terminal
        onActivePaneChanged?()
    }

    func allTerminals() -> [TerminalViewController] {
        root.allLeaves()
    }

    func setCustomTitle(_ title: String) {
        activePane.setCustomTitle(title)
    }

    func applyBackground(path: String, tint: Double) {
        allTerminals().forEach { $0.applyBackground(path: path, tint: tint) }
    }

    func clearBackground() {
        allTerminals().forEach { $0.clearBackground() }
    }

    func setTint(_ alpha: Double) {
        allTerminals().forEach { $0.setTint(alpha) }
    }

    // MARK: - View Hierarchy

    /// Rebuilds the entire view hierarchy from the SplitNode tree.
    /// This is called on every split/close, creating fresh EqualSplitViews each time.
    private func rebuildViewHierarchy() {
        NSLayoutConstraint.deactivate(rootConstraints)
        rootConstraints = []
        containerView.subviews.forEach { $0.removeFromSuperview() }

        let rootView = buildView(from: root)
        rootView.translatesAutoresizingMaskIntoConstraints = false
        containerView.addSubview(rootView)

        let constraints = [
            rootView.topAnchor.constraint(equalTo: containerView.topAnchor),
            rootView.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
            rootView.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
            rootView.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
        ]
        NSLayoutConstraint.activate(constraints)
        rootConstraints = constraints
    }

    /// Recursively builds the view tree. NSSplitView manages subview sizing,
    /// so direct children use translatesAutoresizingMaskIntoConstraints = true.
    private func buildView(from node: SplitNode) -> NSView {
        switch node {
        case let .leaf(vc):
            vc.view.translatesAutoresizingMaskIntoConstraints = true
            vc.view.autoresizingMask = [.width, .height]
            return vc.view

        case let .branch(orientation, children):
            let sv = EqualSplitView()
            sv.isVertical = (orientation == .horizontal)
            sv.dividerStyle = .thin
            for child in children {
                sv.addSubview(buildView(from: child))
            }
            return sv
        }
    }

    // MARK: - Focus Monitor

    private func installClickMonitor() {
        clickMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            guard let self else { return event }
            let leaves = root.allLeaves()
            guard leaves.count > 1 else { return event }
            for termVC in leaves {
                let view = termVC.view
                let locationInView = view.convert(event.locationInWindow, from: nil)
                if view.bounds.contains(locationInView) {
                    setActive(termVC)
                    break
                }
            }
            return event
        }
    }

    // MARK: - Terminal Wiring

    private func wireTerminal(_ termVC: TerminalViewController) {
        termVC.onProcessTerminated = { [weak self, weak termVC] in
            guard let self, let termVC else { return }
            if termVC === activePane {
                closeActive()
            } else {
                guard let newRoot = root.removing(termVC) else {
                    onLastPaneClosed?(); return
                }
                termVC.view.removeFromSuperview()
                termVC.removeFromParent()
                root = newRoot
                rebuildViewHierarchy()
            }
        }
    }
}
