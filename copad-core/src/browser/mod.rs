//! Browser Workbench — the shared spine under copad's webview panes.
//!
//! A webview pane used to be one page: no tabs, no history, an origin-only
//! restore, no credential story, and no way for an agent to see what the page
//! did on the network. This module is the half of the fix that must NOT be
//! written twice — every model, policy, path rule, and redaction rule the two
//! GUIs need lives here, so `copad-linux` and `copad-macos` can only differ in
//! the four places they genuinely must (the opaque history blob, the persistent
//! profile store, the secret-material backend, and offscreen rendering).
//!
//! See `docs/browser-workbench-plan.md` for the full design and the prior-art
//! survey (Cursor's browser tool, aslan-browser, 1Password's agentic autofill).
//!
//! **Design constraints, each traceable to a codex plan round:**
//!
//! - **A protected document, not a lockout on a dirty one** (r2-C1/C2). Refusing
//!   read-back RPCs *after* a password is filled guarantees nothing: the agent
//!   could have installed a listener beforehand and read its copy the moment the
//!   lock lifted. So a credential only ever enters a tab that is already in
//!   [`TabMode::Protected`], which the GUI enters by DESTROYING and rebuilding
//!   the web view. No script from the old document survives that.
//! - **Entry and exit are explicit** (r3-C2/C3). "Auto-protect once a password
//!   field loads" is not an enforceable timing claim — forms are interactive
//!   before load completion, SPA forms appear after it, and fields hide in frames
//!   and shadow roots. Symmetrically, no navigation commit identifies a
//!   successful login (failed logins, MFA, and IdP hops all commit; SPA auth may
//!   never commit). Both transitions are user acts.
//! - **Protection freezes the profile, not the tab** (r3-C1). A concurrent tab on
//!   the same profile is another window onto the same shared storage.
//! - **Capture is suppressed at the source** (r2-C3). Blocking `webview.net`
//!   reads while the JSONL file keeps filling is no defence — the files are
//!   deliberately agent-readable.
//! - **A session file is never filesystem authority** (r3-C4). Tab ids are
//!   validated against a strict charset and blob paths are always REBUILT here;
//!   no string from a session document is ever joined as a path.
//! - **Redaction is hygiene, not a boundary** (r2-C3). A page can `console.log`
//!   anything; name-based filters cannot catch that. The boundary is
//!   [`TabMode`], enforced in [`authorize`].

pub mod authorize;
pub mod history;
pub mod netlog;
pub mod profile;
pub mod restore;
pub mod secrets;
pub mod tabs;

pub use authorize::{MethodClass, authorize, classify};
pub use netlog::{ConsoleRecord, LogCaps, NETLOG_COVERAGE, NetRecord};
pub use profile::{BrowserProfile, DEFAULT_PROFILE};
pub use restore::{RestorePolicy, canonical_origin, canonicalize_for_restore};
pub use secrets::{CredentialRef, CredentialSlot, FillRequest, TabMode};
pub use tabs::{BrowserPaneSnap, BrowserTabSnap};

/// One lock for every test in this module tree that redirects `HOME` /
/// `XDG_STATE_HOME` to a sandbox.
///
/// `history` and `netlog` both do it, and they started with a mutex EACH — so
/// each module's tests were internally serialized and mutually racing. The
/// failure only appeared when the whole suite ran (four unrelated tests failing
/// at once), never when either module was filtered on its own, which is exactly
/// the shape that wastes an afternoon. The lock has to be shared because the
/// resource is: one process-wide environment.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
