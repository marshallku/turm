import Foundation
import TOMLKit

/// Policy for OSC 52 clipboard writes from the PTY.
///
/// Background: SwiftTerm's `LocalProcessTerminalView` writes to `NSPasteboard.general`
/// unconditionally on OSC 52. That lets any program in the terminal silently overwrite
/// the user's clipboard. We intercept by replacing `terminalDelegate` with a proxy
/// that consults this policy. Default is `deny`; matches VTE's hardened default on
/// Linux (VTE has OSC 52 disabled unless explicitly opted in).
enum OSC52Policy: String, Decodable {
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
    /// PR 5c — raw `[[triggers]]` array from config.toml, walked from the
    /// TOMLKit table tree into JSON-friendly `[[String: Any]]` so it can be
    /// JSON-encoded and shipped to the Rust trigger engine via FFI. We don't
    /// type each trigger statically because the schema allows arbitrary
    /// nested values under `params` / `when.payload_match` / `await.payload_match`,
    /// and the Rust side already has the canonical Deserialize impl.
    let triggers: [[String: Any]]

    static func load() -> TurmConfig {
        let configURL = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config")
            .appendingPathComponent("turm")
            .appendingPathComponent("config.toml")

        guard let contents = try? String(contentsOf: configURL, encoding: .utf8) else {
            return .defaults
        }
        return parse(contents)
    }

    /// Decode a TOML config string into TurmConfig. Falls back to `.defaults` if the
    /// document is malformed; the parse error is written to stderr so the user can
    /// fix it. Unknown sections (e.g. `[[triggers]]`, `[keybindings]`, `[statusbar]`
    /// from the Linux schema) are tolerated — we only decode the fields the macOS
    /// app currently uses, and the rest stay intact for future parity work.
    static func parse(_ contents: String) -> TurmConfig {
        let decoder = TOMLDecoder()
        let raw: RawConfig
        do {
            raw = try decoder.decode(RawConfig.self, from: contents)
        } catch {
            let msg = "[turm] config.toml parse failed: \(error.localizedDescription) — using defaults\n"
            FileHandle.standardError.write(Data(msg.utf8))
            return .defaults
        }

        let defaults = TurmConfig.defaults
        let bgImage = raw.background?.path ?? raw.background?.image
        let bgPath: String? = if let bgImage, !bgImage.isEmpty { expandTilde(bgImage) } else { nil }

        return TurmConfig(
            shell: raw.terminal?.shell ?? defaults.shell,
            fontFamily: raw.terminal?.fontFamily ?? defaults.fontFamily,
            fontSize: raw.terminal?.fontSize ?? defaults.fontSize,
            themeName: raw.theme?.name ?? defaults.themeName,
            backgroundPath: bgPath,
            backgroundTint: clamp01(raw.background?.tint ?? defaults.backgroundTint),
            backgroundOpacity: clamp01(raw.background?.opacity ?? defaults.backgroundOpacity),
            osc52: raw.security?.osc52 ?? defaults.osc52,
            triggers: parseTriggersArray(from: contents),
        )
    }

    /// JSON-friendly trigger list ready to ship through `TurmEngine.setTriggers`.
    /// Just exposes the parsed `[[triggers]]` array; kept as a static helper
    /// (rather than an instance method) so AppDelegate doesn't have to know
    /// the encoding rules.
    static func triggersJSON(from config: TurmConfig) -> [[String: Any]] {
        config.triggers
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
            triggers: [],
        )
    }

    private static func clamp01(_ d: Double) -> Double {
        max(0, min(1, d))
    }

    private static func expandTilde(_ path: String) -> String {
        guard path.hasPrefix("~") else { return path }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return home + path.dropFirst()
    }

    /// Walk the TOML `[[triggers]]` array into JSON-friendly `[[String: Any]]`.
    /// We can't use a plain Decodable struct because trigger entries contain
    /// arbitrary nested values (`params`, `payload_match`) that we don't want
    /// to type statically — Rust's `serde_json::Value` round-trips the same
    /// tree losslessly. Walks via `TOMLTable` opaque API so the values flow
    /// straight into `JSONSerialization`-compatible types.
    private static func parseTriggersArray(from contents: String) -> [[String: Any]] {
        guard let table = try? TOMLTable(string: contents),
              let arr = table["triggers"]?.array
        else {
            return []
        }
        var result: [[String: Any]] = []
        for value in arr {
            if let dict = tomlValueToDict(value) {
                result.append(dict)
            }
        }
        return result
    }

    private static func tomlValueToDict(_ v: TOMLValueConvertible) -> [String: Any]? {
        guard let table = v.table else { return nil }
        var dict: [String: Any] = [:]
        for key in table.keys {
            if let val = table[key], let any = tomlValueToAny(val) {
                dict[key] = any
            }
        }
        return dict
    }

    private static func tomlValueToAny(_ v: TOMLValueConvertible) -> Any? {
        // Order matters: check leaf types before composites because TOMLValue
        // may report multiple accessors as non-nil for ambiguous cases.
        if let s = v.string { return s }
        if let i = v.int { return i }
        if let d = v.double { return d }
        if let b = v.bool { return b }
        if let arr = v.array {
            return arr.compactMap(tomlValueToAny)
        }
        if let table = v.table {
            var d: [String: Any] = [:]
            for key in table.keys {
                if let val = table[key], let any = tomlValueToAny(val) {
                    d[key] = any
                }
            }
            return d
        }
        return nil
    }
}

// MARK: - Decodable shadow types

/// TOML shape for the macOS-relevant subset of the shared config schema. Sections
/// we don't decode yet (`[tabs]`, `[statusbar]`, `[keybindings]`, `[[triggers]]`)
/// are silently dropped — TOML decoding ignores unknown keys at the top level, so
/// users can keep their full Linux-shape config and the macOS app just picks out
/// what it understands. TOMLKit 0.6 has no `keyDecodingStrategy`, so snake_case
/// keys need explicit `CodingKeys`.
private struct RawConfig: Decodable {
    var terminal: TerminalSection?
    var theme: ThemeSection?
    var background: BackgroundSection?
    var security: SecuritySection?
}

private struct TerminalSection: Decodable {
    var shell: String?
    var fontFamily: String?
    var fontSize: Int?

    enum CodingKeys: String, CodingKey {
        case shell
        case fontFamily = "font_family"
        case fontSize = "font_size"
    }
}

private struct ThemeSection: Decodable {
    var name: String?
}

private struct BackgroundSection: Decodable {
    var path: String?
    var image: String?
    var tint: Double?
    var opacity: Double?
}

private struct SecuritySection: Decodable {
    var osc52: OSC52Policy?
}
