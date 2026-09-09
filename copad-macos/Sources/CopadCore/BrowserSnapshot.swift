import Foundation

// MARK: - Mirror types

/// Swift mirror of `copad_core::browser::tabs::BrowserPaneSnap`.
///
/// The Rust side is the schema owner; this is a hand-written mirror in the same
/// spirit as `Session.swift`, and it lives in `CopadCore` rather than the
/// executable so the test bundle can reach it (`Package.swift`: "new pure-logic
/// types should land here").
///
/// **A schema change updates the shared fixture in the same commit.** The
/// round-trip test below pins today's schema on both sides, but it cannot catch
/// a future Rust field that is `#[serde(default)]` and omitted when empty — such
/// a field leaves the fixture's output unchanged while Swift silently drops it
/// on every save. The fixture rule is what closes that gap; there is no
/// automatic guard for it.
public struct BrowserPaneSnap: Codable, Equatable, Sendable {
    public var tabs: [BrowserTabSnap]
    public var active: Int
    public var profile: String

    public init(tabs: [BrowserTabSnap], active: Int = 0, profile: String = BrowserPaneSnap.defaultProfile) {
        self.tabs = tabs
        self.active = active
        self.profile = profile
    }

    /// Mirrors `copad_core::browser::profile::DEFAULT_PROFILE`. Maps onto the
    /// platform's PRE-EXISTING data store — never a freshly identified one, or
    /// every existing user is signed out of every site.
    public static let defaultProfile = "default"

    private enum CodingKeys: String, CodingKey {
        case tabs, active, profile
    }

    /// Hand-written because synthesized decoding would make `active` and
    /// `profile` REQUIRED, while Rust marks both `#[serde(default)]`.
    ///
    /// The mismatch is not theoretical: a pane written by any other producer of
    /// this schema — a hand-edited file, a future Rust version that omits a
    /// default, a normalizer response — would fail to decode here, `Session`
    /// would discard the whole pane, and restore would mint a NEW tab id,
    /// silently losing the tab's identity and its history blob with it.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        tabs = try c.decodeIfPresent([BrowserTabSnap].self, forKey: .tabs) ?? []
        active = try c.decodeIfPresent(Int.self, forKey: .active) ?? 0
        profile = try c.decodeIfPresent(String.self, forKey: .profile)
            ?? BrowserPaneSnap.defaultProfile
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(tabs, forKey: .tabs)
        try c.encode(active, forKey: .active)
        try c.encode(profile, forKey: .profile)
    }
}

/// Swift mirror of `copad_core::browser::tabs::BrowserTabSnap`.
public struct BrowserTabSnap: Codable, Equatable, Sendable {
    public var id: String
    public var url: String
    public var title: String
    public var pinned: Bool
    public var historyGeneration: UInt64?
    public var historyDepth: Int?
    public var scrollY: Double?
    public var lastActive: UInt64

    public init(
        id: String,
        url: String,
        title: String = "",
        pinned: Bool = false,
        historyGeneration: UInt64? = nil,
        historyDepth: Int? = nil,
        scrollY: Double? = nil,
        lastActive: UInt64 = 0,
    ) {
        self.id = id
        self.url = url
        self.title = title
        self.pinned = pinned
        self.historyGeneration = historyGeneration
        self.historyDepth = historyDepth
        self.scrollY = scrollY
        self.lastActive = lastActive
    }

    private enum CodingKeys: String, CodingKey {
        case id, url, title, pinned
        case historyGeneration = "history_generation"
        case historyDepth = "history_depth"
        case scrollY = "scroll_y"
        case lastActive = "last_active"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        url = try c.decode(String.self, forKey: .url)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        pinned = try c.decodeIfPresent(Bool.self, forKey: .pinned) ?? false
        historyGeneration = try c.decodeIfPresent(UInt64.self, forKey: .historyGeneration)
        historyDepth = try c.decodeIfPresent(Int.self, forKey: .historyDepth)
        scrollY = try c.decodeIfPresent(Double.self, forKey: .scrollY)
        lastActive = try c.decodeIfPresent(UInt64.self, forKey: .lastActive) ?? 0
    }

    /// Omissions mirror serde's `skip_serializing_if` exactly, so a pane with no
    /// browser state serializes byte-identically to what the pre-Workbench
    /// binary wrote — which is what lets an older binary keep reading the file.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(url, forKey: .url)
        if !title.isEmpty { try c.encode(title, forKey: .title) }
        if pinned { try c.encode(pinned, forKey: .pinned) }
        try c.encodeIfPresent(historyGeneration, forKey: .historyGeneration)
        try c.encodeIfPresent(historyDepth, forKey: .historyDepth)
        try c.encodeIfPresent(scrollY, forKey: .scrollY)
        try c.encode(lastActive, forKey: .lastActive)
    }
}

// MARK: - Snapshot lifecycle

/// How a browser pane turns itself back into a persistable snapshot.
///
/// The macOS save path does not keep the document it loaded — `PaneManager`
/// rebuilds `PaneContent` from the **live panel** on every autosave. A restored
/// pane therefore has to regenerate its snapshot rather than carry one, or the
/// carried copy goes stale the moment the user navigates.
public enum BrowserSnapshot {
    /// Which URL a snapshot should record.
    ///
    /// **Pending wins over live.** A pending URL exists only between a
    /// navigation being *asked for* and the main frame *committing* it, and the
    /// caller clears it on commit — so whenever one is set, it is the newer
    /// intent by construction. Two failures follow from getting this backwards:
    ///
    /// - `WKWebView.url` is nil until WebKit has one, so autosave firing while a
    ///   restored pane is still loading would persist `""` and erase the pane
    ///   before it ever opened. (This happened: a session.json in the wild held
    ///   `{"kind":"webview","url":""}` for a pane the user had open.)
    /// - After loading page A and navigating to an unreachable B, WebKit keeps
    ///   reporting A. Preferring live would persist A and silently discard the
    ///   navigation the user actually asked for.
    ///
    /// A pane whose load FAILS therefore also survives to be retried next
    /// launch, because nothing ever clears its pending URL.
    public static func resolveURL(live: String?, pending: String?) -> String {
        if let pending, !pending.isEmpty {
            return pending
        }
        if let live, !live.isEmpty, live != "about:blank" {
            return live
        }
        return ""
    }

    /// Build the single-tab snapshot a pane persists before the tab strip
    /// exists (work unit B3 makes this N tabs). `tabID` is reused across
    /// restarts so a tab keeps its identity — and therefore its future history
    /// blob — rather than being reissued on every launch.
    public static func pane(
        tabID: String,
        url: String,
        title: String = "",
        profile: String = BrowserPaneSnap.defaultProfile,
    ) -> BrowserPaneSnap {
        BrowserPaneSnap(
            tabs: [BrowserTabSnap(id: tabID, url: url, title: title)],
            active: 0,
            profile: profile,
        )
    }

    /// A fresh id in the accepted charset.

    // NOTE: there is deliberately no `isValidTabID` here. Tab-id validation is
    // the rule that keeps a session document from becoming a filesystem path,
    // and it is enforced once, in Rust — a restored pane reaches a controller
    // only through `BrowserFFI.normalize`, which has already dropped every id
    // that fails it. A second validator in Swift would be exactly the duplicated
    // security rule this whole layer exists to remove (it is how
    // `canonicalWebviewURL` drifted). `UUID().uuidString` is hex + dashes,
    /// so it already qualifies; lowercased for tidiness only.
    public static func freshTabID() -> String {
        UUID().uuidString.lowercased()
    }
}

// MARK: - FFI response decoding

/// Whether a browser RPC may run, and whether its result must be replaced.
public struct BrowserAuthorization: Equatable, Sendable {
    public let allowed: Bool
    public let opaqueWrite: Bool
    public let code: String?
    public let message: String?

    public init(allowed: Bool, opaqueWrite: Bool, code: String? = nil, message: String? = nil) {
        self.allowed = allowed
        self.opaqueWrite = opaqueWrite
        self.code = code
        self.message = message
    }

    /// What a caller gets when the rule could not be consulted at all.
    ///
    /// A dispatcher that cannot ask whether a call is allowed must not run it —
    /// the alternative is that an FFI failure silently becomes "permit
    /// everything", which is the one outcome this whole layer exists to prevent.
    public static let refused = BrowserAuthorization(
        allowed: false,
        opaqueWrite: true,
        code: "authorization_unavailable",
        message: "browser authorization could not be evaluated",
    )
}

/// What `canonicalize` decided about a URL.
public struct BrowserCanonicalURL: Equatable, Sendable {
    public let url: String
    /// Whether the active restore policy permits persisting the page TITLE.
    ///
    /// Decided in Rust, never re-derived here by comparing the raw policy
    /// string: `"ORIGIN"`, `" origin "` and a typo all resolve to origin-only on
    /// that side, and a second interpretation in Swift got them wrong — leaking
    /// titles like "Reset password for …" under a policy the user believed was
    /// origin-only.
    public let persistTitle: Bool

    public init(url: String, persistTitle: Bool) {
        self.url = url
        self.persistTitle = persistTitle
    }

    /// Fail-closed: no URL, and no title either.
    public static let unavailable = BrowserCanonicalURL(url: "", persistTitle: false)
}

/// Decoders for the three FFI responses.
///
/// These live here, apart from the C marshalling, because the interesting part
/// is not the call — it is which value gets chosen when the call fails. That
/// choice is fail-closed in every case and is what the tests exercise.
public enum BrowserFFIDecode {
    /// `{ url, persist_title }` → the decision. Any failure yields
    /// `.unavailable` — an empty URL (restore the blank placeholder) and no
    /// title. Never the raw input: an FFI failure must not silently become
    /// "persist the whole URL, tokens and all".
    public static func canonical(_ raw: String?) -> BrowserCanonicalURL {
        guard let raw,
              let data = raw.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let url = obj["url"] as? String
        else { return .unavailable }
        return BrowserCanonicalURL(
            url: url,
            // Missing reads as "do not persist" — the restrictive direction.
            persistTitle: obj["persist_title"] as? Bool ?? false,
        )
    }

    /// `{ pane, repairs }` → the repaired pane. Any failure yields nil, and the
    /// caller falls back to single-URL restore rather than trusting an
    /// unvalidated pane.
    public static func normalized(_ raw: String?) -> BrowserPaneSnap? {
        guard let raw,
              let data = raw.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              // Must be a DICTIONARY, not merely present: `dataWithJSONObject`
              // raises an ObjC exception on a bare scalar top level, which no
              // `try?` in Swift can catch. Type-checking first is the only way
              // to keep a malformed response from crashing the app.
              let paneObj = obj["pane"] as? [String: Any],
              let paneData = try? JSONSerialization.data(withJSONObject: paneObj)
        else { return nil }
        return try? JSONDecoder().decode(BrowserPaneSnap.self, from: paneData)
    }

    /// `{ allowed, opaque_write, code?, message? }` → the decision. Any failure
    /// yields `.refused`.
    public static func authorization(_ raw: String?) -> BrowserAuthorization {
        guard let raw,
              let data = raw.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let allowed = obj["allowed"] as? Bool
        else { return .refused }
        return BrowserAuthorization(
            allowed: allowed,
            // A missing flag reads as "replace the result" — the restrictive
            // direction, matching the unknown-mode rule on the Rust side.
            opaqueWrite: obj["opaque_write"] as? Bool ?? true,
            code: obj["code"] as? String,
            message: obj["message"] as? String,
        )
    }
}
