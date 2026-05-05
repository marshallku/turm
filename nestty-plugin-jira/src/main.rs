//! First-party Jira (Atlassian Cloud) service plugin for nestty.
//!
//! Two run modes (selected by `argv[1]`):
//! - **`auth`** — validates the env-supplied API token via
//!   `/rest/api/3/myself` and persists `{email, api_token, base_url,
//!   account_id, display_name}` to the configured store. Exits 0 on
//!   success.
//! - **(no args)** — RPC mode. Speaks the nestty service-plugin
//!   protocol over stdio. Slice 16.1 only handles `jira.auth_status`;
//!   slice 16.2 will add the polling loop + 5 read/write actions.
//!
//! If RPC mode starts with no stored credentials AND the env tokens
//! are missing, the supervisor handshake still completes — actions
//! return `not_authenticated` until the user runs the `auth`
//! subcommand or fixes the env.
//!
//! See `docs/service-plugins.md` for the protocol contract. Atlassian
//! Cloud-only (no Server / Data Center). API token + Basic auth — no
//! OAuth round-trip needed for personal use, matching the env+keyring
//! posture every other nestty plugin uses today.

#[cfg(not(unix))]
compile_error!(
    "nestty-plugin-jira is currently Unix-only. The keyring crate's mock fallback \
     would silently lose tokens on platforms without a native credential-store \
     feature; gate exists to make that failure compile-time instead of runtime."
);

mod config;
mod jira;
mod store;

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;

use serde_json::{Value, json};

use config::Config;
use store::{TokenSet, TokenStore};

const PROTOCOL_VERSION: u32 = 1;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("auth") => run_auth(),
        Some(other) => {
            eprintln!("[jira] unknown subcommand: {other}");
            eprintln!("usage: nestty-plugin-jira [auth]");
            std::process::exit(2);
        }
        None => run_rpc(),
    }
}

fn run_auth() {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[jira] config error: {e}");
            std::process::exit(1);
        }
    };
    // `auth` is the one path where the env credentials MUST be
    // present — RPC mode is allowed to fall through to the store,
    // but `auth` is what populates the store in the first place.
    if config.base_url.is_empty() {
        eprintln!(
            "[jira] auth requires NESTTY_JIRA_BASE_URL (e.g. https://yourcompany.atlassian.net)"
        );
        std::process::exit(1);
    }
    if config.email.is_empty() {
        eprintln!("[jira] auth requires NESTTY_JIRA_EMAIL");
        std::process::exit(1);
    }
    if config.api_token.is_empty() {
        eprintln!(
            "[jira] auth requires NESTTY_JIRA_API_TOKEN (generate at id.atlassian.com/manage-profile/security/api-tokens)"
        );
        std::process::exit(1);
    }
    let store = store::open_store(&config);
    eprintln!("[jira] token store: {}", store.kind());

    eprintln!(
        "[jira] validating credentials via {}/rest/api/3/myself...",
        config.base_url
    );
    let user = match jira::validate_credentials(&config.base_url, &config.email, &config.api_token)
    {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[jira] /myself failed: {e}");
            eprintln!(
                "[jira] verify NESTTY_JIRA_BASE_URL, NESTTY_JIRA_EMAIL, NESTTY_JIRA_API_TOKEN \
                 — generate a token at id.atlassian.com/manage-profile/security/api-tokens"
            );
            std::process::exit(1);
        }
    };
    // Cross-check the env email against what Atlassian reports. A
    // common paste-mismatch is one team member's token + another's
    // email; that combination passes validate_credentials silently
    // (Basic auth only checks the secret, not the email field — the
    // email field is just a username) but later API calls under the
    // user-confused identity get confusing.
    if !user.email_address.is_empty() && !user.email_address.eq_ignore_ascii_case(&config.email) {
        // Fields differ. We still allow the auth to proceed — some
        // workspaces hide email via privacy controls, returning an
        // empty string which we already short-circuited. But warn
        // loudly so the user knows what they got.
        eprintln!(
            "[jira] WARNING: NESTTY_JIRA_EMAIL ({:?}) does not match the account's \
             emailAddress ({:?}). Auth succeeded with the API token, but you may have \
             pasted credentials from a different Atlassian account than you intended.",
            config.email, user.email_address
        );
    }
    let tokens = TokenSet {
        email: config.email.clone(),
        api_token: config.api_token.clone(),
        base_url: config.base_url.clone(),
        account_id: user.account_id.clone(),
        display_name: user.display_name.clone(),
    };
    if let Err(e) = store.save(&tokens) {
        eprintln!("[jira] failed to save tokens: {e}");
        std::process::exit(1);
    }
    eprintln!(
        "[jira] auth ok — account={} display={:?} stored ({})",
        user.account_id,
        user.display_name,
        store.kind()
    );
}

fn run_rpc() {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[jira] FATAL config error — Jira actions disabled until fixed: {e}");
            Config::minimal_with_error(e)
        }
    };
    let store: Arc<dyn TokenStore> = Arc::from(store::open_store(&config));
    eprintln!(
        "[jira] token store: {} (env credentials: {})",
        store.kind(),
        if config.env_creds_empty() {
            "empty — will fall back to store"
        } else {
            "present — will override store"
        }
    );

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    // Single writer thread funnels all outgoing JSON so init reply,
    // action replies, and (slice 16.2) event.publish notifications
    // never interleave.
    let (tx, rx) = channel::<String>();
    let writer_tx = tx.clone();
    thread::spawn(move || {
        let mut out = stdout.lock();
        for line in rx.iter() {
            if writeln!(out, "{line}").is_err() || out.flush().is_err() {
                break;
            }
        }
    });

    let initialized = Arc::new(AtomicBool::new(false));
    // Keep this for slice 16.2 — when the poller spawns it'll read
    // this flag to decide whether to start ticking. Today it's a
    // no-op.
    let _ = initialized.clone();

    let reader = BufReader::new(stdin.lock());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[jira] parse error: {e}");
                continue;
            }
        };
        handle_frame(&frame, &writer_tx, &initialized, &config, &store);
    }
}

fn handle_frame(
    frame: &Value,
    tx: &Sender<String>,
    initialized: &AtomicBool,
    config: &Config,
    store: &Arc<dyn TokenStore>,
) {
    let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
    let id = frame.get("id").and_then(Value::as_str).unwrap_or("");
    let params = frame.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let proto = params.get("protocol_version").and_then(Value::as_u64);
            if proto != Some(PROTOCOL_VERSION as u64) {
                send_error(
                    tx,
                    id,
                    "protocol_mismatch",
                    &format!("jira plugin speaks protocol {PROTOCOL_VERSION}; got {proto:?}"),
                );
                return;
            }
            send_response(
                tx,
                id,
                json!({
                    "service_version": env!("CARGO_PKG_VERSION"),
                    "provides": ["jira.auth_status"],
                    "subscribes": [],
                }),
            );
        }
        "initialized" => {
            initialized.store(true, Ordering::SeqCst);
        }
        "action.invoke" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let action_params = params.get("params").cloned().unwrap_or(Value::Null);
            let result = handle_action(&name, &action_params, config, store);
            match result {
                Ok(v) => send_response(tx, id, v),
                Err((code, msg)) => send_error(tx, id, &code, &msg),
            }
        }
        "event.dispatch" => {
            // jira plugin doesn't subscribe — quietly ignore.
        }
        "shutdown" => std::process::exit(0),
        other if !other.is_empty() && !id.is_empty() => {
            send_error(
                tx,
                id,
                "unknown_method",
                &format!("jira plugin: unknown method {other}"),
            );
        }
        _ => {}
    }
}

fn handle_action(
    name: &str,
    _params: &Value,
    config: &Config,
    store: &Arc<dyn TokenStore>,
) -> Result<Value, (String, String)> {
    if name == "jira.auth_status" {
        return Ok(auth_status_payload(config, store));
    }
    Err((
        "action_not_found".to_string(),
        format!("jira plugin does not handle {name}"),
    ))
}

/// Resolved credentials with their source label, mirroring
/// slack::current_credentials. Returned as `None` when neither env
/// nor store has a usable set.
pub struct ResolvedCreds {
    pub source: &'static str,
    pub base_url: String,
    pub email: String,
    pub api_token: String,
}

pub fn current_credentials(config: &Config, store: &dyn TokenStore) -> Option<ResolvedCreds> {
    if !config.env_creds_empty() {
        return Some(ResolvedCreds {
            source: "env",
            base_url: config.base_url.clone(),
            email: config.email.clone(),
            api_token: config.api_token.clone(),
        });
    }
    let t = store.load()?;
    // Reject incomplete stored entries — a manually edited keyring
    // record (or a malformed plaintext file partially overwritten by
    // a stuck write) with empty fields would otherwise make
    // `auth_status` report `authenticated=true` and push the
    // runtime into confusing 401s instead of a clean
    // `not_authenticated`. Same posture as slack/discord's stored
    // credential resolvers.
    if t.email.is_empty() || t.api_token.is_empty() || t.base_url.is_empty() {
        eprintln!(
            "[jira] stored credentials are incomplete (email/api_token/base_url empty); \
             treating as not_authenticated. Re-run `nestty-plugin-jira auth` to repair."
        );
        return None;
    }
    // Apply the SAME `*.atlassian.net` host restriction to stored
    // base_url that env validation enforces. Without this gate a
    // tampered keyring entry (or a hand-edited plaintext store with
    // a swapped host) would steer authenticated REST calls to an
    // arbitrary HTTPS endpoint — defeating the env-time defense
    // against API-token exfiltration. Treats validation failure as
    // "stored credentials are unsafe" → not_authenticated; user
    // re-runs `auth` to overwrite with a clean record.
    if let Err(e) = config::validate_base_url(&t.base_url) {
        eprintln!(
            "[jira] stored base_url failed Atlassian Cloud host check ({e}); \
             treating as not_authenticated to avoid sending the API token to an unintended host. \
             Re-run `nestty-plugin-jira auth` to repair."
        );
        return None;
    }
    Some(ResolvedCreds {
        source: "store",
        base_url: t.base_url,
        email: t.email,
        api_token: t.api_token,
    })
}

/// Build the `jira.auth_status` payload. Pulled into a function so
/// slice 16.2's polling loop and any future `nestctl` wrapper can
/// reuse the same shape verbatim.
fn auth_status_payload(config: &Config, store: &Arc<dyn TokenStore>) -> Value {
    // When env validation produced a fatal_error we MUST NOT report
    // authenticated=true based on a fall-through default-workspace
    // store load — the runtime would refuse to make any HTTP call
    // (because handle_action short-circuits on fatal_error in 16.2),
    // so the status surface would lie about the runtime state.
    if let Some(err) = &config.fatal_error {
        return json!({
            "configured": false,
            "authenticated": false,
            "credentials_source": "none",
            "fatal_error": err,
            "store_kind": store.kind(),
            "workspace": config.workspace_label.clone(),
            "base_url": config.base_url.clone(),
            "account_id": Value::Null,
            "display_name": Value::Null,
        });
    }
    let resolved = current_credentials(config, &**store);
    let stored = store.load();
    let credentials_source = resolved.as_ref().map(|c| c.source).unwrap_or("none");
    let authenticated = resolved.is_some();
    // Identity (account_id, display_name) is only verified for the
    // store source — that's the only path where we ran /myself at
    // auth time. For env-overridden credentials we don't have a
    // verified identity for THOSE specific tokens, so reporting the
    // stored identity would be misleading (the env tokens could be
    // from a different Atlassian account than the store). Surface
    // them only when consistent with the live source.
    let report_identity = credentials_source == "store";
    // The base_url field reports the live source's URL — matches
    // resolved.base_url when authenticated, falls back to env when
    // env is set but store is empty (handled by current_credentials),
    // empty when neither is set.
    let base_url = resolved
        .as_ref()
        .map(|c| c.base_url.clone())
        .unwrap_or_else(|| config.base_url.clone());
    json!({
        "configured": true,
        "authenticated": authenticated,
        "credentials_source": credentials_source,
        "fatal_error": Value::Null,
        "store_kind": store.kind(),
        "workspace": config.workspace_label.clone(),
        "base_url": base_url,
        "account_id": if report_identity {
            stored.as_ref().map(|t| t.account_id.clone()).map(Value::String).unwrap_or(Value::Null)
        } else { Value::Null },
        "display_name": if report_identity {
            stored.as_ref().map(|t| t.display_name.clone()).map(Value::String).unwrap_or(Value::Null)
        } else { Value::Null },
    })
}

fn send_response(tx: &Sender<String>, id: &str, result: Value) {
    let frame = json!({ "id": id, "ok": true, "result": result });
    let _ = tx.send(frame.to_string());
}

fn send_error(tx: &Sender<String>, id: &str, code: &str, message: &str) {
    let frame = json!({
        "id": id,
        "ok": false,
        "error": { "code": code, "message": message },
    });
    let _ = tx.send(frame.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::TokenStore;

    struct EmptyStore;
    impl TokenStore for EmptyStore {
        fn load(&self) -> Option<TokenSet> {
            None
        }
        fn save(&self, _: &TokenSet) -> Result<(), String> {
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            Ok(())
        }
        fn kind(&self) -> &'static str {
            "test-empty"
        }
    }

    struct FixedStore(TokenSet);
    impl TokenStore for FixedStore {
        fn load(&self) -> Option<TokenSet> {
            Some(self.0.clone())
        }
        fn save(&self, _: &TokenSet) -> Result<(), String> {
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            Ok(())
        }
        fn kind(&self) -> &'static str {
            "test-fixed"
        }
    }

    fn sample_tokens() -> TokenSet {
        TokenSet {
            email: "marshall@example.com".into(),
            api_token: "tok-stored".into(),
            base_url: "https://stored.atlassian.net".into(),
            account_id: "5b-stored".into(),
            display_name: "Stored Marshall".into(),
        }
    }

    fn good_config() -> Config {
        Config {
            base_url: "https://env.atlassian.net".into(),
            email: "env@example.com".into(),
            api_token: "tok-env".into(),
            workspace_label: "default".into(),
            require_secure_store: false,
            plaintext_path: std::path::PathBuf::from("/tmp/x"),
            fatal_error: None,
        }
    }

    #[test]
    fn auth_status_reports_fatal_error_short_circuit() {
        let store: Arc<dyn TokenStore> = Arc::new(FixedStore(sample_tokens()));
        let cfg = Config::minimal_with_error("missing X".to_string());
        let v = auth_status_payload(&cfg, &store);
        assert_eq!(v["configured"], false);
        assert_eq!(v["authenticated"], false);
        assert_eq!(v["credentials_source"], "none");
        assert_eq!(v["fatal_error"], "missing X");
        // Identity must NOT leak even though store has tokens — the
        // runtime won't honor them, so the status must reflect that.
        assert_eq!(v["account_id"], Value::Null);
        assert_eq!(v["display_name"], Value::Null);
    }

    #[test]
    fn auth_status_env_source_hides_identity() {
        let store: Arc<dyn TokenStore> = Arc::new(FixedStore(sample_tokens()));
        let cfg = good_config();
        let v = auth_status_payload(&cfg, &store);
        assert_eq!(v["configured"], true);
        assert_eq!(v["authenticated"], true);
        assert_eq!(v["credentials_source"], "env");
        assert_eq!(v["base_url"], "https://env.atlassian.net");
        assert_eq!(v["account_id"], Value::Null);
        assert_eq!(v["display_name"], Value::Null);
    }

    #[test]
    fn auth_status_store_source_reports_identity() {
        let store: Arc<dyn TokenStore> = Arc::new(FixedStore(sample_tokens()));
        // env empty: store wins.
        let mut cfg = good_config();
        cfg.email = String::new();
        cfg.api_token = String::new();
        cfg.base_url = String::new();
        let v = auth_status_payload(&cfg, &store);
        assert_eq!(v["configured"], true);
        assert_eq!(v["authenticated"], true);
        assert_eq!(v["credentials_source"], "store");
        assert_eq!(v["base_url"], "https://stored.atlassian.net");
        assert_eq!(v["account_id"], "5b-stored");
        assert_eq!(v["display_name"], "Stored Marshall");
    }

    #[test]
    fn auth_status_no_creds_anywhere() {
        let store: Arc<dyn TokenStore> = Arc::new(EmptyStore);
        let mut cfg = good_config();
        cfg.email = String::new();
        cfg.api_token = String::new();
        cfg.base_url = String::new();
        let v = auth_status_payload(&cfg, &store);
        assert_eq!(v["configured"], true);
        assert_eq!(v["authenticated"], false);
        assert_eq!(v["credentials_source"], "none");
    }

    #[test]
    fn handle_action_unknown_returns_action_not_found() {
        let store: Arc<dyn TokenStore> = Arc::new(EmptyStore);
        let cfg = good_config();
        let err = handle_action("jira.no_such_action", &Value::Null, &cfg, &store).unwrap_err();
        assert_eq!(err.0, "action_not_found");
    }

    #[test]
    fn handle_action_routes_auth_status() {
        let store: Arc<dyn TokenStore> = Arc::new(EmptyStore);
        let cfg = good_config();
        let v = handle_action("jira.auth_status", &Value::Null, &cfg, &store).unwrap();
        assert_eq!(v["credentials_source"], "env");
    }

    #[test]
    fn current_credentials_prefers_env() {
        let store: Box<dyn TokenStore> = Box::new(FixedStore(sample_tokens()));
        let cfg = good_config();
        let r = current_credentials(&cfg, &*store).unwrap();
        assert_eq!(r.source, "env");
        assert_eq!(r.email, "env@example.com");
    }

    #[test]
    fn current_credentials_falls_back_to_store() {
        let store: Box<dyn TokenStore> = Box::new(FixedStore(sample_tokens()));
        let mut cfg = good_config();
        cfg.email = String::new();
        cfg.api_token = String::new();
        cfg.base_url = String::new();
        let r = current_credentials(&cfg, &*store).unwrap();
        assert_eq!(r.source, "store");
        assert_eq!(r.email, "marshall@example.com");
    }

    #[test]
    fn current_credentials_rejects_incomplete_store_entry() {
        // A keyring entry hand-edited to drop the api_token (or any
        // of the three required fields) must NOT surface as
        // authenticated — that would mask the corruption behind a
        // confusing 401 at runtime instead of a clean
        // not_authenticated.
        let mut tokens = sample_tokens();
        tokens.api_token = String::new();
        let store: Box<dyn TokenStore> = Box::new(FixedStore(tokens));
        let mut cfg = good_config();
        cfg.email = String::new();
        cfg.api_token = String::new();
        cfg.base_url = String::new();
        assert!(current_credentials(&cfg, &*store).is_none());
    }

    #[test]
    fn current_credentials_rejects_stored_non_atlassian_base_url() {
        // Critical security boundary: env-time validation rejects
        // non-Atlassian hosts to prevent API-token exfiltration. A
        // hand-edited keyring entry must NOT bypass that check by
        // injecting an arbitrary HTTPS URL.
        for bad_host in [
            "https://attacker.example.com",
            "https://atlassian.net.evil.example",
            "https://x.atlassian.net/jira",
            "https://jira.company.com",
        ] {
            let mut tokens = sample_tokens();
            tokens.base_url = bad_host.to_string();
            let store: Box<dyn TokenStore> = Box::new(FixedStore(tokens));
            let mut cfg = good_config();
            cfg.email = String::new();
            cfg.api_token = String::new();
            cfg.base_url = String::new();
            assert!(
                current_credentials(&cfg, &*store).is_none(),
                "should reject stored base_url {bad_host:?}"
            );
        }
    }
}
