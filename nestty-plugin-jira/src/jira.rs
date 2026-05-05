//! Atlassian Cloud REST v3 client (slice 16.1: just the credential
//! validation surface). Auth is HTTP Basic with `email:api_token`
//! base64-encoded — same shape every Atlassian Cloud SDK uses.
//!
//! Slice 16.2 will add `search`, `get_issue`, `create_issue`,
//! `transition`, `add_comment`, but the Basic-auth header construction
//! and error-classification stay identical, so they live here.

use serde_json::Value;

/// Returned from `/rest/api/3/myself` on a successful Basic-auth
/// request. Used during `auth` to (a) prove the credentials work and
/// (b) capture the account_id needed for mention detection later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub account_id: String,
    pub display_name: String,
    pub email_address: String,
}

/// Build the `Authorization: Basic <b64(email:api_token)>` header
/// value. Pulled into its own function (rather than inlined) so tests
/// can verify the encoding without making a network call.
pub fn basic_auth_header(email: &str, api_token: &str) -> String {
    let raw = format!("{email}:{api_token}");
    format!("Basic {}", base64_encode(raw.as_bytes()))
}

/// Validate credentials against Jira Cloud by calling `GET /rest/api/3/myself`.
/// Returns the user's accountId/displayName/email on 200, classified
/// error string on any failure.
pub fn validate_credentials(
    base_url: &str,
    email: &str,
    api_token: &str,
) -> Result<UserInfo, String> {
    let url = join_url(base_url, "/rest/api/3/myself");
    let body = http_get(&url, email, api_token)?;
    parse_myself(&body)
}

/// Parse a `/myself` response. Pulled out of the network path so tests
/// can drive it with canonical fixtures.
pub fn parse_myself(body: &Value) -> Result<UserInfo, String> {
    let account_id = body
        .get("accountId")
        .and_then(Value::as_str)
        .ok_or("/myself response missing accountId")?
        .to_string();
    let display_name = body
        .get("displayName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let email_address = body
        .get("emailAddress")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(UserInfo {
        account_id,
        display_name,
        email_address,
    })
}

/// `Authorization: Basic ...` GET, JSON response. Status-code
/// classification matches Slack's bare-snake-case convention so the
/// dispatcher can promote the prefix to a top-level error code.
pub fn http_get(url: &str, email: &str, api_token: &str) -> Result<Value, String> {
    let resp = match ureq::get(url)
        .set("Authorization", &basic_auth_header(email, api_token))
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(classify_status(code, &body, None));
        }
        Err(e) => return Err(format!("transport: {e}")),
    };
    resp.into_json::<Value>()
        .map_err(|e| format!("json parse: {e}"))
}

/// Map HTTP status to a snake_case error prefix. The dispatcher
/// (`main::handle_action`) inspects the leading token before the first
/// space/parens and promotes pure `[a-z_]+` prefixes to the action's
/// `code` field, leaving the rest in `message` — same posture as
/// Slack's response classifier so triggers can pattern-match without
/// substring searches.
pub fn classify_status(code: u16, body: &str, retry_after: Option<&str>) -> String {
    match code {
        401 => format!("unauthorized HTTP 401: {body}"),
        403 => format!("forbidden HTTP 403: {body}"),
        404 => format!("not_found HTTP 404: {body}"),
        429 => match retry_after {
            Some(s) => format!("rate_limited (Retry-After: {s})"),
            None => "rate_limited".to_string(),
        },
        _ => format!("io_error Jira HTTP {code}: {body}"),
    }
}

/// Concatenate a base URL with a `/rest/api/...` suffix without
/// producing `//` in the middle. Both inputs may have or lack a
/// trailing/leading slash; result has exactly one separator.
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

/// RFC 4648 base64 (with `=` padding). Hand-rolled because this is
/// the only base64 use-site in the plugin and pulling in the `base64`
/// crate for ~10 LOC of work isn't worth a dep. Same posture as
/// calendar's hand-rolled `urlencode`.
pub fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        0 => {}
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => unreachable!("chunks_exact(3) remainder is 0..=2"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base64_encode_matches_known_vectors() {
        // RFC 4648 §10 test vectors.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_encode_handles_binary() {
        // Non-ASCII byte sequence (UTF-8 'é' + 'á').
        assert_eq!(base64_encode(b"\xc3\xa9\xc3\xa1"), "w6nDoQ==");
    }

    #[test]
    fn basic_auth_header_format() {
        // Atlassian's docs use this exact example: `fred@example.com:api_token`
        // → `Basic ZnJlZEBleGFtcGxlLmNvbTphcGlfdG9rZW4=`
        assert_eq!(
            basic_auth_header("fred@example.com", "api_token"),
            "Basic ZnJlZEBleGFtcGxlLmNvbTphcGlfdG9rZW4="
        );
    }

    #[test]
    fn join_url_no_double_slash() {
        assert_eq!(
            join_url("https://x.atlassian.net", "/rest/api/3/myself"),
            "https://x.atlassian.net/rest/api/3/myself"
        );
        assert_eq!(
            join_url("https://x.atlassian.net/", "/rest/api/3/myself"),
            "https://x.atlassian.net/rest/api/3/myself"
        );
        assert_eq!(
            join_url("https://x.atlassian.net", "rest/api/3/myself"),
            "https://x.atlassian.net/rest/api/3/myself"
        );
    }

    #[test]
    fn classify_status_promotes_known_codes() {
        assert!(classify_status(401, "Login required", None).starts_with("unauthorized"));
        assert!(classify_status(403, "no perm", None).starts_with("forbidden"));
        assert!(classify_status(404, "no issue", None).starts_with("not_found"));
        assert_eq!(
            classify_status(429, "", Some("30")),
            "rate_limited (Retry-After: 30)"
        );
        assert_eq!(classify_status(429, "", None), "rate_limited");
        assert!(classify_status(500, "boom", None).starts_with("io_error"));
        assert!(classify_status(503, "ish", None).starts_with("io_error"));
    }

    #[test]
    fn parse_myself_returns_user_info() {
        let body = json!({
            "accountId": "5b1234567890",
            "displayName": "Marshall Ku",
            "emailAddress": "marshall@example.com",
            "active": true,
        });
        let user = parse_myself(&body).unwrap();
        assert_eq!(user.account_id, "5b1234567890");
        assert_eq!(user.display_name, "Marshall Ku");
        assert_eq!(user.email_address, "marshall@example.com");
    }

    #[test]
    fn parse_myself_rejects_missing_account_id() {
        let body = json!({ "displayName": "x" });
        assert!(parse_myself(&body).is_err());
    }

    #[test]
    fn parse_myself_tolerates_missing_optional_fields() {
        // displayName and emailAddress are optional in the upstream
        // schema (rare, but we shouldn't crash on a privacy-redacted
        // response).
        let body = json!({ "accountId": "x" });
        let user = parse_myself(&body).unwrap();
        assert_eq!(user.display_name, "");
        assert_eq!(user.email_address, "");
    }
}
