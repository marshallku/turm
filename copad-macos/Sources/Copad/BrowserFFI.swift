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
