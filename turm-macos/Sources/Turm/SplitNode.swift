import AppKit

enum SplitOrientation {
    /// Vertical divider — panes side by side (Cmd+D)
    case horizontal
    /// Horizontal divider — panes stacked (Cmd+Shift+D)
    case vertical
}

/// N-ary recursive split tree for a single tab.
/// Does NOT store NSSplitView references — the view hierarchy is rebuilt from
/// scratch on every split/close operation.
indirect enum SplitNode {
    case leaf(TerminalViewController)
    case branch(SplitOrientation, [SplitNode])

    // MARK: - Leaf enumeration

    func allLeaves() -> [TerminalViewController] {
        switch self {
        case let .leaf(vc): [vc]
        case let .branch(_, children): children.flatMap { $0.allLeaves() }
        }
    }

    // MARK: - Tree mutations

    /// Replaces `terminal`'s leaf with a new two-child branch containing the
    /// original leaf and `newNode`. This always splits the focused pane's own
    /// space in half, leaving every other pane completely unchanged.
    func splitting(
        _ terminal: TerminalViewController,
        with newNode: SplitNode,
        orientation: SplitOrientation,
    ) -> SplitNode {
        switch self {
        case let .leaf(vc):
            guard vc === terminal else { return self }
            return .branch(orientation, [.leaf(vc), newNode])

        case let .branch(o, children):
            return .branch(o, children.map { $0.splitting(terminal, with: newNode, orientation: orientation) })
        }
    }

    /// Returns a new tree with `terminal` removed, or nil if this was the only leaf.
    func removing(_ terminal: TerminalViewController) -> SplitNode? {
        switch self {
        case let .leaf(vc):
            return vc === terminal ? nil : self

        case let .branch(o, children):
            let remaining = children.compactMap { $0.removing(terminal) }
            if remaining.isEmpty { return nil }
            if remaining.count == 1 { return remaining[0] } // collapse single-child branch
            return .branch(o, remaining)
        }
    }
}
