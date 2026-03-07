import AppKit

@main
struct TurmApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }
}

class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false
        )

        window.title = "turm"
        window.center()
        window.makeKeyAndOrderFront(nil)
        window.backgroundColor = NSColor(red: 0.118, green: 0.118, blue: 0.180, alpha: 1.0) // #1e1e2e

        // TODO: Add terminal view (Phase 2)
        // Will embed a terminal emulator view here - options:
        // 1. SwiftTerm (pure Swift terminal emulator)
        // 2. Custom PTY + NSTextView rendering
        // 3. Ghostty embedding (like cmux)

        self.window = window
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
