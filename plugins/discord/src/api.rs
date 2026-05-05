//! Discord REST API helpers for write actions.
//!
//! Slice 2 ships only `send_message` (POST `/channels/{id}/messages`).
//! Future: edit/delete, reactions, file uploads, threads.
//!
//! Rate limiting: Discord uses per-route bucket headers (`X-RateLimit-*`)
//! plus a global limit. We don't pre-emptively throttle — we surface
//! HTTP 429 verbatim with the `Retry-After` value so triggers / the
//! UI can decide whether to back off and retry. A future scheduler
//! can layer on top of this without changing the call site.
//!
//! Errors return as `(code, message)` so the plugin's RPC layer can
//! forward `code` directly as the action error code. That preserves
//! Discord's numeric error codes (`discord_50001` for "Missing
//! Access" etc.) at the structured layer instead of burying them in
//! free-text — without that, downstream triggers couldn't branch on
//! specific Discord failures via the action-completion fanout, which
//! only preserves `{code, message}`.

use std::time::Duration;

use serde_json::{Value, json};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Structured failure: `code` is one of `rate_limited`,
/// `discord_<numeric>` (e.g. `discord_50001`), or `io_error` for
/// transport-level failures with no Discord error body.
pub struct ApiError {
    pub code: String,
    pub message: String,
}

/// Map a non-success response into a structured ApiError. Shared
/// between every REST helper so the failure surface stays uniform —
/// `rate_limited` for 429 with Retry-After, `discord_<numeric>` when
/// the body parses as Discord's `{code, message}` shape, `io_error`
/// otherwise.
fn classify_response_error(err: ureq::Error) -> ApiError {
    match err {
        ureq::Error::Status(429, r) => {
            let header_retry = r
                .header("Retry-After")
                .map(str::to_string)
                .unwrap_or_default();
            let body_retry = r
                .into_json::<Value>()
                .ok()
                .and_then(|v| v.get("retry_after").and_then(Value::as_f64))
                .map(|s| format!("{s:.3}"))
                .unwrap_or_default();
            ApiError {
                code: "rate_limited".to_string(),
                message: format!(
                    "Retry-After: {}; body retry_after: {}",
                    if header_retry.is_empty() {
                        "unknown"
                    } else {
                        &header_retry
                    },
                    if body_retry.is_empty() {
                        "unknown"
                    } else {
                        &body_retry
                    }
                ),
            }
        }
        ureq::Error::Status(http_code, r) => {
            let body = r.into_string().unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<Value>(&body)
                && let Some(dc) = v.get("code").and_then(Value::as_u64)
            {
                let dm = v
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("(no message)");
                return ApiError {
                    code: format!("discord_{dc}"),
                    message: format!("HTTP {http_code}: {dm}"),
                };
            }
            ApiError {
                code: "io_error".to_string(),
                message: format!("HTTP {http_code}: {body}"),
            }
        }
        e => ApiError {
            code: "io_error".to_string(),
            message: format!("transport: {e}"),
        },
    }
}

/// Post a plain-text message to a channel or DM. Returns
/// `(message_id, channel_id)` on success — `message_id` is the
/// canonical handle for follow-up edits/reactions.
///
/// `content` must be ≤ 2000 chars (Discord's hard limit). Caller is
/// responsible for truncation; the API rejects over-length with
/// HTTP 400 `BASE_TYPE_MAX_LENGTH`.
pub fn post_message(
    bot_token: &str,
    channel_id: &str,
    content: &str,
) -> Result<(String, String), ApiError> {
    let body = json!({ "content": content });
    let resp = ureq::post(&format!(
        "{DISCORD_API_BASE}/channels/{channel_id}/messages"
    ))
    .set("Authorization", &format!("Bot {bot_token}"))
    .set("User-Agent", "nestty-plugin-discord (nestty, 0.1)")
    .timeout(HTTP_TIMEOUT)
    .send_json(body)
    .map_err(classify_response_error)?;
    let v: Value = resp.into_json().map_err(|e| ApiError {
        code: "io_error".to_string(),
        message: format!("send_message response parse: {e}"),
    })?;
    let message_id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError {
            code: "io_error".to_string(),
            message: "send_message response missing id".to_string(),
        })?
        .to_string();
    let posted_channel = v
        .get("channel_id")
        .and_then(Value::as_str)
        .unwrap_or(channel_id)
        .to_string();
    Ok((message_id, posted_channel))
}

/// `GET /channels/{channel_id}/messages/{message_id}` — fetch a
/// single message. Used by reaction-capture triggers to pull the
/// original message body (reactions don't carry it). Returns the
/// verbatim JSON object so callers can interpolate any field
/// (`content`, `author.id`, `attachments[]`, etc.).
pub fn get_message(bot_token: &str, channel_id: &str, message_id: &str) -> Result<Value, ApiError> {
    let resp = ureq::get(&format!(
        "{DISCORD_API_BASE}/channels/{channel_id}/messages/{message_id}"
    ))
    .set("Authorization", &format!("Bot {bot_token}"))
    .set("User-Agent", "nestty-plugin-discord (nestty, 0.1)")
    .timeout(HTTP_TIMEOUT)
    .call()
    .map_err(classify_response_error)?;
    resp.into_json().map_err(|e| ApiError {
        code: "io_error".to_string(),
        message: format!("get_message response parse: {e}"),
    })
}
