//! Env-based configuration for the Discord plugin.
//!
//! - `TURM_DISCORD_BOT_TOKEN` — bot token from
//!   <https://discord.com/developers/applications> → Bot tab → "Reset
//!   Token". Required for `auth` subcommand; optional for the long-
//!   running RPC mode (we read from the keyring there). Discord bot
//!   tokens don't have a documented prefix; we accept anything
//!   non-empty and let the `users/@me` validation catch typos.
//! - `TURM_DISCORD_WORKSPACE` — keyring-entry namespace. Default
//!   `"default"`. Same role as `TURM_SLACK_WORKSPACE`: lets one
//!   account host multiple bots whose keyring entries don't collide.
//! - `TURM_DISCORD_REQUIRE_SECURE_STORE` — `1` / `true` refuses to
//!   fall back to plaintext when the OS keyring is unavailable. Same
//!   posture as the Slack plugin's flag.
//!
//! `from_env` never errors: invalid values land in `fatal_error` and
//! the plugin still completes the `initialize` handshake so the
//! supervisor's `provides` resolution succeeds. Every action then
//! short-circuits to `config_error` instead of running on a half-
//! validated config.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub workspace_label: String,
    /// Bot token from env, if supplied. The keyring-stored value is
    /// authoritative at runtime — the env value is only used by the
    /// `auth` subcommand to seed the store. None at runtime is the
    /// expected case once `auth` has been run once.
    pub bot_token_env: Option<String>,
    pub plaintext_path: PathBuf,
    pub require_secure_store: bool,
    pub fatal_error: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let mut errors: Vec<String> = Vec::new();
        let workspace_label = std::env::var("TURM_DISCORD_WORKSPACE")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "default".to_string());
        if let Err(e) = validate_workspace_label(&workspace_label) {
            errors.push(format!("TURM_DISCORD_WORKSPACE: {e}"));
        }
        let bot_token_env = std::env::var("TURM_DISCORD_BOT_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        let require_secure_store = std::env::var("TURM_DISCORD_REQUIRE_SECURE_STORE")
            .ok()
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes"))
            .unwrap_or(false);
        let plaintext_path = default_plaintext_path(&workspace_label);
        let fatal_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        };
        Self {
            workspace_label,
            bot_token_env,
            plaintext_path,
            require_secure_store,
            fatal_error,
        }
    }
}

fn default_plaintext_path(workspace_label: &str) -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("turm")
        .join(format!("discord-token-{workspace_label}.json"))
}

/// Workspace labels become keyring entry names. Same charset as the
/// KB / Todo plugins so a stray `..` can't escape into other
/// keyring namespaces.
fn validate_workspace_label(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("workspace label cannot be empty".to_string());
    }
    if s == "." || s == ".." {
        return Err(format!("{s:?} is reserved"));
    }
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@');
        if !ok {
            return Err(format!(
                "invalid character {c:?} (allowed: ASCII alphanumeric and _ - . @)"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_label_rejects_bad() {
        for s in ["", "..", "a/b", "x\0y", "/etc"] {
            assert!(
                validate_workspace_label(s).is_err(),
                "expected reject {s:?}"
            );
        }
    }

    #[test]
    fn workspace_label_accepts_normal() {
        for s in ["default", "team@acme.com", "a.b-c_d"] {
            validate_workspace_label(s).unwrap_or_else(|e| panic!("{s:?} rejected: {e}"));
        }
    }
}
