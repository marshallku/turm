//! Slack Socket Mode WebSocket client.
//!
//! Flow:
//! 1. POST `apps.connections.open` with the App-Level Token to get a
//!    fresh single-use WSS URL (Slack does its own load-balancing /
//!    rotation here).
//! 2. Connect via tungstenite. The URL already has auth embedded so
//!    no further headers needed.
//! 3. Read frames:
//!    - `hello` — the connection is live; reset reconnect backoff.
//!    - `events_api` — actual event delivery. Parse the inner
//!      payload via `events::from_events_api_payload`, ACK with
//!      `{"envelope_id": ...}`, and call the supplied event handler.
//!    - `disconnect` — Slack is rotating us off this instance. Treat
//!      as a normal closure and reconnect with the bootstrap URL
//!      regenerated.
//!    - other frame types (`slash_commands`, `interactive`) — ignored
//!      for v1.
//! 4. Any I/O error or close frame → reconnect with exponential
//!    backoff (capped at `Config::reconnect_max`).
//!
//! Reconnect bootstrap is mandatory each cycle: the WSS URL Slack
//! returns is single-use, so a stale URL would 403. Calling
//! `apps.connections.open` repeatedly is the documented contract.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::{Message, http::Uri};

use crate::config::Config;
use crate::events::{SlackEvent, from_events_api_payload};
use crate::store::TokenStore;

const CONNECTIONS_OPEN_URL: &str = "https://slack.com/api/apps.connections.open";

/// Wait period when no credentials are available — the loop polls
/// the store this often so a `turm-plugin-slack auth` invocation
/// while the plugin is already running gets picked up without a
/// supervisor restart. Wakes up every 250ms to check the stop flag
/// so shutdown stays responsive.
const NO_CREDS_RECHECK: Duration = Duration::from_secs(30);

/// Source-tagged credential pair for the next connect attempt. The
/// tag is exposed via `slack.auth_status.credentials_source` so
/// callers see which set is live without ambiguity.
pub struct ResolvedCredentials {
    pub bot_token: String,
    pub app_token: String,
    pub source: &'static str, // "env" | "store"
}

/// Resolve a complete `(bot, app)` pair from a SINGLE source — env
/// wins when both env tokens are present; otherwise both tokens
/// must come from the store. Cross-source mixing (env_bot +
/// store_app etc.) is intentionally not allowed because it would
/// let the runtime connect with credentials the user never
/// authenticated together — and `auth_status` couldn't report a
/// single coherent source for the live pair.
pub fn current_credentials(config: &Config, store: &dyn TokenStore) -> Option<ResolvedCredentials> {
    if !config.bot_token.is_empty() && !config.app_token.is_empty() {
        return Some(ResolvedCredentials {
            bot_token: config.bot_token.clone(),
            app_token: config.app_token.clone(),
            source: "env",
        });
    }
    let stored = store.load()?;
    if stored.bot_token.is_empty() || stored.app_token.is_empty() {
        return None;
    }
    Some(ResolvedCredentials {
        bot_token: stored.bot_token,
        app_token: stored.app_token,
        source: "store",
    })
}

/// Run the Socket Mode client until the supplied stop flag flips
/// true. Reconnects automatically across normal disconnects, network
/// failures, and Slack-initiated rotations. Re-resolves credentials
/// on every iteration so a credential update via
/// `turm-plugin-slack auth` (or a token rotation) is picked up on
/// the next reconnect without needing a process restart. Each
/// successfully parsed event is delivered to `on_event`. Errors are
/// logged to stderr and the loop continues — only a hard stop
/// signal exits.
pub fn run_loop<F>(
    config: &Config,
    store: Arc<dyn TokenStore>,
    stop: &std::sync::atomic::AtomicBool,
    mut on_event: F,
) where
    F: FnMut(SlackEvent) + Send,
{
    // Refuse to run if env validation surfaced a malformed value.
    // Without this guard, falling back to the `default`-workspace
    // store would silently mask a user typo (bad TURM_SLACK_WORKSPACE,
    // bad token prefix). The plugin keeps the supervisor handshake
    // alive so `slack.auth_status` can report the error.
    if let Some(err) = &config.fatal_error {
        eprintln!(
            "[slack] socket mode loop NOT starting: {err}\n\
             [slack] check `turmctl call slack.auth_status` for details"
        );
        return;
    }
    let mut backoff_secs = config.reconnect_initial.as_secs().max(1);
    let mut last_no_creds_log = std::time::Instant::now()
        .checked_sub(NO_CREDS_RECHECK)
        .unwrap_or_else(std::time::Instant::now);
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let creds = current_credentials(config, &*store);
        let Some(creds) = creds else {
            // Don't spam logs on every recheck — emit once per
            // NO_CREDS_RECHECK window. Avoids 30 lines/15min if the
            // user is genuinely deciding when to run `auth`.
            if last_no_creds_log.elapsed() >= NO_CREDS_RECHECK {
                eprintln!(
                    "[slack] no credentials yet — waiting (run `turm-plugin-slack auth` \
                     or set TURM_SLACK_BOT_TOKEN / TURM_SLACK_APP_TOKEN)"
                );
                last_no_creds_log = std::time::Instant::now();
            }
            if interruptible_sleep(stop, NO_CREDS_RECHECK) {
                return;
            }
            continue;
        };
        eprintln!("[slack] connecting (credentials_source={})", creds.source);
        let _ = creds.bot_token; // bot_token is reserved for future write actions
        match connect_and_serve(&creds.app_token, stop, &mut on_event) {
            Ok(reason) => {
                eprintln!("[slack] socket mode disconnected ({reason}); reconnecting");
                backoff_secs = config.reconnect_initial.as_secs().max(1);
            }
            Err(e) => {
                eprintln!(
                    "[slack] socket mode error, reconnecting in {backoff_secs}s: {e}"
                );
                if interruptible_sleep(stop, Duration::from_secs(backoff_secs)) {
                    return;
                }
                backoff_secs =
                    (backoff_secs * 2).min(config.reconnect_max.as_secs().max(1));
            }
        }
    }
    eprintln!("[slack] socket mode loop exited (stop signal)");
}

/// Sleep up to `total`, waking every 250ms to check the stop flag.
/// Returns true if stop was observed (caller should exit).
fn interruptible_sleep(stop: &std::sync::atomic::AtomicBool, total: Duration) -> bool {
    let step = Duration::from_millis(250);
    let mut elapsed = Duration::ZERO;
    while elapsed < total {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            return true;
        }
        let remaining = total - elapsed;
        let chunk = if remaining < step { remaining } else { step };
        std::thread::sleep(chunk);
        elapsed += chunk;
    }
    false
}

/// Returns `Ok(reason)` on a graceful disconnect (Slack rotated us /
/// peer closed normally) so the caller can reconnect immediately
/// without backoff penalty. Returns `Err(msg)` for any error worth
/// backing off from.
fn connect_and_serve<F>(
    app_token: &str,
    stop: &std::sync::atomic::AtomicBool,
    on_event: &mut F,
) -> Result<&'static str, String>
where
    F: FnMut(SlackEvent),
{
    let wss_url = bootstrap_url(app_token)?;
    let uri: Uri = wss_url
        .parse()
        .map_err(|e| format!("invalid wss url returned by Slack: {e}"))?;
    let (mut ws, _resp) = tungstenite::connect(uri).map_err(|e| format!("ws connect: {e}"))?;
    eprintln!("[slack] socket mode connected");

    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let msg = match ws.read() {
            Ok(m) => m,
            // Generic WebSocket closes (TCP RST, peer goodbye, idle
            // timeout) are NOT the same as Slack's `disconnect`
            // frame — they may indicate a real problem (network
            // hiccup, server overload). Return Err so the outer
            // loop applies exponential backoff. Without this, a
            // peer that closes immediately would drive a tight
            // reconnect against Slack.
            Err(tungstenite::Error::ConnectionClosed) => {
                return Err("connection closed unexpectedly".to_string());
            }
            Err(tungstenite::Error::AlreadyClosed) => {
                return Err("connection already closed".to_string());
            }
            Err(e) => return Err(format!("ws read: {e}")),
        };
        match msg {
            Message::Text(s) => {
                if let Some(reason) = handle_text_frame(&s, &mut ws, on_event)? {
                    return Ok(reason);
                }
            }
            Message::Binary(_) => {
                // Slack Socket Mode is text-only; binary frames would
                // be a protocol surprise. Ignore.
            }
            Message::Ping(_) | Message::Pong(_) => {
                // tungstenite handles ping/pong automatically when
                // we drive `read()` regularly.
            }
            Message::Close(frame) => {
                // Peer-initiated close that is NOT a Slack
                // `disconnect` rotation — back off in case the
                // underlying issue is a real problem.
                let reason = frame
                    .as_ref()
                    .map(|f| format!("close {:?} '{}'", f.code, f.reason))
                    .unwrap_or_else(|| "close (no frame)".to_string());
                return Err(format!("peer close: {reason}"));
            }
            Message::Frame(_) => {
                // Raw frame — passthrough; not used in our high-level loop.
            }
        }
    }
    Ok("stop signal")
}

fn handle_text_frame<F>(
    s: &str,
    ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    on_event: &mut F,
) -> Result<Option<&'static str>, String>
where
    F: FnMut(SlackEvent),
{
    let frame: Value = serde_json::from_str(s).map_err(|e| format!("bad frame: {e}"))?;
    let frame_type = frame.get("type").and_then(Value::as_str).unwrap_or("");
    match frame_type {
        "hello" => {
            // Connection ready; nothing to do — could log connection_info
            // here for diagnostics.
        }
        "events_api" => {
            let envelope_id = frame
                .get("envelope_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
            // ACK FIRST so Slack doesn't retry. If the event handler
            // takes time (KB write, etc.) Slack would otherwise
            // re-deliver the same event up to 3 times.
            let ack = json!({"envelope_id": envelope_id});
            ws.send(Message::Text(ack.to_string()))
                .map_err(|e| format!("ack send: {e}"))?;
            // One frame can produce two events: a `slack.raw`
            // firehose entry plus an optional filtered
            // mention/dm. Caller is expected to handle each.
            for slack_event in from_events_api_payload(&payload) {
                on_event(slack_event);
            }
        }
        "disconnect" => {
            // Normal Slack-initiated rotation. Reconnect with a fresh
            // bootstrap URL.
            let reason = frame
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unspecified");
            eprintln!("[slack] received disconnect frame, reason={reason}");
            return Ok(Some("slack rotated us"));
        }
        "slash_commands" | "interactive" => {
            // Out of scope for v1 — would need to ACK these for
            // Slack to consider them handled, but we don't emit any
            // turm event for them. ACK with empty payload so Slack
            // doesn't accumulate retries.
            if let Some(envelope_id) = frame.get("envelope_id").and_then(Value::as_str) {
                let ack = json!({"envelope_id": envelope_id});
                let _ = ws.send(Message::Text(ack.to_string()));
            }
        }
        other if !other.is_empty() => {
            eprintln!("[slack] ignoring unknown frame type: {other}");
        }
        _ => {}
    }
    Ok(None)
}

/// POST `apps.connections.open` with the App-Level Token. Returns the
/// single-use WebSocket URL.
fn bootstrap_url(app_token: &str) -> Result<String, String> {
    let resp = ureq::post(CONNECTIONS_OPEN_URL)
        .set("Authorization", &format!("Bearer {app_token}"))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| format!("apps.connections.open: {e}"))?;
    if resp.status() != 200 {
        let status = resp.status();
        let body = resp.into_string().unwrap_or_default();
        return Err(format!("apps.connections.open HTTP {status}: {body}"));
    }
    let body: Value = resp
        .into_json()
        .map_err(|e| format!("apps.connections.open response parse: {e}"))?;
    if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let err = body.get("error").and_then(Value::as_str).unwrap_or("?");
        return Err(format!("apps.connections.open error: {err}"));
    }
    body.get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "apps.connections.open response missing 'url'".to_string())
}

/// Validate the App-Level Token by exercising the same endpoint
/// Socket Mode uses at runtime — `apps.connections.open`. Discards
/// the returned single-use URL; if the token is bad or missing
/// `connections:write` scope the API returns a non-ok body and we
/// surface the error string. Used by the `auth` subcommand so a
/// typo or scope misconfiguration fails fast at setup time.
pub fn validate_app_token(app_token: &str) -> Result<(), String> {
    bootstrap_url(app_token).map(|_url| ())
}

/// Post a message to a Slack channel or DM via `chat.postMessage`.
/// Returns the posted message's `(ts, channel)` on success — `ts`
/// is the canonical handle for follow-up edits/deletes/reactions.
/// Surfaces Slack's `error` field as the Err string so callers see
/// `missing_scope` / `not_in_channel` / `channel_not_found` etc.
/// directly. `thread_ts` is optional; supply to thread the reply.
pub fn post_message(
    bot_token: &str,
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
) -> Result<(String, String), String> {
    let mut form: Vec<(&str, &str)> = vec![("channel", channel), ("text", text)];
    if let Some(ts) = thread_ts {
        form.push(("thread_ts", ts));
    }
    let resp = match ureq::post("https://slack.com/api/chat.postMessage")
        .set("Authorization", &format!("Bearer {bot_token}"))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .timeout(Duration::from_secs(15))
        .send_form(&form)
    {
        Ok(r) => r,
        // 429 from Slack carries a `Retry-After` header rather than an
        // ok=false JSON body, so it never reaches the JSON-error path
        // below. Map it to the documented `rate_limited` code so
        // triggers can branch on the string verbatim. We surface
        // Retry-After in the message because callers usually want it.
        Err(ureq::Error::Status(429, r)) => {
            let retry = r
                .header("Retry-After")
                .map(str::to_string)
                .unwrap_or_default();
            return Err(format!(
                "rate_limited (Retry-After: {})",
                if retry.is_empty() { "unknown" } else { &retry }
            ));
        }
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(format!("chat.postMessage HTTP {code}: {body}"));
        }
        Err(e) => return Err(format!("chat.postMessage: {e}")),
    };
    let body: Value = resp
        .into_json()
        .map_err(|e| format!("chat.postMessage response parse: {e}"))?;
    if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let err = body.get("error").and_then(Value::as_str).unwrap_or("?");
        return Err(err.to_string());
    }
    // Slack guarantees `ts` and `channel` on success.
    let ts = body
        .get("ts")
        .and_then(Value::as_str)
        .ok_or_else(|| "chat.postMessage response missing ts".to_string())?
        .to_string();
    let posted_channel = body
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or(channel)
        .to_string();
    Ok((ts, posted_channel))
}

/// Validate a Bot User OAuth Token against `auth.test`. Returns the
/// `(team_id, user_id)` tuple on success. Used by `auth` subcommand
/// to confirm the token before persisting.
pub fn auth_test(bot_token: &str) -> Result<(String, String), String> {
    let resp = ureq::post("https://slack.com/api/auth.test")
        .set("Authorization", &format!("Bearer {bot_token}"))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| format!("auth.test: {e}"))?;
    if resp.status() != 200 {
        let status = resp.status();
        let body = resp.into_string().unwrap_or_default();
        return Err(format!("auth.test HTTP {status}: {body}"));
    }
    let body: Value = resp
        .into_json()
        .map_err(|e| format!("auth.test response parse: {e}"))?;
    if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let err = body.get("error").and_then(Value::as_str).unwrap_or("?");
        return Err(format!("auth.test error: {err}"));
    }
    let team_id = body
        .get("team_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "auth.test response missing team_id".to_string())?
        .to_string();
    let user_id = body
        .get("user_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "auth.test response missing user_id".to_string())?
        .to_string();
    Ok((team_id, user_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TokenSet;

    struct StubStore(Option<TokenSet>);
    impl TokenStore for StubStore {
        fn load(&self) -> Option<TokenSet> {
            self.0.clone()
        }
        fn save(&self, _: &TokenSet) -> Result<(), String> {
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            Ok(())
        }
        fn kind(&self) -> &'static str {
            "stub"
        }
    }

    fn cfg(bot: &str, app: &str) -> Config {
        Config {
            bot_token: bot.to_string(),
            app_token: app.to_string(),
            workspace_label: "test".into(),
            require_secure_store: false,
            plaintext_path: std::path::PathBuf::from("/tmp/unused"),
            reconnect_initial: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(60),
            fatal_error: None,
        }
    }

    fn stored_pair(bot: &str, app: &str) -> StubStore {
        StubStore(Some(TokenSet {
            bot_token: bot.into(),
            app_token: app.into(),
            team_id: Some("T_STORE".into()),
            user_id: Some("U_STORE".into()),
        }))
    }

    #[test]
    fn full_env_pair_wins_over_store() {
        let store = stored_pair("xoxb-store", "xapp-store");
        let r = current_credentials(&cfg("xoxb-env", "xapp-env"), &store).unwrap();
        assert_eq!(r.bot_token, "xoxb-env");
        assert_eq!(r.app_token, "xapp-env");
        assert_eq!(r.source, "env");
    }

    #[test]
    fn empty_env_falls_back_to_complete_store() {
        let store = stored_pair("xoxb-store", "xapp-store");
        let r = current_credentials(&cfg("", ""), &store).unwrap();
        assert_eq!(r.bot_token, "xoxb-store");
        assert_eq!(r.app_token, "xapp-store");
        assert_eq!(r.source, "store");
    }

    #[test]
    fn cross_source_mixing_is_forbidden_partial_env_only_bot() {
        // env supplies only bot; store has both. Must NOT mix
        // env_bot + store_app — that pair was never authenticated
        // together. Result: defer to store as a complete unit.
        let store = stored_pair("xoxb-store", "xapp-store");
        let r = current_credentials(&cfg("xoxb-env-only", ""), &store).unwrap();
        assert_eq!(r.bot_token, "xoxb-store");
        assert_eq!(r.app_token, "xapp-store");
        assert_eq!(r.source, "store");
    }

    #[test]
    fn cross_source_mixing_is_forbidden_partial_env_only_app() {
        let store = stored_pair("xoxb-store", "xapp-store");
        let r = current_credentials(&cfg("", "xapp-env-only"), &store).unwrap();
        assert_eq!(r.source, "store");
        assert_eq!(r.bot_token, "xoxb-store");
    }

    #[test]
    fn no_env_no_store_returns_none() {
        let store = StubStore(None);
        assert!(current_credentials(&cfg("", ""), &store).is_none());
    }

    #[test]
    fn partial_store_with_no_env_returns_none() {
        // Store contains a TokenSet but one half is empty —
        // shouldn't ever happen in practice (auth writes both
        // simultaneously) but the runtime defends anyway.
        let store = stored_pair("xoxb-store", "");
        assert!(current_credentials(&cfg("", ""), &store).is_none());
    }
}
