//! The credential boundary: metadata crosses the agent wire, secrets never do.
//!
//! The type an agent can see is [`CredentialRef`], and it has **no field that
//! can hold secret material** — that is the point of it being a separate type
//! rather than a struct with an `Option<String> password`. The secret itself
//! lives only in the platform keychain, keyed by [`CredentialRef::id`], and
//! travels from there into the page inside the GUI process. It appears in no
//! request, no response, no event, and no log file.
//!
//! ## Why a "seal" was rejected
//!
//! The first design filled the password and then refused read-back RPCs while
//! it sat in the DOM. Codex (plan r2-C1/C2) showed that guarantees nothing: the
//! agent can install a listener *before* the fill that copies the value into
//! `localStorage`, and read the copy the moment the lock lifts. Nor does any
//! transition prove the secret is gone — a cancelled submit, an SPA that
//! swallows the event, or a back-navigation all leave it recoverable.
//!
//! So a credential only ever enters a document that automation was never in:
//! [`TabMode::Protected`], which the GUI enters by **destroying and rebuilding
//! the web view**. No script from the previous document survives that.
//!
//! ## What this module does and does not promise
//!
//! Promises: the secret never crosses the agent boundary, and no agent-installed
//! script is present in the document holding it.
//!
//! Does not promise: safety from a page that is *itself* hostile. A malicious
//! login page exfiltrates whatever is typed into it, by hand or by copad. True
//! of every browser, out of scope here.

use serde::{Deserialize, Serialize};

use crate::protocol::ResponseError;

/// Which field of a login form a credential fills. A password credential may
/// only ever be written into an `<input type="password">`; the check lives in
/// [`validate_fill`] so both GUIs inherit it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CredentialSlot {
    Username,
    Password,
}

/// **Metadata only.** This is what crosses the RPC boundary, and therefore what
/// an agent can see. Adding a secret-bearing field here would silently undo the
/// entire design.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CredentialRef {
    /// Stable keychain key, e.g. `github.com/marshallku`.
    pub id: String,
    /// Exact origin this credential may be filled into, `scheme://host[:port]`.
    pub origin: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub slot: CredentialSlot,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
}

/// Whether a tab currently admits automation.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabMode {
    /// Agent RPCs are served. The default.
    #[default]
    Automation,
    /// Freshly rebuilt web view, no agent script present, no capture installed.
    /// Agent *reads* are refused; credential fills are allowed.
    Protected,
}

/// A tab's live state as the dispatcher sees it when authorizing a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabState {
    pub tab_id: String,
    pub profile: String,
    pub mode: TabMode,
    pub origin: String,
    /// Bumped by the GUI on every main-frame commit and on every mode change.
    /// This is what binds an authorization to a document rather than an origin.
    pub document_generation: u64,
}

/// An in-flight `browser.secret.fill`, captured at approval time and
/// re-validated at injection time.
///
/// An origin check taken *before* an async keychain read is insufficient — the
/// tab can navigate mid-flight (codex plan r2-C2). Every field here is compared
/// again against the live [`TabState`] immediately before the value touches the
/// page; any mismatch aborts and the secret is dropped without being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillRequest {
    pub credential_id: String,
    pub profile: String,
    pub tab_id: String,
    pub origin: String,
    pub document_generation: u64,
    pub slot: CredentialSlot,
}

/// The target element **as observed right now**, in the same synchronous step
/// that will perform the injection.
///
/// This is deliberately NOT a field of [`FillRequest`]: a value captured at
/// approval time is stale by the time the keychain read returns, and a page can
/// swap the element or flip its `type` without any navigation — so
/// `document_generation` would not change and a stale "yes, it's a password
/// input" would still validate (codex review C1).
///
/// **Platform contract:** observe the element, call [`validate_fill`], and write
/// the value into *that same element*, with no `await` between the three. A GUI
/// that probes the DOM, awaits anything, and then injects has reintroduced the
/// bug this type exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTarget {
    /// Selector the value will actually be written to.
    pub selector: String,
    /// Whether that element is an `<input type="password">` right now.
    pub is_password_input: bool,
}

fn err(code: &str, message: impl Into<String>) -> ResponseError {
    ResponseError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// Re-validate a fill immediately before injection.
///
/// Enforces, in order: the tab still exists in the same profile, it is still
/// `Protected`, the document has not changed under us, the origin matches the
/// credential *exactly* (no subdomain or scheme widening), and a password
/// credential is going into a password input.
pub fn validate_fill(
    req: &FillRequest,
    cred: &CredentialRef,
    live: &TabState,
    target: &LiveTarget,
) -> Result<(), ResponseError> {
    if req.credential_id != cred.id {
        return Err(err(
            "credential_mismatch",
            "request does not name this credential",
        ));
    }
    // The approval captured an origin and a slot. Re-check both against the
    // credential as it stands NOW: the index is a file, and a credential's
    // metadata could have been rewritten between approval and injection —
    // turning a `Username` the user approved into a `Password` that would then
    // be injected on the strength of an approval for something else (codex
    // review C2).
    if !origin_matches(&req.origin, &cred.origin) {
        return Err(err(
            "approval_mismatch",
            "the credential's origin changed after approval",
        ));
    }
    if req.slot != cred.slot {
        return Err(err(
            "approval_mismatch",
            "the credential's slot changed after approval",
        ));
    }
    if req.tab_id != live.tab_id || req.profile != live.profile {
        return Err(err(
            "tab_changed",
            "target tab is no longer the approved one",
        ));
    }
    if live.mode != TabMode::Protected {
        return Err(err(
            "requires_protected",
            "a credential may only be filled into a protected tab",
        ));
    }
    if req.document_generation != live.document_generation {
        return Err(err(
            "document_changed",
            "the tab navigated after approval — fill aborted",
        ));
    }
    if !origin_matches(&cred.origin, &live.origin) {
        return Err(err(
            "origin_mismatch",
            format!(
                "credential is bound to {} but the tab is on {}",
                cred.origin, live.origin
            ),
        ));
    }
    if cred.slot == CredentialSlot::Password && !target.is_password_input {
        return Err(err(
            "not_a_password_input",
            "a password credential may only be filled into <input type=\"password\">",
        ));
    }
    Ok(())
}

/// Exact, case-insensitive origin equality. Deliberately NOT a suffix or
/// subdomain match: `https://evil.github.com` must not receive a credential
/// bound to `https://github.com`, and `http://` must not receive one bound to
/// `https://`.
pub fn origin_matches(credential_origin: &str, tab_origin: &str) -> bool {
    !credential_origin.is_empty()
        && !tab_origin.is_empty()
        && credential_origin.eq_ignore_ascii_case(tab_origin)
}

/// Index of known credentials. Holds no secret material (that is in the
/// keychain), but is still written 0600 because the set of sites you have
/// accounts on is itself worth protecting.
pub fn credential_index_path() -> std::path::PathBuf {
    crate::paths::state_dir()
        .join("browser")
        .join("credentials.json")
}

/// Error returned when the platform keychain is unavailable.
///
/// There is **no plaintext fallback** (codex plan r2-C4). Decision #54's
/// plaintext store exists for revocable Slack bot tokens read by a background
/// plugin; a user's website passwords are neither revocable in one click nor
/// low-value, and a 0600 file is readable by the very same-user agent this
/// module exists to keep them away from. An unavailable backend disables the
/// feature rather than downgrading it.
pub fn backend_unavailable(detail: &str) -> ResponseError {
    err(
        "secret_backend_unavailable",
        format!("platform keychain unavailable, password features are disabled: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cred() -> CredentialRef {
        CredentialRef {
            id: "github.com/marshallku".into(),
            origin: "https://github.com".into(),
            username: "marshallku".into(),
            label: None,
            slot: CredentialSlot::Password,
            created_at: 0,
            last_used: None,
        }
    }

    fn live(mode: TabMode, generation: u64, origin: &str) -> TabState {
        TabState {
            tab_id: "t1".into(),
            profile: "default".into(),
            mode,
            origin: origin.into(),
            document_generation: generation,
        }
    }

    fn req() -> FillRequest {
        FillRequest {
            credential_id: "github.com/marshallku".into(),
            profile: "default".into(),
            tab_id: "t1".into(),
            origin: "https://github.com".into(),
            document_generation: 4,
            slot: CredentialSlot::Password,
        }
    }

    fn target(is_password: bool) -> LiveTarget {
        LiveTarget {
            selector: "#password".into(),
            is_password_input: is_password,
        }
    }

    fn ok_tab() -> TabState {
        live(TabMode::Protected, 4, "https://github.com")
    }

    #[test]
    fn a_matching_protected_document_accepts_the_fill() {
        assert!(validate_fill(&req(), &cred(), &ok_tab(), &target(true)).is_ok());
    }

    #[test]
    fn an_unprotected_tab_is_refused() {
        let tab = live(TabMode::Automation, 4, "https://github.com");
        assert_eq!(
            validate_fill(&req(), &cred(), &tab, &target(true))
                .unwrap_err()
                .code,
            "requires_protected"
        );
    }

    #[test]
    fn a_navigation_between_approval_and_injection_aborts_the_fill() {
        // r2-C2: the tab navigated while the keychain read was in flight.
        let tab = live(TabMode::Protected, 5, "https://github.com");
        assert_eq!(
            validate_fill(&req(), &cred(), &tab, &target(true))
                .unwrap_err()
                .code,
            "document_changed"
        );
    }

    #[test]
    fn a_lookalike_subdomain_does_not_receive_the_credential() {
        let tab = live(TabMode::Protected, 4, "https://evil.github.com");
        assert_eq!(
            validate_fill(&req(), &cred(), &tab, &target(true))
                .unwrap_err()
                .code,
            "origin_mismatch"
        );
    }

    #[test]
    fn downgrading_the_scheme_does_not_receive_the_credential() {
        let tab = live(TabMode::Protected, 4, "http://github.com");
        assert_eq!(
            validate_fill(&req(), &cred(), &tab, &target(true))
                .unwrap_err()
                .code,
            "origin_mismatch"
        );
    }

    #[test]
    fn a_password_will_not_go_into_a_non_password_input() {
        // codex review C1: the check reads the element as it is NOW, so a page
        // that swapped the input after approval — no navigation, so
        // `document_generation` is unchanged — is still caught.
        assert_eq!(
            validate_fill(&req(), &cred(), &ok_tab(), &target(false))
                .unwrap_err()
                .code,
            "not_a_password_input"
        );
    }

    #[test]
    fn a_username_credential_may_go_into_an_ordinary_input() {
        let mut c = cred();
        c.slot = CredentialSlot::Username;
        let mut r = req();
        r.slot = CredentialSlot::Username;
        assert!(validate_fill(&r, &c, &ok_tab(), &target(false)).is_ok());
    }

    #[test]
    fn a_credential_rewritten_after_approval_is_refused() {
        // codex review C2. The index was rewritten between approval and
        // injection, so the captured approval no longer describes what would be
        // injected.
        let mut c = cred();
        c.slot = CredentialSlot::Username;
        assert_eq!(
            validate_fill(&req(), &c, &ok_tab(), &target(true))
                .unwrap_err()
                .code,
            "approval_mismatch"
        );
    }

    #[test]
    fn a_credential_whose_origin_moved_after_approval_is_refused() {
        let mut c = cred();
        c.origin = "https://gitlab.com".into();
        let tab = live(TabMode::Protected, 4, "https://gitlab.com");
        assert_eq!(
            validate_fill(&req(), &c, &tab, &target(true))
                .unwrap_err()
                .code,
            "approval_mismatch"
        );
    }

    #[test]
    fn a_retargeted_tab_is_refused() {
        let mut tab = ok_tab();
        tab.tab_id = "t2".into();
        assert_eq!(
            validate_fill(&req(), &cred(), &tab, &target(true))
                .unwrap_err()
                .code,
            "tab_changed"
        );
    }

    #[test]
    fn origin_match_is_case_insensitive_but_never_a_suffix_match() {
        assert!(origin_matches("https://GitHub.com", "https://github.com"));
        assert!(!origin_matches(
            "https://github.com",
            "https://github.com.evil.net"
        ));
        assert!(!origin_matches("", ""));
    }

    #[test]
    fn the_agent_facing_type_serializes_without_any_secret_field() {
        // Guard against someone later adding a `password` field to the type that
        // is, by definition, what the agent gets to see.
        let json = serde_json::to_string(&cred()).unwrap();
        for forbidden in ["password", "secret", "value", "token"] {
            assert!(
                !json.contains(&format!("\"{forbidden}\":")),
                "CredentialRef must not serialize a `{forbidden}` field: {json}"
            );
        }
    }

    #[test]
    fn the_approval_type_cannot_carry_a_stale_target_observation() {
        // codex review C1 as a structural guard: `FillRequest` is the approval
        // and must not hold anything about the element, or a GUI could pass a
        // value captured before the keychain await.
        let debug = format!("{:?}", req());
        assert!(
            !debug.contains("password_input"),
            "FillRequest must not carry a target observation: {debug}"
        );
    }
}
