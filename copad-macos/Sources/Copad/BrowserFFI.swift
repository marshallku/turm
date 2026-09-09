import CCopadFFI
import CopadCore
import Foundation

/// C marshalling for the three browser rules that live in `copad-core`.
///
/// Only the marshalling is here. Every *decision* — which value to fall back to
/// when a call fails — lives in `CopadCore.BrowserFFIDecode`, where the test
/// bundle can reach it (`CCopadFFI`'s linker flags are on the executable target,
/// so anything touching the C symbols cannot be unit-tested by `swift test`).
///
/// The rules are not mirrored in Swift on purpose. `PaneManager` used to carry
/// its own `canonicalWebviewURL`, and every bug the Rust implementation has
/// since fixed — a backslash smuggling a whole path into what looked like an
/// origin, a percent-encoded `%63ode=` slipping past the token denylist — was
/// still live in that copy. One implementation, two callers.
enum BrowserFFI {
    /// Apply the `[browser] restore` policy to a live URL, and learn whether the
    /// page title may be persisted alongside it.
    ///
    /// Both answers come from Rust. Re-deriving the title rule here by comparing
    /// the policy string would reintroduce the drift this layer exists to
    /// remove — `"ORIGIN"` and `" origin "` resolve to origin-only on that side.
    ///
    /// Returns `.unavailable` when the rule could not be consulted, which
    /// restores the blank URL-entry placeholder. Never the raw URL: a failed
    /// call must not silently become "persist everything, tokens included".
    static func canonicalize(_ url: String, policy: String) -> BrowserCanonicalURL {
        BrowserFFIDecode.canonical(call(
            copad_ffi_browser_canonicalize,
            ["url": url, "policy": policy],
            label: "canonicalize",
        ))
    }

    /// Repair an untrusted pane snapshot read off disk.
    /// Returns nil when the rule could not be consulted; the caller then falls
    /// back to single-URL restore rather than trusting an unvalidated pane.
    static func normalize(_ pane: BrowserPaneSnap) -> BrowserPaneSnap? {
        guard let encoded = try? JSONEncoder().encode(pane),
              let paneObj = try? JSONSerialization.jsonObject(with: encoded)
        else { return nil }
        return BrowserFFIDecode.normalized(call(
            copad_ffi_browser_normalize,
            ["pane": paneObj],
            label: "normalize",
        ))
    }

    /// May this browser RPC run against a tab in `mode`, and must its result be
    /// replaced with a page-independent one?
    ///
    /// Ask at dispatch, again before delivering a result, and again before any
    /// side effect that leaves the process (the `--path` screenshot write). A
    /// read already in flight when a tab entered `protected` has to be
    /// suppressed, and a file already written cannot be unwritten by refusing
    /// the response.
    static func authorize(
        method: String,
        mode: String,
        profileProtected: Bool,
    ) -> BrowserAuthorization {
        BrowserFFIDecode.authorization(call(
            copad_ffi_browser_authorize,
            ["method": method, "mode": mode, "profile_protected": profileProtected],
            label: "authorize",
        ))
    }

    /// Does this restore policy keep the opaque history blob?
    ///
    /// Asked of Rust rather than compared here, for the same reason
    /// `persistTitle` is: `"FULL"` and `" full "` resolve on that side, and a
    /// second reading of the raw string in Swift is exactly the drift this
    /// layer exists to remove.
    static func keepsHistory(policy: String) -> Bool {
        BrowserFFIDecode.keepsHistory(call(
            copad_ffi_browser_canonicalize,
            ["url": "", "policy": policy],
            label: "keepsHistory",
        ))
    }

    // MARK: - History blobs

    /// Persist a tab's opaque back/forward + scroll blob as a new generation.
    /// Returns false when the rule refused it, in which case the caller must
    /// NOT record the generation in the session — a reference to a blob that
    /// was never written would restore as a silent failure later.
    static func writeHistory(tabID: String, generation: UInt64, data: Data) -> Bool {
        let raw = call(
            copad_ffi_browser_history_write,
            [
                "tab_id": tabID,
                "generation": generation,
                "data_hex": data.map { String(format: "%02x", $0) }.joined(),
            ],
            label: "history_write",
        )
        return BrowserFFIDecode.wroteHistory(raw)
    }

    /// Read a blob back. `nil` covers absent, unreadable, oversize, and
    /// symlinked — all of which mean the same thing to the caller: restore the
    /// URL plainly rather than a history it cannot trust.
    static func readHistory(tabID: String, generation: UInt64) -> Data? {
        BrowserFFIDecode.historyBlob(call(
            copad_ffi_browser_history_read,
            ["tab_id": tabID, "generation": generation],
            label: "history_read",
        ))
    }

    /// Reclaim superseded generations. Called AFTER the session has committed,
    /// so the generation it references is never the one being removed.
    @discardableResult
    static func gcHistory(live: [(String, UInt64)]) -> Int {
        BrowserFFIDecode.gcRemoved(call(
            copad_ffi_browser_history_gc,
            ["live": live.map { [$0.0, $0.1] as [Any] }],
            label: "history_gc",
        ))
    }

    // MARK: - Network + console log

    /// Append one captured record. Returns false when core DROPPED it for
    /// exceeding the per-record cap — surfaced rather than swallowed so a
    /// silent under-report is at least visible in the logs.
    @discardableResult
    static func appendLog(
        panelID: String,
        kind: String,
        record: [String: Any],
        captureBodies: Bool,
    ) -> Bool {
        BrowserFFIDecode.wroteLog(call(
            copad_ffi_browser_netlog_append,
            [
                "panel_id": panelID,
                "kind": kind,
                "record": record,
                "capture_bodies": captureBodies,
            ],
            label: "netlog_append",
        ))
    }

    /// Read back matching records plus the declared capture coverage.
    static func readLog(panelID: String, query: [String: Any]) -> (records: [Any], coverage: String) {
        var payload = query
        payload["panel_id"] = panelID
        return BrowserFFIDecode.logRecords(call(
            copad_ffi_browser_netlog_read,
            payload,
            label: "netlog_read",
        ))
    }

    static func clearLog(panelID: String) -> Int {
        BrowserFFIDecode.gcRemoved(call(
            copad_ffi_browser_netlog_clear,
            ["panel_id": panelID],
            label: "netlog_clear",
        ))
    }

    /// Ask core whether this fill may proceed.
    ///
    /// Not re-implemented in Swift. These are the checks that decide whether a
    /// password is written into a page, so a second implementation is a second
    /// thing that can be subtly wrong on its own — and a first draft of this
    /// file did exactly that, checking origin and input type here while never
    /// consulting core's document-generation and profile binding at all.
    static func validateFill(
        request: [String: Any],
        credential: CredentialRef,
        live: [String: Any],
        target: [String: Any],
    ) -> RPCError? {
        guard let data = try? JSONEncoder().encode(credential),
              let credObj = try? JSONSerialization.jsonObject(with: data)
        else {
            return RPCError(code: "invalid_params", message: "credential could not be encoded")
        }
        let verdict = BrowserFFIDecode.fillVerdict(call(
            copad_ffi_browser_validate_fill,
            ["request": request, "credential": credObj, "live": live, "target": target],
            label: "validate_fill",
        ))
        return verdict.map { RPCError(code: $0.code, message: $0.message) }
    }

    // MARK: - Credential index (metadata only)

    static func listCredentials(origin: String?) -> [CredentialRef] {
        var payload: [String: Any] = [:]
        if let origin { payload["origin"] = origin }
        return BrowserFFIDecode.credentials(call(
            copad_ffi_browser_credentials_list,
            payload,
            label: "credentials_list",
        ))
    }

    static func upsertCredential(_ ref: CredentialRef) -> Bool {
        guard let data = try? JSONEncoder().encode(ref),
              let obj = try? JSONSerialization.jsonObject(with: data)
        else { return false }
        return BrowserFFIDecode.wroteLog(call(
            copad_ffi_browser_credentials_upsert,
            ["credential": obj],
            label: "credentials_upsert",
        ).map { $0.replacingOccurrences(of: "\"ok\"", with: "\"written\"") })
    }

    static func removeCredential(id: String) -> Bool {
        BrowserFFIDecode.removedCredential(call(
            copad_ffi_browser_credentials_remove,
            ["credential_id": id],
            label: "credentials_remove",
        ))
    }

    // MARK: - Private

    /// Copy-then-free, the same lifecycle `CopadFFI.callJSON` uses: the Rust
    /// allocation is turned into a Swift String and released immediately, so no
    /// caller ever holds a raw pointer.
    private static func call(
        _ fn: (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?,
        _ payload: [String: Any],
        label: String,
    ) -> String? {
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let input = String(data: data, encoding: .utf8)
        else {
            warn("\(label): could not serialize the request")
            return nil
        }
        return input.withCString { cstr -> String? in
            guard let out = fn(cstr) else {
                warn("\(label): \(CopadFFI.lastError() ?? "<no error message>")")
                return nil
            }
            defer { copad_ffi_free_string(out) }
            return String(cString: out)
        }
    }

    private static func warn(_ message: String) {
        FileHandle.standardError.write(Data("[copad-browser-ffi] \(message)\n".utf8))
    }
}
