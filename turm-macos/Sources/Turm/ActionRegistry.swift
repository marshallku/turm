import Foundation

/// Mirrors `turm-core::action_registry::ActionRegistry` for macOS — but only
/// the subset we need for PR 2 (the registry seam). Linux's full surface
/// includes `register_silent`, `register_blocking`, `invoke` / `try_invoke`,
/// completion-event fan-out via `with_completion_bus`, and Phase 14.1 chained
/// triggers. We don't need any of that yet:
///
/// - **`register_silent` deferred** — completion-event broadcast doesn't
///   exist on macOS yet (no `EventBus`-backed completion bus is wired into
///   the registry), so every registered action is effectively silent today.
///   Re-add silent vs noisy when PR 5 brings completion fan-out online.
///
/// - **`register_blocking` deferred** — codex round-2 flagged that macOS's
///   socket dispatch already pins the socket thread on a `DispatchSemaphore`
///   waiting for the main-actor completion to fire, so a long-running
///   handler blocks one client at a time but doesn't pin the main thread.
///   `register_blocking` would spawn a worker per dispatch (Linux semantics),
///   which conflicts with this model and risks deadlock once trigger-driven
///   re-entry shows up. Defer to PR 5 (TriggerEngine FFI) where we can
///   redesign the async boundary holistically.
///
/// - **`invoke` / `try_invoke` (sync) deferred** — only the trigger engine
///   needs sync invoke; we don't have one yet.
///
/// What's left is the minimum required for PR 3 (plugin host) and PR 5
/// (trigger engine) to register handlers and have them reachable from the
/// socket dispatcher: register, try-dispatch, has, names. Concurrency model
/// is `@MainActor` because every call site runs on the main thread already
/// (the socket server marshals into `DispatchQueue.main.async` before
/// invoking `commandHandler`).
@MainActor
final class ActionRegistry {
    /// Action handler. Receives the parsed `params` dict and a `completion`
    /// closure that MUST be called exactly once with either:
    ///
    /// - The success result (any JSON-serializable Swift value, typically
    ///   `[String: Any]`), OR
    /// - An `RPCError` instance to surface a JSON-RPC error envelope (same
    ///   path used by webview commands today).
    ///
    /// Calling `completion` more than once is a programmer error and will
    /// race with the socket-server semaphore signal — assertion in debug
    /// builds, undefined behavior in release. Calling it zero times will
    /// hang the calling client forever.
    typealias Handler = (_ params: [String: Any], _ completion: @escaping (Any?) -> Void) -> Void

    private var handlers: [String: Handler] = [:]

    /// Register a handler under `name`. Replaces any existing handler with
    /// the same name (last writer wins — same as Linux). Plugins that bind
    /// to a name owned by another plugin will silently overwrite; the
    /// supervisor's `resolve_provides` step on Linux is where conflict
    /// detection lives. We don't have a supervisor yet on macOS, so today
    /// only first-party `system.*` actions register here.
    func register(_ name: String, handler: @escaping Handler) {
        handlers[name] = handler
    }

    /// Try to dispatch `method`. If a handler is registered, call it with
    /// `params` and `completion`, and return `true`. If not registered,
    /// return `false` WITHOUT touching `completion` — the caller owns the
    /// fall-through path (typically `AppDelegate.handleCommand`'s legacy
    /// switch). Mirrors Linux `try_dispatch`'s bool semantics so the call
    /// site can compose the same way:
    ///
    /// ```swift
    /// if registry.tryDispatch(method, params: params, completion: completion) { return }
    /// // … fall through to hardcoded handlers …
    /// ```
    @discardableResult
    func tryDispatch(
        _ method: String,
        params: [String: Any],
        completion: @escaping (Any?) -> Void,
    ) -> Bool {
        guard let handler = handlers[method] else { return false }
        handler(params, completion)
        return true
    }

    /// True if a handler is registered under `name`. Useful for diagnostics
    /// and for `system.list_actions` introspection.
    func has(_ name: String) -> Bool {
        handlers[name] != nil
    }

    /// All registered action names, sorted alphabetically. Sort is stable
    /// across calls so consumers (e.g. `turmctl call system.list_actions`)
    /// can diff successive snapshots without re-sorting.
    func names() -> [String] {
        handlers.keys.sorted()
    }

    var count: Int {
        handlers.count
    }
}
