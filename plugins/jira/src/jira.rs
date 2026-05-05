//! Atlassian Cloud REST v3 client. Auth is HTTP Basic with
//! `email:api_token` base64-encoded — same shape every Atlassian Cloud
//! SDK uses. Same client serves both `auth` (one-shot `/myself`) and
//! the polling loop / action surface.
//!
//! Bodies destined for `description` / `comment` fields are wrapped in
//! ADF (Atlassian Document Format) — Jira Cloud rejected the legacy
//! Wiki Markup format on 2020-06 and now requires structured JSON.
//! ADF helpers live here so all callers (actions + future trigger
//! consumers) get a consistent wrap shape.

use serde_json::{Value, json};

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

/// Resolved credentials (env- or store-sourced). Threaded through every
/// API call so the caller doesn't have to remember which arg goes where.
/// Borrowed (`&str`) rather than owned because the caller already holds
/// the strings on the stack — no need for the helpers to clone.
#[derive(Debug, Clone, Copy)]
pub struct Creds<'a> {
    pub base_url: &'a str,
    pub email: &'a str,
    pub api_token: &'a str,
}

/// `Authorization: Basic ...` GET, JSON response. Status-code
/// classification matches Slack's bare-snake-case convention so the
/// dispatcher can promote the prefix to a top-level error code.
/// Captures `Retry-After` on 429 so callers can branch on it
/// (currently surfaced verbatim in the error suffix).
pub fn http_get(url: &str, email: &str, api_token: &str) -> Result<Value, String> {
    let resp = match ureq::get(url)
        .set("Authorization", &basic_auth_header(email, api_token))
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let retry_after = r.header("Retry-After").map(str::to_string);
            let body = r.into_string().unwrap_or_default();
            return Err(classify_status(code, &body, retry_after.as_deref()));
        }
        Err(e) => return Err(format!("transport: {e}")),
    };
    resp.into_json::<Value>()
        .map_err(|e| format!("json parse: {e}"))
}

/// `Authorization: Basic ...` POST with a JSON body. Returns the
/// parsed response body on 2xx; on 204 No Content (transition succeeds
/// with no payload) returns `Value::Null` so callers can ignore the
/// shape. Same status-code classification as `http_get`.
pub fn http_post(creds: Creds, url: &str, body: &Value) -> Result<Value, String> {
    let resp = match ureq::post(url)
        .set(
            "Authorization",
            &basic_auth_header(creds.email, creds.api_token),
        )
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send_json(body.clone())
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let retry_after = r.header("Retry-After").map(str::to_string);
            let resp_body = r.into_string().unwrap_or_default();
            return Err(classify_status(code, &resp_body, retry_after.as_deref()));
        }
        Err(e) => return Err(format!("transport: {e}")),
    };
    if resp.status() == 204 {
        return Ok(Value::Null);
    }
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

// ============================================================
//                     High-level API methods
// ============================================================

/// Result of a paginated `/rest/api/3/search/jql` call.
///
/// **API endpoint note**: Atlassian deprecated the legacy
/// `GET /rest/api/3/search` in 2024 with sunset by 2025; new code
/// must use `POST /rest/api/3/search/jql` which uses cursor-based
/// pagination (nextPageToken) instead of offset-based (startAt).
/// The new endpoint also drops the `total` field from responses
/// since cursor pagination doesn't need it.
#[derive(Debug)]
pub struct SearchResponse {
    pub issues: Vec<Value>,
    /// Cursor for the NEXT page. `None` when no more pages
    /// (`isLast: true` in the response, or the field is absent).
    pub next_page_token: Option<String>,
}

/// Run `POST /rest/api/3/search/jql` with the given JQL, paginated
/// by `next_page_token`. Pass `None` for the first page; pass the
/// previous response's `next_page_token` for subsequent pages.
///
/// `fields` filters the response shape — passing the explicit list
/// the caller actually uses cuts payload size by ~80% in typical
/// cases. Pass `["*all"]` to get every field (including customs).
pub fn search(
    creds: Creds,
    jql: &str,
    next_page_token: Option<&str>,
    max_results: u64,
    fields: &[&str],
) -> Result<SearchResponse, String> {
    let url = format!(
        "{}/rest/api/3/search/jql",
        creds.base_url.trim_end_matches('/')
    );
    let mut body = serde_json::Map::new();
    body.insert("jql".to_string(), json!(jql));
    body.insert("maxResults".to_string(), json!(max_results));
    if !fields.is_empty() {
        body.insert(
            "fields".to_string(),
            json!(fields.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        );
    }
    if let Some(token) = next_page_token {
        body.insert("nextPageToken".to_string(), json!(token));
    }
    let resp = http_post(creds, &url, &Value::Object(body))?;
    parse_search_response(&resp)
}

/// Pulled out of the network path so tests can drive it with canonical
/// fixtures. The new `/search/jql` endpoint signals "more pages exist"
/// via the presence of `nextPageToken` (and `isLast: false`); when
/// `isLast: true` or the token is absent, we're done.
pub fn parse_search_response(body: &Value) -> Result<SearchResponse, String> {
    let issues = body
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // `isLast` is the canonical signal in the new API. Fall back to
    // checking for nextPageToken presence when `isLast` is absent
    // (defensive — Atlassian's docs hint they may simplify the
    // response shape further).
    let is_last = body.get("isLast").and_then(Value::as_bool).unwrap_or(false);
    let next_page_token = if is_last {
        None
    } else {
        body.get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    Ok(SearchResponse {
        issues,
        next_page_token,
    })
}

/// Get a single issue by key. Returns the full payload — all fields
/// (including customs) plus changelog. The verbatim Jira shape, NOT
/// the trigger-envelope shape (action callers who want envelope can
/// use `list_my_tickets` which returns one envelope per ticket).
///
/// **Comment cap caveat**: Jira's `/issue/{key}` endpoint always
/// returns up to 50 comments inline regardless of `fields=` — there
/// is no `?maxComments` knob. Callers needing >50 comments would
/// fetch the rest via the `/issue/{key}/comment` paginated endpoint
/// (currently only the poller uses it; a dedicated `jira.get_comments`
/// action could land in a follow-up if needed).
pub fn get_issue(creds: Creds, key: &str) -> Result<Value, String> {
    validate_issue_key(key)?;
    let url = format!(
        "{}/rest/api/3/issue/{}?fields=*all&expand=changelog,renderedFields",
        creds.base_url.trim_end_matches('/'),
        urlencode(key),
    );
    http_get(&url, creds.email, creds.api_token)
}

/// Create an issue. `description` is plain text and gets ADF-wrapped
/// here. `assignee` is an Atlassian accountId (NOT email — Jira Cloud
/// dropped email-based assignment in 2018 for GDPR compliance).
/// `issue_type` is required for company-managed (classic) projects and
/// surfaces as Jira's `400 issuetype is required` otherwise; defaults
/// to `"Task"` when omitted because that's the universal fallback every
/// out-of-the-box project schema accepts. Caller can pass `"Bug"`,
/// `"Story"`, `"Subtask"`, or any custom type configured in their
/// project. Team-managed (next-gen) projects without a "Task" type
/// may still 400 — the error surfaces verbatim so the user knows to
/// pass `issue_type` explicitly.
pub fn create_issue(
    creds: Creds,
    project_key: &str,
    summary: &str,
    description: Option<&str>,
    assignee_account_id: Option<&str>,
    parent_key: Option<&str>,
    issue_type: Option<&str>,
) -> Result<Value, String> {
    validate_project_key(project_key)?;
    if let Some(p) = parent_key {
        validate_issue_key(p)?;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("project".to_string(), json!({ "key": project_key }));
    fields.insert("summary".to_string(), json!(summary));
    fields.insert(
        "issuetype".to_string(),
        json!({ "name": issue_type.unwrap_or("Task") }),
    );
    if let Some(desc) = description {
        fields.insert("description".to_string(), text_to_adf(desc));
    }
    if let Some(acct) = assignee_account_id {
        fields.insert("assignee".to_string(), json!({ "accountId": acct }));
    }
    if let Some(parent) = parent_key {
        fields.insert("parent".to_string(), json!({ "key": parent }));
    }
    let body = json!({ "fields": Value::Object(fields) });
    let url = format!("{}/rest/api/3/issue", creds.base_url.trim_end_matches('/'),);
    http_post(creds, &url, &body)
}

/// List the available transitions for an issue. Returned shape:
/// `[{id, name, to: {name, id}}]` — minimum needed for the
/// case-insensitive name lookup in `transition()`.
pub fn list_transitions(creds: Creds, key: &str) -> Result<Vec<Value>, String> {
    validate_issue_key(key)?;
    let url = format!(
        "{}/rest/api/3/issue/{}/transitions",
        creds.base_url.trim_end_matches('/'),
        urlencode(key),
    );
    let body = http_get(&url, creds.email, creds.api_token)?;
    Ok(body
        .get("transitions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Transition an issue. Looks up the matching transition by destination
/// status name (case-insensitive) so callers can pass `"In Progress"`
/// without knowing Jira's internal numeric transition ids (which differ
/// per workflow). Returns the resolved (from, to) pair on success;
/// surfaces `transition_not_available` when the requested status name
/// has no matching transition (most common cause: the user lacks the
/// permission OR the workflow doesn't allow that step from the current
/// state).
pub fn transition(
    creds: Creds,
    key: &str,
    target_status_name: &str,
) -> Result<(String, String), String> {
    validate_issue_key(key)?;
    let issue = get_issue(creds, key)?;
    let from_status = issue
        .get("fields")
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let transitions = list_transitions(creds, key)?;
    let target_lower = target_status_name.to_ascii_lowercase();
    let matched = transitions.iter().find(|t| {
        t.get("to")
            .and_then(|to| to.get("name"))
            .and_then(Value::as_str)
            .map(|n| n.to_ascii_lowercase() == target_lower)
            .unwrap_or(false)
    });
    let Some(matched) = matched else {
        let available: Vec<String> = transitions
            .iter()
            .filter_map(|t| {
                t.get("to")
                    .and_then(|to| to.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        return Err(format!(
            "transition_not_available no transition to {target_status_name:?} from current state. \
             Available targets: {available:?}"
        ));
    };
    let trans_id = matched
        .get("id")
        .and_then(Value::as_str)
        .ok_or("transition response missing id")?;
    let resolved_to = matched
        .get("to")
        .and_then(|to| to.get("name"))
        .and_then(Value::as_str)
        .unwrap_or(target_status_name)
        .to_string();
    let url = format!(
        "{}/rest/api/3/issue/{}/transitions",
        creds.base_url.trim_end_matches('/'),
        urlencode(key),
    );
    let body = json!({ "transition": { "id": trans_id } });
    http_post(creds, &url, &body)?;
    Ok((from_status, resolved_to))
}

/// Add a plain-text comment. Body is ADF-wrapped automatically.
/// Returns the new comment's id (Jira's response carries the full
/// comment object; we hand back just the id since that's the dedup
/// key the rest of the system uses).
pub fn add_comment(creds: Creds, key: &str, body_text: &str) -> Result<String, String> {
    validate_issue_key(key)?;
    let url = format!(
        "{}/rest/api/3/issue/{}/comment",
        creds.base_url.trim_end_matches('/'),
        urlencode(key),
    );
    let body = json!({ "body": text_to_adf(body_text) });
    let resp = http_post(creds, &url, &body)?;
    let id = resp
        .get("id")
        .and_then(Value::as_str)
        .ok_or("comment response missing id")?
        .to_string();
    Ok(id)
}

// ============================================================
//                     Validators / encoders
// ============================================================

/// Issue keys are `[A-Z][A-Z0-9_]*-[0-9]+` per Jira's documentation.
/// Validate so a malformed key (or an attempt to splice a path
/// fragment via `key = "FOO/../bar"`) can't escape into the request
/// URL. Same trust-boundary posture as Slack's `is_valid_slack_id`.
pub fn validate_issue_key(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("issue key: cannot be empty".to_string());
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_uppercase() {
        return Err(format!(
            "issue key {s:?}: must start with an uppercase ASCII letter"
        ));
    }
    let dash_pos = s.find('-').ok_or_else(|| {
        format!("issue key {s:?}: must contain a hyphen separating project and number")
    })?;
    if dash_pos == 0 || dash_pos == s.len() - 1 {
        return Err(format!(
            "issue key {s:?}: hyphen must separate non-empty project and number"
        ));
    }
    let (proj, rest) = s.split_at(dash_pos);
    let num = &rest[1..];
    for &b in proj.as_bytes() {
        if !(b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
            return Err(format!(
                "issue key {s:?}: project segment contains invalid character"
            ));
        }
    }
    for &b in num.as_bytes() {
        if !b.is_ascii_digit() {
            return Err(format!(
                "issue key {s:?}: number segment must be ASCII digits"
            ));
        }
    }
    Ok(())
}

/// Project keys are `[A-Z][A-Z0-9_]*`. Same trust-boundary defense
/// as `validate_issue_key`. Mirrors the env-time check in
/// `config::validate_project_key` (same rules) but lives here so the
/// `create_issue` helper can validate without depending on `config`.
pub fn validate_project_key(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("project key: cannot be empty".to_string());
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_uppercase() {
        return Err(format!(
            "project key {s:?}: must start with an uppercase ASCII letter"
        ));
    }
    for &b in bytes {
        if !(b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
            return Err(format!(
                "project key {s:?}: invalid character (allowed: A-Z, 0-9, _)"
            ));
        }
    }
    Ok(())
}

/// Wrap a plain-text string in Atlassian Document Format. Jira Cloud
/// rejected legacy Wiki Markup format on 2020-06; `description` and
/// `comment.body` now require ADF. Each non-empty line of input
/// becomes its own `paragraph` node — Jira renders `\n\n` as the
/// natural paragraph break, but we pre-split so single newlines
/// produce visible breaks too (matching how a user types in vim/an
/// editor expects).
///
/// Empty input maps to a single empty paragraph rather than an empty
/// document (Jira's REST validates that `content` has at least one
/// node — an empty doc returns 400).
pub fn text_to_adf(text: &str) -> Value {
    let mut content = Vec::new();
    if text.is_empty() {
        content.push(json!({ "type": "paragraph" }));
    } else {
        for line in text.split('\n') {
            if line.is_empty() {
                content.push(json!({ "type": "paragraph" }));
            } else {
                content.push(json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": line }]
                }));
            }
        }
    }
    json!({
        "version": 1,
        "type": "doc",
        "content": content,
    })
}

/// Minimal RFC 3986 percent-encode for query/path segments. Same
/// posture as calendar's hand-rolled `urlencode` — we control all
/// inputs (validated keys, JQL we built ourselves, integer offsets)
/// so there's no exotic-byte concern that would justify pulling in
/// the `url::form_urlencoded` crate.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

/// Build the JQL string the polling loop uses. Shape:
/// `(assignee = currentUser() OR watcher = currentUser()) AND updated > -<n>h[ AND project in (X, Y)]`.
/// Project keys go through `validate_project_key` at config load
/// (`parse_projects`) so the values spliced here are guaranteed
/// charset-safe — no JQL injection risk from user-supplied content.
pub fn build_polling_jql(lookback_hours: u32, projects: Option<&[String]>) -> String {
    let mut jql = format!(
        "(assignee = currentUser() OR watcher = currentUser()) AND updated > -{lookback_hours}h"
    );
    if let Some(list) = projects
        && !list.is_empty()
    {
        jql.push_str(" AND project in (");
        for (i, p) in list.iter().enumerate() {
            if i > 0 {
                jql.push_str(", ");
            }
            jql.push_str(p);
        }
        jql.push(')');
    }
    jql.push_str(" ORDER BY updated DESC");
    jql
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

    #[test]
    fn validate_issue_key_accepts_canonical_shapes() {
        for s in ["PROJ-1", "APP-12345", "I_OS-42", "X1Y2-7"] {
            validate_issue_key(s).unwrap_or_else(|e| panic!("{s:?} rejected: {e}"));
        }
    }

    #[test]
    fn validate_issue_key_rejects_bad_shapes() {
        for bad in [
            "",
            "proj-1",     // lowercase project
            "1PROJ-1",    // starts with digit
            "PROJ",       // no hyphen
            "PROJ-",      // no number
            "-1",         // no project
            "PROJ-1a",    // non-digit number
            "PROJ/1",     // wrong separator (would otherwise allow path injection)
            "PROJ-1/foo", // path injection attempt
            "PROJ-1?x=1", // query string injection
        ] {
            assert!(validate_issue_key(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn validate_project_key_matches_config_charset() {
        validate_project_key("PROJ").unwrap();
        validate_project_key("APP_X").unwrap();
        validate_project_key("X1").unwrap();
        assert!(validate_project_key("proj").is_err());
        assert!(validate_project_key("1PROJ").is_err());
        assert!(validate_project_key("PROJ-X").is_err());
    }

    #[test]
    fn urlencode_passes_unreserved_chars() {
        assert_eq!(urlencode("PROJ-1"), "PROJ-1");
        assert_eq!(urlencode("abc-XYZ_123.~"), "abc-XYZ_123.~");
    }

    #[test]
    fn urlencode_escapes_jql_specials() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("a=b"), "a%3Db");
        assert_eq!(urlencode("("), "%28");
        assert_eq!(urlencode(","), "%2C");
        assert_eq!(urlencode("é"), "%C3%A9");
    }

    #[test]
    fn build_polling_jql_no_projects() {
        let jql = build_polling_jql(24, None);
        assert!(jql.contains("currentUser()"));
        assert!(jql.contains("updated > -24h"));
        assert!(!jql.contains("project in"));
        assert!(jql.contains("ORDER BY updated DESC"));
    }

    #[test]
    fn build_polling_jql_with_projects() {
        let projects = vec!["APP".to_string(), "PROJ".to_string()];
        let jql = build_polling_jql(48, Some(&projects));
        assert!(jql.contains("AND project in (APP, PROJ)"));
        assert!(jql.contains("updated > -48h"));
    }

    #[test]
    fn build_polling_jql_empty_projects_omits_clause() {
        let projects: Vec<String> = vec![];
        let jql = build_polling_jql(24, Some(&projects));
        assert!(!jql.contains("project in"));
    }

    #[test]
    fn text_to_adf_wraps_single_line() {
        let v = text_to_adf("hello world");
        assert_eq!(v["type"], "doc");
        assert_eq!(v["version"], 1);
        let content = v["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "paragraph");
        assert_eq!(content[0]["content"][0]["text"], "hello world");
    }

    #[test]
    fn text_to_adf_splits_on_newline() {
        let v = text_to_adf("line one\nline two\n\nline three");
        let content = v["content"].as_array().unwrap();
        // 4 paragraphs: line one, line two, empty (between \n\n), line three
        assert_eq!(content.len(), 4);
        assert_eq!(content[0]["content"][0]["text"], "line one");
        assert_eq!(content[1]["content"][0]["text"], "line two");
        assert!(content[2].get("content").is_none()); // empty paragraph
        assert_eq!(content[3]["content"][0]["text"], "line three");
    }

    #[test]
    fn text_to_adf_empty_produces_empty_paragraph() {
        // Jira's REST validates content has ≥1 node; empty doc 400s.
        let v = text_to_adf("");
        let content = v["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "paragraph");
    }

    #[test]
    fn parse_search_response_with_next_page_token() {
        let body = json!({
            "issues": vec![json!({"key":"A"}); 50],
            "nextPageToken": "tok-2",
            "isLast": false,
        });
        let resp = parse_search_response(&body).unwrap();
        assert_eq!(resp.issues.len(), 50);
        assert_eq!(resp.next_page_token.as_deref(), Some("tok-2"));
    }

    #[test]
    fn parse_search_response_is_last_returns_no_token() {
        let body = json!({
            "issues": vec![json!({"key":"A"}); 20],
            "isLast": true,
            "nextPageToken": "ignored-when-isLast-true",
        });
        let resp = parse_search_response(&body).unwrap();
        assert_eq!(resp.next_page_token, None);
    }

    #[test]
    fn parse_search_response_no_token_no_is_last_treated_as_last() {
        // Defensive against a response shape that omits both fields
        // (Atlassian doc hints they may simplify further).
        let body = json!({
            "issues": Vec::<Value>::new(),
        });
        let resp = parse_search_response(&body).unwrap();
        assert_eq!(resp.next_page_token, None);
        assert!(resp.issues.is_empty());
    }

    #[test]
    fn parse_search_response_empty_issues_ok() {
        let body = json!({
            "issues": Vec::<Value>::new(),
            "isLast": true,
        });
        let resp = parse_search_response(&body).unwrap();
        assert!(resp.issues.is_empty());
    }
}
