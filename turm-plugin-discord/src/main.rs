//! First-party Discord service plugin for turm — slice 1.
//!
//! **Status (Phase 16-equivalent slice 1)**: this commit ships
//! the crate skeleton + `auth` subcommand + plugin manifest +
//! `discord.auth_status` action. The Gateway WebSocket client (which
//! emits `discord.message` / `discord.mention` / `discord.dm` events
//! and provides `discord.send_message`) lands in slice 2.
//!
//! Auth flow (matches `turm-plugin-slack`):
//! 1. User creates an app at <https://discord.com/developers/applications>,
//!    adds a Bot, copies the bot token (Reset Token if first time),
//!    enables the MESSAGE CONTENT privileged intent if they want
//!    message bodies (required for keyword matching).
//! 2. `TURM_DISCORD_BOT_TOKEN=<token> turm-plugin-discord auth` →
//!    validates against Discord's `/users/@me`, persists `TokenSet`
//!    in the OS keyring (with plaintext fallback under
//!    `$XDG_CONFIG_HOME/turm/discord-token-<workspace>.json` unless
//!    `TURM_DISCORD_REQUIRE_SECURE_STORE=1` is set).
//! 3. turm spawns the plugin via the supervisor (`onStartup`); the
//!    plugin reads the token from the store at init time and (in
//!    slice 2) opens the Gateway WebSocket.

mod config;
mod store;

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use config::Config;
use store::{TokenSet, TokenStore, open_store};

const PROTOCOL_VERSION: u32 = 1;
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("auth") => run_auth(),
        None => run_rpc(),
        Some(other) => {
            eprintln!("usage: turm-plugin-discord [auth]");
            eprintln!("unknown subcommand: {other:?}");
            std::process::exit(2);
        }
    }
}

/// Validate the env-supplied bot token against `/users/@me` and
/// persist a `TokenSet` to the configured store. Run interactively
/// from a shell, NOT via the supervisor — supervisor-spawned
/// instances skip the args path.
fn run_auth() {
    let config = Config::from_env();
    if let Some(err) = &config.fatal_error {
        eprintln!("[discord] auth: config error: {err}");
        std::process::exit(2);
    }
    let token = match &config.bot_token_env {
        Some(t) => t.clone(),
        None => {
            eprintln!(
                "[discord] auth requires TURM_DISCORD_BOT_TOKEN env. \
                 Get a token from <https://discord.com/developers/applications>"
            );
            std::process::exit(2);
        }
    };
    eprintln!("[discord] validating token via /users/@me...");
    let me = match get_users_me(&token) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[discord] /users/@me failed: {e}");
            std::process::exit(2);
        }
    };
    let user_id = me.get("id").and_then(Value::as_str).map(str::to_string);
    let username = me
        .get("global_name")
        .and_then(Value::as_str)
        .or_else(|| me.get("username").and_then(Value::as_str))
        .map(str::to_string);
    let store = open_store(&config);
    let set = TokenSet {
        bot_token: token,
        user_id: user_id.clone(),
        username: username.clone(),
    };
    if let Err(e) = store.save(&set) {
        eprintln!("[discord] save failed: {e}");
        std::process::exit(2);
    }
    eprintln!(
        "[discord] auth OK. user_id={} username={} stored in {}",
        user_id.as_deref().unwrap_or("?"),
        username.as_deref().unwrap_or("?"),
        store.kind()
    );
}

/// Issue a `GET /users/@me` against Discord's REST API with the
/// supplied bot token. Returns the JSON body on 2xx, an error
/// describing the failure otherwise.
fn get_users_me(bot_token: &str) -> Result<Value, String> {
    let resp = ureq::get(&format!("{DISCORD_API_BASE}/users/@me"))
        .set("Authorization", &format!("Bot {bot_token}"))
        .set("User-Agent", "turm-plugin-discord (turm, 0.1)")
        .call()
        .map_err(|e| format!("http: {e}"))?;
    let status = resp.status();
    let body = resp.into_string().map_err(|e| format!("read body: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("decode: {e}"))
}

/// Stdio JSON-line RPC loop. Slice 1 handles `initialize` /
/// `initialized` / `action.invoke` for `discord.auth_status` /
/// `shutdown` only. The Gateway WebSocket client + `discord.message`
/// events + `discord.send_message` action come in slice 2.
fn run_rpc() {
    let config = Config::from_env();
    if let Some(err) = &config.fatal_error {
        eprintln!("[discord] config error (actions will return config_error): {err}");
    }
    let store = open_store(&config);
    eprintln!(
        "[discord] workspace={}, store={}",
        config.workspace_label,
        store.kind()
    );

    let stdin = std::io::stdin();
    let stdout = Arc::new(std::sync::Mutex::new(std::io::stdout()));
    let initialized = Arc::new(AtomicBool::new(false));

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
                eprintln!("[discord] parse error: {e}");
                continue;
            }
        };
        handle_frame(&frame, &stdout, &initialized, &config, store.as_ref());
    }
}

fn handle_frame(
    frame: &Value,
    stdout: &Arc<std::sync::Mutex<std::io::Stdout>>,
    initialized: &AtomicBool,
    config: &Config,
    store: &dyn TokenStore,
) {
    let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
    let id = frame.get("id").and_then(Value::as_str).unwrap_or("");
    let params = frame.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let proto = params.get("protocol_version").and_then(Value::as_u64);
            if proto != Some(PROTOCOL_VERSION as u64) {
                send_error(
                    stdout,
                    id,
                    "protocol_mismatch",
                    &format!("discord plugin speaks protocol {PROTOCOL_VERSION}; got {proto:?}"),
                );
                return;
            }
            send_response(
                stdout,
                id,
                json!({
                    "service_version": env!("CARGO_PKG_VERSION"),
                    "provides": ["discord.auth_status"],
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
                Ok(v) => send_response(stdout, id, v),
                Err((code, msg)) => send_error(stdout, id, &code, &msg),
            }
        }
        "event.dispatch" => {}
        "shutdown" => std::process::exit(0),
        other if !other.is_empty() && !id.is_empty() => {
            send_error(
                stdout,
                id,
                "unknown_method",
                &format!("discord plugin: unknown method {other}"),
            );
        }
        _ => {}
    }
}

fn handle_action(
    name: &str,
    _params: &Value,
    config: &Config,
    store: &dyn TokenStore,
) -> Result<Value, (String, String)> {
    match name {
        // Discoverable even when config has fatal errors — same shape
        // Slack's `slack.auth_status` uses, so a future
        // `turmctl context --full` can query both messengers
        // uniformly without special-casing degraded modes.
        // `configured = false` signals "env validation failed";
        // `authenticated = false` signals "env OK but no creds stored
        // yet". Independent flags so the UI can distinguish.
        "discord.auth_status" => {
            let configured = config.fatal_error.is_none();
            let stored = if configured { store.load() } else { None };
            Ok(json!({
                "configured": configured,
                "authenticated": stored.is_some(),
                "store_kind": store.kind(),
                "workspace": config.workspace_label,
                "user_id": stored.as_ref().and_then(|s| s.user_id.clone()),
                "username": stored.as_ref().and_then(|s| s.username.clone()),
                "fatal_error": config.fatal_error.clone(),
            }))
        }
        // Other actions short-circuit on fatal_error.
        _ if config.fatal_error.is_some() => {
            Err(("config_error".into(), config.fatal_error.clone().unwrap()))
        }
        other => Err((
            "action_not_found".into(),
            format!("discord plugin does not handle {other}"),
        )),
    }
}

fn send_response(stdout: &Arc<std::sync::Mutex<std::io::Stdout>>, id: &str, result: Value) {
    let frame = json!({ "id": id, "ok": true, "result": result });
    emit(stdout, &frame.to_string());
}

fn send_error(
    stdout: &Arc<std::sync::Mutex<std::io::Stdout>>,
    id: &str,
    code: &str,
    message: &str,
) {
    let frame = json!({
        "id": id,
        "ok": false,
        "error": { "code": code, "message": message },
    });
    emit(stdout, &frame.to_string());
}

fn emit(stdout: &Arc<std::sync::Mutex<std::io::Stdout>>, line: &str) {
    let mut out = match stdout.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if writeln!(out, "{line}").is_err() {
        return;
    }
    let _ = out.flush();
}
