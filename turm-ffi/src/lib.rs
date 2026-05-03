//! turm-ffi — C-ABI bridge from the shared Rust core to platform UIs that
//! can't link Rust directly (currently `turm-macos`, which is SwiftPM).
//!
//! ## Why this crate exists (PR 1 — Tier 2.1 spike)
//!
//! Before committing to wiring `TriggerEngine` / `ActionRegistry` / supervisor
//! over FFI, we need to prove the boring boundary first: cargo can produce a
//! staticlib, SwiftPM links it, a Swift call lands in Rust, JSON crosses the
//! boundary in both directions, and ownership rules don't leak. This spike
//! exposes the **smallest possible C surface** that demonstrates each of those
//! concerns in isolation, so when something breaks we know whether it's the
//! build wiring, the link, the calling convention, or the data marshalling.
//!
//! Surface (4 symbols):
//!
//! - `turm_ffi_version() -> *const c_char` — points at a static `'static`
//!   string. Caller must NOT free. Proves: lib loads, basic call works,
//!   pointer-back-to-Rust-static is sound, no allocation involved.
//!
//! - `turm_ffi_call_json(input: *const c_char) -> *mut c_char` — accepts a
//!   borrowed JSON string, parses it, attaches `{"echoed_at": <unix epoch
//!   ms>}`, and returns a heap-allocated JSON string the caller owns. Proves:
//!   bidirectional JSON marshalling works, Rust-side allocation that the
//!   Swift side later releases works, error paths produce structured errors
//!   instead of panicking across FFI.
//!
//! - `turm_ffi_free_string(*mut c_char)` — releases a string previously
//!   returned by this crate. Required because Swift's ARC can't free Rust's
//!   heap. Proves: ownership round-trip closes cleanly without leaks.
//!
//! - `turm_ffi_last_error() -> *const c_char` — returns the most recent
//!   error message captured by this thread (or NULL if none). Proves:
//!   thread-local error reporting works without a return-by-pointer pattern
//!   that Swift would have to construct C buffers for.
//!
//! Anything beyond this set is for follow-up PRs (PR 2+: registry seam, then
//! supervisor, then trigger engine). Keep this file boring on purpose.
//!
//! ## PR 5c additions — engine surface
//!
//! Wraps `turm_core::trigger::TriggerEngine` so the macOS Swift host can
//! load triggers, dispatch events, and receive action-fire callbacks
//! without reimplementing the engine semantics in Swift. The engine
//! itself stays in Rust (single source of truth across Linux + macOS);
//! this module is just the C-ABI bridge.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use turm_core::action_registry::ActionResult;
use turm_core::event_bus::Event;
use turm_core::protocol::ResponseError;
use turm_core::trigger::{Trigger, TriggerEngine, TriggerSink};

thread_local! {
    /// Per-thread last-error slot. Cleared by every successful FFI call.
    /// Threading model note: every entry point writes to this slot before
    /// returning either an error or a success value, so a Swift caller that
    /// got NULL/error can pick up the message without needing a side channel.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error<S: Into<String>>(message: S) {
    let cs = CString::new(message.into()).unwrap_or_else(|_| {
        // Fallback for the (impossible) case where the message contains an
        // interior NUL. Don't lose the failure signal entirely.
        CString::new("FFI error message contained a NUL byte").unwrap()
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(cs));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Returns a pointer to a static, NUL-terminated version string. Caller must
/// NOT free. The string lives for the program's lifetime.
///
/// # Safety
///
/// The returned pointer is always non-null and valid for as long as the
/// process lives. Reading past the NUL terminator is UB.
#[unsafe(no_mangle)]
pub extern "C" fn turm_ffi_version() -> *const c_char {
    // Static C string, no allocation. `c"..."` literal is a Rust 2021+ feature
    // that produces a `&'static CStr`, so .as_ptr() is good for the program
    // lifetime.
    c"turm-ffi 0.1.0".as_ptr()
}

/// Accepts a borrowed JSON string and returns a heap-allocated JSON string
/// that the caller MUST release with `turm_ffi_free_string`.
///
/// On the success path the returned JSON contains the input plus an
/// `echoed_at` Unix-epoch-millis field, so a Swift caller can prove the
/// round-trip with a value that's both Rust-generated AND not constant.
///
/// On the error path returns NULL and stores a human-readable message in
/// `LAST_ERROR` retrievable via `turm_ffi_last_error`.
///
/// # Safety
///
/// `input` must be a valid pointer to a NUL-terminated UTF-8 string. The
/// pointer must remain valid for the duration of this call. The returned
/// pointer (when non-null) must be passed to `turm_ffi_free_string` exactly
/// once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_ffi_call_json(input: *const c_char) -> *mut c_char {
    if input.is_null() {
        set_last_error("turm_ffi_call_json: input pointer is NULL");
        return ptr::null_mut();
    }

    // SAFETY: caller contract requires `input` to be NUL-terminated UTF-8.
    let input_bytes = unsafe { CStr::from_ptr(input) }.to_bytes();
    let input_str = match std::str::from_utf8(input_bytes) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("turm_ffi_call_json: input is not valid UTF-8: {e}"));
            return ptr::null_mut();
        }
    };

    let mut parsed: Value = match serde_json::from_str(input_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(format!("turm_ffi_call_json: input is not valid JSON: {e}"));
            return ptr::null_mut();
        }
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    if let Value::Object(ref mut map) = parsed {
        map.insert("echoed_at".into(), json!(now_ms));
    } else {
        // Non-object input is allowed but loses the echo metadata; wrap it
        // so the response shape is always an object.
        parsed = json!({ "input": parsed, "echoed_at": now_ms });
    }

    let serialized = match serde_json::to_string(&parsed) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("turm_ffi_call_json: serialization failed: {e}"));
            return ptr::null_mut();
        }
    };

    let cs = match CString::new(serialized) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(format!(
                "turm_ffi_call_json: serialized JSON contained NUL byte: {e}"
            ));
            return ptr::null_mut();
        }
    };

    clear_last_error();
    cs.into_raw()
}

/// Releases a string previously returned by a turm-ffi function that
/// allocates (currently `turm_ffi_call_json`).
///
/// # Safety
///
/// `s` must be a pointer previously returned by a turm-ffi function and
/// not yet freed, OR a null pointer (in which case this is a no-op).
/// Passing any other pointer is UB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_ffi_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract requires `s` to come from a previous turm-ffi
    // CString::into_raw call. Reconstructing the CString hands ownership back
    // to Rust which then drops it.
    let _ = unsafe { CString::from_raw(s) };
}

/// Returns the most recent error message recorded on the calling thread,
/// or NULL if no error has been recorded since the last successful call.
///
/// # Safety
///
/// The returned pointer is borrowed from a thread-local slot and remains
/// valid only until the next FFI call on the same thread. Callers that
/// need to retain the message must copy it (e.g. Swift `String(cString:)`).
/// The pointer must NOT be passed to `turm_ffi_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn turm_ffi_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(cs) => cs.as_ptr(),
        None => ptr::null(),
    })
}

// ============================================================================
// PR 5c — Engine FFI surface
// ============================================================================

/// Opaque handle wrapping `Arc<TriggerEngine>` plus the C-side action
/// callback. Kept at module level (not behind `LAST_ERROR`-style thread
/// locals) so the Swift host can hold a single engine instance for the
/// lifetime of the app and serialize calls into it through its own
/// `DispatchQueue`. The struct is `pub` for the FFI but its body is
/// opaque from C — callers only ever see `*mut EngineHandle`.
pub struct EngineHandle {
    engine: Arc<TriggerEngine>,
    /// We keep the FfiSink Arc here so it shares lifetime with the
    /// engine — engine internally also holds an Arc to the same sink
    /// via `Arc<dyn TriggerSink>` (cloned at `TriggerEngine::new`),
    /// but holding our own ref makes "callback is set" testable.
    _sink: Arc<FfiSink>,
}

/// `TriggerSink` impl that forwards action dispatch into a C function
/// pointer registered by the Swift host. Fire-and-forget — we don't
/// wait for Swift's ActionRegistry to actually run the action. This
/// matches Linux's `LiveTriggerSink` shape (returns `{queued: true}`
/// immediately; real result arrives async or via completion-event
/// fan-out later). For PR 5c the spike doesn't exercise the await
/// primitive, so the placeholder result is enough.
struct FfiSink {
    /// Atomic-pointer-like cell so the callback can be installed AFTER
    /// `turm_engine_create`. Stored as `usize` (not `AtomicPtr`) because
    /// the C function-pointer type is fixed at compile time and we
    /// only need swap-on-set, not lock-free updates.
    callback: std::sync::Mutex<Option<ActionCallback>>,
    /// Swift-owned context pointer (typically `Unmanaged<TurmEngine>.toOpaque()`).
    /// Stored as `usize` so the struct stays `Send + Sync` automatically;
    /// we cast back to `*mut c_void` only at invocation time. Lifetime
    /// is the host's responsibility — the host must keep its receiver
    /// alive at least until `turm_engine_destroy` returns.
    user_data: std::sync::Mutex<usize>,
}

/// C-callable signature the Swift host registers via
/// `turm_engine_set_action_callback`. The engine calls this on
/// whatever thread `turm_engine_dispatch_event` was invoked from
/// (Swift's serial DispatchQueue today). The callback receives
/// borrowed strings — must NOT free them. To bridge into a Swift
/// closure the host typically copies via `String(cString:)` and
/// re-dispatches to the main actor.
pub type ActionCallback = unsafe extern "C" fn(
    user_data: *mut c_void,
    action_name: *const c_char,
    params_json: *const c_char,
);

impl TriggerSink for FfiSink {
    fn dispatch_action(&self, action: &str, params: Value) -> ActionResult {
        let cb_opt = *self.callback.lock().unwrap();
        let user = *self.user_data.lock().unwrap();
        let Some(cb) = cb_opt else {
            // No callback registered yet — log and treat as "no sink available"
            // so the engine doesn't keep retrying. Returning an Err here would
            // be cleaner but ActionResult's Err type is ResponseError which
            // requires a code/message — `{queued:false, reason:"no callback"}`
            // in Ok keeps the engine moving without polluting the error path.
            eprintln!("[turm-ffi] dispatch_action({action}) but no Swift callback registered");
            return Ok(json!({ "queued": false, "reason": "no callback registered" }));
        };
        // Hand-rolled CString ladder. CString::new fails on NUL bytes;
        // for action names that's defensive (action keys are well-formed),
        // for params it's the caller's problem if their JSON contains NULs.
        let action_cstr = match CString::new(action) {
            Ok(c) => c,
            Err(_) => {
                return Err(ResponseError {
                    code: "ffi_error".into(),
                    message: format!("action name {action:?} contained NUL byte"),
                });
            }
        };
        let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string());
        let params_cstr = match CString::new(params_str) {
            Ok(c) => c,
            Err(_) => {
                return Err(ResponseError {
                    code: "ffi_error".into(),
                    message: "params JSON contained NUL byte".into(),
                });
            }
        };
        // SAFETY: callback is a function pointer the host registered;
        // user_data is the host-owned pointer the host promised to keep
        // alive until destroy. Both the action and params CStrings live
        // until end-of-function.
        unsafe {
            cb(
                user as *mut c_void,
                action_cstr.as_ptr(),
                params_cstr.as_ptr(),
            );
        }
        Ok(json!({ "queued": true }))
    }
}

/// Construct a fresh trigger engine + FfiSink. No triggers loaded yet —
/// host calls `turm_engine_set_triggers` next. No callback registered —
/// host calls `turm_engine_set_action_callback` next.
///
/// # Safety
///
/// The returned pointer must be passed to `turm_engine_destroy` exactly
/// once, after no concurrent FFI call into the engine is still in flight.
#[unsafe(no_mangle)]
pub extern "C" fn turm_engine_create() -> *mut EngineHandle {
    let sink = Arc::new(FfiSink {
        callback: std::sync::Mutex::new(None),
        user_data: std::sync::Mutex::new(0),
    });
    let engine = Arc::new(TriggerEngine::new(sink.clone()));
    let handle = Box::new(EngineHandle {
        engine,
        _sink: sink,
    });
    Box::into_raw(handle)
}

/// Free the engine handle. After this call any further FFI use of the
/// pointer is UB.
///
/// # Safety
///
/// Pointer must come from `turm_engine_create` and not have been freed
/// already. Caller must ensure no other thread is mid-call into the
/// engine when destroy runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_engine_destroy(handle: *mut EngineHandle) {
    if handle.is_null() {
        return;
    }
    // SAFETY: caller contract guarantees `handle` came from `Box::into_raw`
    // in `turm_engine_create` and hasn't been freed.
    let _ = unsafe { Box::from_raw(handle) };
}

/// Install or replace the action callback. NULL `callback` clears the
/// slot (engine reverts to "no callback registered" log + skip).
///
/// # Safety
///
/// `handle` must be a valid pointer from `turm_engine_create`. `user_data`
/// must remain alive until either (a) replaced by a subsequent
/// `set_action_callback` call, or (b) `turm_engine_destroy` returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_engine_set_action_callback(
    handle: *mut EngineHandle,
    callback: Option<ActionCallback>,
    user_data: *mut c_void,
) {
    if handle.is_null() {
        return;
    }
    // SAFETY: caller contract.
    let h = unsafe { &*handle };
    *h._sink.callback.lock().unwrap() = callback;
    *h._sink.user_data.lock().unwrap() = user_data as usize;
}

/// Parse a JSON array of triggers and replace the engine's trigger set.
/// Returns the number of triggers loaded on success, -1 on JSON parse
/// failure (use `turm_ffi_last_error` for the message).
///
/// JSON shape mirrors the TOML `[[triggers]]` table — each element
/// matches `turm_core::trigger::Trigger`'s Deserialize impl. Hot-reload
/// just calls this again with the new array; the engine atomically
/// swaps the trigger list and drops any in-flight await state.
///
/// # Safety
///
/// `handle` must be a valid engine pointer. `triggers_json` must be a
/// NUL-terminated UTF-8 string. Both must remain valid for the duration
/// of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_engine_set_triggers(
    handle: *mut EngineHandle,
    triggers_json: *const c_char,
) -> i32 {
    if handle.is_null() || triggers_json.is_null() {
        set_last_error("turm_engine_set_triggers: NULL pointer");
        return -1;
    }
    // SAFETY: caller contract.
    let h = unsafe { &*handle };
    let json_str = unsafe { CStr::from_ptr(triggers_json) }.to_string_lossy();
    let triggers: Vec<Trigger> = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(format!("turm_engine_set_triggers: JSON parse error: {e}"));
            return -1;
        }
    };
    let count = triggers.len() as i32;
    h.engine.set_triggers(triggers);
    clear_last_error();
    count
}

/// Dispatch an event into the engine. Engine matches against loaded
/// triggers and fires the C action callback for each match. Returns
/// the number of triggers that fired.
///
/// # Safety
///
/// `handle` must be a valid engine pointer. `event_kind` and `payload_json`
/// must be NUL-terminated UTF-8 (payload may be NULL → defaults to `null`).
/// All pointers must outlive the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_engine_dispatch_event(
    handle: *mut EngineHandle,
    event_kind: *const c_char,
    payload_json: *const c_char,
) -> i32 {
    if handle.is_null() || event_kind.is_null() {
        set_last_error("turm_engine_dispatch_event: NULL pointer");
        return -1;
    }
    // SAFETY: caller contract.
    let h = unsafe { &*handle };
    let kind = unsafe { CStr::from_ptr(event_kind) }
        .to_string_lossy()
        .into_owned();
    let payload: Value = if payload_json.is_null() {
        Value::Null
    } else {
        let s = unsafe { CStr::from_ptr(payload_json) }.to_string_lossy();
        serde_json::from_str(&s).unwrap_or(Value::Null)
    };
    let event = Event::new(kind, "macos.eventbus", payload);
    let fired = h.engine.dispatch(&event, None);
    clear_last_error();
    fired as i32
}

/// Number of triggers currently loaded. Diagnostic — useful for
/// `system.list_triggers`-equivalent introspection.
///
/// # Safety
///
/// `handle` must be a valid engine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_engine_count_triggers(handle: *mut EngineHandle) -> i32 {
    if handle.is_null() {
        return -1;
    }
    // SAFETY: caller contract.
    let h = unsafe { &*handle };
    h.engine.count() as i32
}
