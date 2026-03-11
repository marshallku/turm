import AppKit
import SwiftTerm

extension Notification.Name {
    static let terminalTitleChanged = Notification.Name("TurmTerminalTitleChanged")
}

@MainActor
class TerminalViewController: NSViewController {
    private let config: TurmConfig
    private let theme: TurmTheme
    private var terminalView: LocalProcessTerminalView?
    private var currentFontSize: CGFloat

    private(set) var currentTitle: String = "Terminal"
    private var shellStarted = false
    var onProcessTerminated: (() -> Void)?

    init(config: TurmConfig, theme: TurmTheme) {
        self.config = config
        self.theme = theme
        currentFontSize = CGFloat(config.fontSize)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func loadView() {
        let tv = LocalProcessTerminalView(frame: NSRect(x: 0, y: 0, width: 1200, height: 800))
        configureColors(tv)
        configureFont(tv, size: currentFontSize)
        tv.processDelegate = self
        terminalView = tv
        view = tv
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // Shell is started explicitly by TabViewController via startShellIfNeeded(),
        // after contentArea.layoutSubtreeIfNeeded() ensures the correct frame.
    }

    /// Called by TabViewController after the view has been added to the hierarchy
    /// and Auto Layout has been forced to resolve (layoutSubtreeIfNeeded).
    func startShellIfNeeded() {
        guard !shellStarted else { return }
        shellStarted = true
        startShell()
    }

    // MARK: - Configuration

    private func configureColors(_ tv: LocalProcessTerminalView) {
        tv.nativeBackgroundColor = theme.background.nsColor
        tv.nativeForegroundColor = theme.foreground.nsColor

        let ansiColors = theme.palette.map { c in
            SwiftTerm.Color(red: UInt16(c.r) * 257, green: UInt16(c.g) * 257, blue: UInt16(c.b) * 257)
        }
        tv.installColors(ansiColors)
    }

    private func configureFont(_ tv: LocalProcessTerminalView, size: CGFloat) {
        if let font = NSFont(name: config.fontFamily, size: size) {
            tv.font = font
        } else {
            tv.font = NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
        }
    }

    // MARK: - Shell

    private func startShell() {
        guard let tv = terminalView else { return }
        let pid = ProcessInfo.processInfo.processIdentifier
        let socketPath = "/tmp/turm-\(pid).sock"

        // Inherit current environment, then append/override our vars
        var env = ProcessInfo.processInfo.environment.map { "\($0.key)=\($0.value)" }
        env.append("TERM=xterm-256color")
        env.append("COLORTERM=truecolor")
        env.append("TURM_SOCKET=\(socketPath)")

        tv.startProcess(executable: config.shell, args: [], environment: env, execName: nil)
    }

    // MARK: - Socket Commands (called on main thread by SocketServer)

    /// Send a command + newline to the PTY (terminal.exec)
    func execCommand(_ command: String) {
        terminalView?.send(txt: command + "\n")
    }

    /// Send raw text to the PTY (terminal.feed)
    func feedText(_ text: String) {
        terminalView?.send(txt: text)
    }

    /// Return terminal state: cols, rows, cursor [row, col], title (terminal.state)
    func terminalState() -> [String: Any] {
        guard let tv = terminalView else { return [:] }
        let term = tv.terminal!
        let cursor = term.getCursorLocation()
        return [
            "cols": term.cols,
            "rows": term.rows,
            "cursor": [cursor.y, cursor.x],
            "title": view.window?.title ?? "turm",
        ]
    }

    /// Return visible screen text (terminal.read)
    func readScreen() -> [String: Any] {
        guard let tv = terminalView else { return [:] }
        let term = tv.terminal!
        var lines: [String] = []
        for row in 0 ..< term.rows {
            guard let line = term.getLine(row: row) else {
                lines.append(String(repeating: " ", count: term.cols))
                continue
            }
            var str = ""
            for col in 0 ..< term.cols {
                let ch = line[col].getCharacter()
                str.append(ch == "\0" ? " " : ch)
            }
            lines.append(str)
        }
        let cursor = term.getCursorLocation()
        return [
            "text": lines.joined(separator: "\n"),
            "cursor": [cursor.y, cursor.x],
            "rows": term.rows,
            "cols": term.cols,
        ]
    }

    // MARK: - Zoom

    func zoomIn() {
        let newSize = min(currentFontSize + 1, 72)
        setFontSize(newSize)
    }

    func zoomOut() {
        let newSize = max(currentFontSize - 1, 6)
        setFontSize(newSize)
    }

    func zoomReset() {
        setFontSize(CGFloat(config.fontSize))
    }

    private func setFontSize(_ size: CGFloat) {
        currentFontSize = size
        guard let tv = terminalView else { return }
        configureFont(tv, size: size)
    }
}

// MARK: - LocalProcessTerminalViewDelegate

extension TerminalViewController: LocalProcessTerminalViewDelegate {
    nonisolated func sizeChanged(source _: LocalProcessTerminalView, newCols _: Int, newRows _: Int) {
        // No-op: terminal handles resize internally
    }

    nonisolated func setTerminalTitle(source _: LocalProcessTerminalView, title: String) {
        Task { @MainActor in
            self.currentTitle = title.isEmpty ? "Terminal" : title
            NotificationCenter.default.post(name: .terminalTitleChanged, object: self)
        }
    }

    nonisolated func processTerminated(source _: TerminalView, exitCode _: Int32?) {
        Task { @MainActor in
            if let cb = self.onProcessTerminated {
                cb()
            } else {
                self.view.window?.close()
            }
        }
    }

    nonisolated func hostCurrentDirectoryUpdate(source _: TerminalView, directory _: String?) {
        // No-op: CWD tracking via OSC 7 (future: emit event)
    }
}
