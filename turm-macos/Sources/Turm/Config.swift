import Foundation

/// Policy for OSC 52 clipboard writes from the PTY.
///
/// Background: SwiftTerm's `LocalProcessTerminalView` writes to `NSPasteboard.general`
/// unconditionally on OSC 52. That lets any program in the terminal silently overwrite
/// the user's clipboard. We intercept by replacing `terminalDelegate` with a proxy
/// that consults this policy. Default is `deny`; matches VTE's hardened default on
/// Linux (VTE has OSC 52 disabled unless explicitly opted in).
enum OSC52Policy: String {
    case deny
    case allow
}

struct TurmConfig {
    let shell: String
    let fontFamily: String
    let fontSize: Int
    let themeName: String
    let backgroundPath: String?
    let backgroundTint: Double
    /// Opacity of the background image layer itself (0.0 = invisible, 1.0 = fully visible).
    /// Distinct from `backgroundTint`, which darkens the image via an overlay.
    let backgroundOpacity: Double
    let osc52: OSC52Policy

    static func load() -> TurmConfig {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let configURL = home
            .appendingPathComponent(".config")
            .appendingPathComponent("turm")
            .appendingPathComponent("config.toml")

        guard let contents = try? String(contentsOf: configURL, encoding: .utf8) else {
            return TurmConfig.defaults
        }

        return TurmConfig.parse(contents)
    }

    static func parse(_ contents: String) -> TurmConfig {
        var shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        var fontFamily = "JetBrains Mono"
        var fontSize = 14
        var themeName = "catppuccin-mocha"
        var backgroundPath: String? = nil
        var backgroundTint = 0.6
        var backgroundOpacity = 1.0
        var osc52: OSC52Policy = .deny

        var currentSection = ""

        for line in contents.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed.isEmpty || trimmed.hasPrefix("#") { continue }

            if trimmed.hasPrefix("["), trimmed.hasSuffix("]") {
                currentSection = String(trimmed.dropFirst().dropLast())
                continue
            }

            guard let eqRange = trimmed.range(of: "=") else { continue }
            let key = trimmed[..<eqRange.lowerBound].trimmingCharacters(in: .whitespaces)
            var value = String(trimmed[eqRange.upperBound...]).trimmingCharacters(in: .whitespaces)

            // Strip inline comments
            if let commentRange = value.range(of: " #") {
                value = String(value[..<commentRange.lowerBound]).trimmingCharacters(in: .whitespaces)
            }

            // Strip surrounding quotes
            if value.hasPrefix("\""), value.hasSuffix("\""), value.count >= 2 {
                value = String(value.dropFirst().dropLast())
            }

            switch (currentSection, key) {
            case ("terminal", "shell"):
                shell = value
            case ("terminal", "font_family"):
                fontFamily = value
            case ("terminal", "font_size"):
                if let n = Int(value) { fontSize = n }
            case ("theme", "name"):
                themeName = value
            case ("background", "path"), ("background", "image"):
                backgroundPath = value.isEmpty ? nil : expandTilde(value)
            case ("background", "tint"):
                if let d = Double(value) { backgroundTint = max(0, min(1, d)) }
            case ("background", "opacity"):
                if let d = Double(value) { backgroundOpacity = max(0, min(1, d)) }
            case ("security", "osc52"):
                if let p = OSC52Policy(rawValue: value.lowercased()) { osc52 = p }
            default:
                break
            }
        }

        return TurmConfig(
            shell: shell, fontFamily: fontFamily, fontSize: fontSize,
            themeName: themeName, backgroundPath: backgroundPath,
            backgroundTint: backgroundTint, backgroundOpacity: backgroundOpacity,
            osc52: osc52,
        )
    }

    static var defaults: TurmConfig {
        TurmConfig(
            shell: ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh",
            fontFamily: "JetBrains Mono",
            fontSize: 14,
            themeName: "catppuccin-mocha",
            backgroundPath: nil,
            backgroundTint: 0.6,
            backgroundOpacity: 1.0,
            osc52: .deny,
        )
    }

    private static func expandTilde(_ path: String) -> String {
        guard path.hasPrefix("~") else { return path }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return home + path.dropFirst()
    }
}
