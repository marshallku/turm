//! Network + console records an agent can read, and the limits on them.
//!
//! Shape borrowed from Cursor's browser tool: records go to a **file the agent
//! greps**, not into every tool response, because a verbose per-action dump is
//! what makes browser tooling unaffordable in a context window.
//!
//! Three honesty rules, each of which the reader depends on:
//!
//! 1. **Declared coverage** (codex plan r2-I1). Patching `fetch`/`XHR` is not a
//!    packet log: subresources, `sendBeacon`, WebSocket frames and
//!    service-worker traffic are invisible to it. Every response carries
//!    [`NETLOG_COVERAGE`] so an agent is never misled into concluding "no
//!    request was made" from an empty list.
//! 2. **Redaction is hygiene, not a boundary.** A page can `console.log`
//!    anything, and no name-based filter catches that. The actual boundary is
//!    [`super::TabMode::Protected`], which stops the capture script from being
//!    installed at all (r2-C3) — a suppression at the source, because refusing
//!    reads while the file keeps filling is no defence when the file is
//!    deliberately agent-readable.
//! 3. **Bodies are off by default.** Turning them on is a deliberate act.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// What the capture mechanism actually sees. Returned verbatim in every
/// `webview.net` response.
pub const NETLOG_COVERAGE: &str = "js+navigation";

/// Header names whose values are replaced before a record is written. Matched
/// case-insensitively against the whole name.
pub const REDACT_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "proxy-authorization",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-csrf-token",
];

/// Placeholder written in place of a redacted value. Distinct from an empty
/// string so a reader can tell "absent" from "withheld".
pub const REDACTED: &str = "«redacted»";

/// Longest HTTP method string kept verbatim. A page picks the method
/// (`fetch(url, { method: … })`), so it is page-controlled and bounded.
const MAX_METHOD: usize = 32;

/// Field names whose values are redacted inside a captured body or console
/// line. Matched case-insensitively.
pub const REDACT_FIELDS: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "auth",
    "authorization",
    "client_secret",
    "code",
    "id_token",
    "otp",
    "passwd",
    "password",
    "pwd",
    "refresh_token",
    "secret",
    "session",
    "sig",
    "signature",
    "token",
];

/// `"password": "hunter2"` and `'password': 'hunter2'` in JSON-ish text.
static JSON_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i)(["']({})["']\s*:\s*)(["'])(?:\\.|[^"'\\])*(["'])"#,
        REDACT_FIELDS.join("|")
    ))
    .expect("static field pattern compiles")
});

/// `password=hunter2` in form-encoded or query-shaped text.
static FORM_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("(?i)\\b({})=[^&\\s\"']*", REDACT_FIELDS.join("|")))
        .expect("static field pattern compiles")
});

/// `Bearer <token>` and bare JWTs, which carry no field name to key off.
static BEARERISH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}|\beyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]*")
        .expect("static bearer pattern compiles")
});

/// Redact credential-shaped content inside free text (a captured body, a console
/// line). Returns the markers describing what was hit.
///
/// **Redact THEN clamp.** Truncating first could cut a secret so it no longer
/// matches its pattern and a prefix survives — the same ordering rule
/// `design_grab` follows.
///
/// This is hygiene, not a boundary: `console.log(pw)` has no field name to key
/// off and no pattern to match. The boundary is [`super::TabMode::Protected`],
/// which stops the capture script from being installed at all.
pub fn redact_text(text: &mut String) -> Vec<String> {
    let mut hits = Vec::new();
    let after_json = JSON_FIELD.replace_all(text, |c: &regex::Captures| {
        format!("{}{}{}{}", &c[1], &c[3], REDACTED, &c[4])
    });
    if after_json != *text {
        hits.push("body:field".into());
        *text = after_json.into_owned();
    }
    let after_form =
        FORM_FIELD.replace_all(text, |c: &regex::Captures| format!("{}={REDACTED}", &c[1]));
    if after_form != *text {
        if !hits.iter().any(|h| h == "body:field") {
            hits.push("body:field".into());
        }
        *text = after_form.into_owned();
    }
    let after_bearer = BEARERISH.replace_all(text, REDACTED);
    if after_bearer != *text {
        hits.push("body:bearer".into());
        *text = after_bearer.into_owned();
    }
    hits
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LogCaps {
    /// Whether request/response bodies are captured at all.
    pub capture_bodies: bool,
    /// Bytes of body kept per record when `capture_bodies` is on.
    pub body_truncate: usize,
    /// Hard ceiling on one **serialized** record, enforced by
    /// [`sanitize_net`] / [`sanitize_console`] against the JSON they will
    /// actually write — not against any one field. A page controls URLs, header
    /// values and console text, so budgeting a single field leaves the record as
    /// a whole unbounded (codex review C3).
    pub per_record: usize,
    /// Ring size in records.
    pub ring_records: usize,
    /// Ring size in bytes. Whichever ceiling is hit first evicts oldest-first.
    pub ring_bytes: usize,
}

impl Default for LogCaps {
    fn default() -> Self {
        Self {
            capture_bodies: false,
            body_truncate: 8 * 1024,
            per_record: 32 * 1024,
            ring_records: 2000,
            ring_bytes: 8 * 1024 * 1024,
        }
    }
}

/// How a record reached us — part of keeping [`NETLOG_COVERAGE`] honest.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetSource {
    /// `fetch()` or `XMLHttpRequest`, seen by the injected user script.
    Script,
    /// A main-frame or subframe navigation, seen by the native navigation
    /// delegate rather than by the script.
    Navigation,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct NetRecord {
    pub ts: u64,
    pub tab_id: String,
    pub source: NetSource,
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Names of everything that was withheld from this record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ConsoleRecord {
    pub ts: u64,
    pub tab_id: String,
    pub level: ConsoleLevel,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<String>,
}

/// JSONL sink for one pane. Kept per-pane rather than per-tab so a tab that
/// closes does not orphan a file.
pub fn log_path(panel_id: &str) -> Option<std::path::PathBuf> {
    // Panel ids are UUIDs generated by copad, but they arrive here from the RPC
    // wire, so they are held to the same charset rule as a tab id — no string
    // from an untrusted caller is ever joined as a path unvalidated.
    if !super::tabs::is_valid_tab_id(panel_id) {
        return None;
    }
    Some(
        crate::paths::state_dir()
            .join("browser")
            .join("logs")
            .join(format!("{panel_id}.jsonl")),
    )
}

/// Redact a header list in place, returning the names that were withheld.
pub fn redact_headers(headers: &mut [(String, String)]) -> Vec<String> {
    let mut hit = Vec::new();
    for (name, value) in headers.iter_mut() {
        if REDACT_HEADERS.iter().any(|d| name.eq_ignore_ascii_case(d)) {
            *value = REDACTED.to_string();
            hit.push(name.clone());
        }
    }
    hit
}

/// Truncate a body to the cap on a **char boundary**, so a multi-byte sequence
/// is never cut in half. Returns `true` when it truncated.
pub fn truncate_body(body: &mut String, cap: usize) -> bool {
    if body.len() <= cap {
        return false;
    }
    let mut end = cap;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    body.truncate(end);
    true
}

/// Apply the caps and header redaction to a record before it is written.
/// Returns `false` when the record cannot be brought under
/// [`LogCaps::per_record`] even after shedding everything sheddable — the caller
/// **drops it** rather than writing an over-budget line.
#[must_use]
pub fn sanitize_net(record: &mut NetRecord, caps: &LogCaps) -> bool {
    let mut redacted = redact_headers(&mut record.request_headers);
    redacted.extend(redact_headers(&mut record.response_headers));

    if !caps.capture_bodies {
        if record.body.take().is_some() {
            redacted.push("body".into());
        }
    } else if let Some(body) = record.body.as_mut() {
        // Redact THEN clamp: truncating first could cut a secret below its
        // pattern and leave a prefix behind.
        redacted.extend(redact_text(body));
        if truncate_body(body, caps.body_truncate) {
            redacted.push("body:truncated".into());
        }
    }

    record.redacted.extend(redacted);
    enforce_net_cap(record, caps.per_record)
}

/// Serialized size of a record, or `usize::MAX` if it cannot be serialized (in
/// which case every caller treats it as over budget and sheds).
fn serialized_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|b| b.len())
        .unwrap_or(usize::MAX)
}

/// Shed a net record's page-controlled fields until the JSON it will write fits
/// `cap`, cheapest-to-lose first: body, then headers, then the URL.
///
/// The ts/tab/method/status skeleton is never shed — a record that says only
/// "a POST happened at T and was too big to log" is still useful, whereas a
/// record that silently exceeded the budget is a memory-growth bug.
fn enforce_net_cap(record: &mut NetRecord, cap: usize) -> bool {
    if serialized_len(record) <= cap {
        return true;
    }
    if record.body.take().is_some() {
        record.redacted.push("body:oversize".into());
        if serialized_len(record) <= cap {
            return true;
        }
    }
    if !record.request_headers.is_empty() || !record.response_headers.is_empty() {
        record.request_headers.clear();
        record.response_headers.clear();
        record.redacted.push("headers:oversize".into());
        if serialized_len(record) <= cap {
            return true;
        }
    }
    // A page controls the request method string too (`fetch(url, {method: …})`).
    if record.method.len() > MAX_METHOD {
        truncate_body(&mut record.method, MAX_METHOD);
        if serialized_len(record) <= cap {
            return true;
        }
    }
    if !record.url.is_empty() {
        record.redacted.push("url:truncated".into());
        while serialized_len(record) > cap && !record.url.is_empty() {
            let target = record.url.len() / 2;
            if target == 0 {
                record.url.clear();
            } else {
                truncate_body(&mut record.url, target);
            }
        }
        if serialized_len(record) <= cap {
            return true;
        }
    }
    // `redacted` is itself variable-length — every shed above pushes onto it and
    // a page with many credential headers grows it further. Collapsed LAST, and
    // to one honest marker rather than an arbitrary surviving entry: keeping
    // e.g. "Authorization" while dropping "body:oversize" would misreport what
    // happened to this record.
    if record.redacted.len() > 1 {
        record.redacted = vec!["shed:oversize".into()];
        if serialized_len(record) <= cap {
            return true;
        }
    }
    // Everything sheddable is gone and the skeleton alone still exceeds the cap
    // (an absurdly long `tab_id`, or a cap set below the skeleton size). Refuse
    // rather than write an over-budget line — silently exceeding a documented
    // ceiling is how a log becomes a memory-growth bug.
    false
}

/// Cap a console record's text. Console text is NOT name-redacted — there is no
/// name to key off — which is precisely why protected tabs never install the
/// capture script in the first place.
#[must_use]
pub fn sanitize_console(record: &mut ConsoleRecord, caps: &LogCaps) -> bool {
    let hits = redact_text(&mut record.text);
    record.redacted.extend(hits);
    // Budget against the SERIALIZED record: JSON escaping (a page can emit a
    // string of nothing but quotes and newlines) plus the ts/tab/level metadata
    // both add bytes on top of the raw text, so capping `text` alone leaves the
    // record unbounded (codex review C3/C1).
    if serialized_len(record) <= caps.per_record {
        return true;
    }
    record.redacted.push("text:truncated".into());
    while serialized_len(record) > caps.per_record && !record.text.is_empty() {
        let target = record.text.len() / 2;
        if target == 0 {
            record.text.clear();
        } else {
            truncate_body(&mut record.text, target);
        }
    }
    if serialized_len(record) <= caps.per_record {
        return true;
    }
    // `source` is page-controlled too — a stack frame's script URL. Emptying
    // `text` alone would leave a 40 KiB `source` above the ceiling.
    if record.source.take().is_some() {
        record.redacted.push("source:oversize".into());
        if serialized_len(record) <= caps.per_record {
            return true;
        }
    }
    if record.redacted.len() > 1 {
        record.redacted = vec!["shed:oversize".into()];
        if serialized_len(record) <= caps.per_record {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net() -> NetRecord {
        NetRecord {
            ts: 1,
            tab_id: "t1".into(),
            source: NetSource::Script,
            method: "POST".into(),
            url: "https://api.example.com/login".into(),
            status: Some(200),
            duration_ms: Some(42),
            request_headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Authorization".into(), "Bearer sk-live-abc".into()),
            ],
            response_headers: vec![("Set-Cookie".into(), "session=abc; HttpOnly".into())],
            body: Some("{\"password\":\"hunter2\"}".into()),
            redacted: vec![],
        }
    }

    #[test]
    fn credential_headers_are_withheld_and_named() {
        let mut r = net();
        assert!(sanitize_net(&mut r, &LogCaps::default()));
        assert_eq!(r.request_headers[1].1, REDACTED);
        assert_eq!(r.response_headers[0].1, REDACTED);
        assert!(r.redacted.contains(&"Authorization".to_string()));
        assert!(r.redacted.contains(&"Set-Cookie".to_string()));
        // Non-credential headers survive.
        assert_eq!(r.request_headers[0].1, "application/json");
    }

    #[test]
    fn header_matching_ignores_case() {
        let mut h = vec![
            ("AUTHORIZATION".into(), "x".into()),
            ("cookie".into(), "y".into()),
        ];
        let hit = redact_headers(&mut h);
        assert_eq!(hit.len(), 2);
        assert!(h.iter().all(|(_, v)| v == REDACTED));
    }

    #[test]
    fn bodies_are_dropped_by_default_and_the_drop_is_recorded() {
        let mut r = net();
        assert!(sanitize_net(&mut r, &LogCaps::default()));
        assert!(r.body.is_none());
        assert!(r.redacted.contains(&"body".to_string()));
    }

    #[test]
    fn an_opted_in_body_is_truncated_not_dropped() {
        let caps = LogCaps {
            capture_bodies: true,
            body_truncate: 8,
            ..Default::default()
        };
        let mut r = net();
        assert!(sanitize_net(&mut r, &caps));
        assert_eq!(r.body.as_deref(), Some("{\"passwo"));
        assert!(r.redacted.contains(&"body:truncated".to_string()));
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let mut s = "한글한글".to_string(); // 3 bytes per char
        truncate_body(&mut s, 4);
        assert_eq!(s, "한"); // cut back to the boundary at 3, not 4
        assert!(s.is_char_boundary(s.len()));
    }

    #[test]
    fn a_short_body_is_left_alone() {
        let mut s = "hi".to_string();
        assert!(!truncate_body(&mut s, 8));
        assert_eq!(s, "hi");
    }

    #[test]
    fn an_oversize_net_record_sheds_page_controlled_fields_until_it_fits() {
        // codex review C3: a page controls the URL and the header values, so the
        // documented per-record ceiling has to hold against the serialized JSON.
        let caps = LogCaps {
            capture_bodies: true,
            per_record: 300,
            ..Default::default()
        };
        let mut r = net();
        r.url = format!("https://api.example.com/{}", "x".repeat(4000));
        r.request_headers.push(("X-Trace".into(), "y".repeat(4000)));
        assert!(sanitize_net(&mut r, &caps));
        assert!(
            serde_json::to_vec(&r).unwrap().len() <= caps.per_record,
            "record still {} bytes",
            serde_json::to_vec(&r).unwrap().len()
        );
        // The skeleton survives, so the record still says a POST happened.
        assert_eq!(r.method, "POST");
        assert!(
            r.redacted
                .iter()
                .any(|x| x.ends_with(":oversize") || x == "url:truncated")
        );
    }

    #[test]
    fn a_normal_record_is_not_shed_by_the_cap() {
        let mut r = net();
        assert!(sanitize_net(&mut r, &LogCaps::default()));
        assert_eq!(r.url, "https://api.example.com/login");
        assert_eq!(r.request_headers.len(), 2);
        assert!(!r.redacted.iter().any(|x| x.ends_with(":oversize")));
    }

    #[test]
    fn console_escaping_cannot_push_a_record_over_the_cap() {
        // A page emitting nothing but quotes doubles under JSON escaping.
        let caps = LogCaps {
            per_record: 120,
            ..Default::default()
        };
        let mut c = ConsoleRecord {
            ts: 1,
            tab_id: "t1".into(),
            level: ConsoleLevel::Log,
            text: "\"".repeat(400),
            source: None,
            redacted: vec![],
        };
        assert!(sanitize_console(&mut c, &caps));
        assert!(serde_json::to_vec(&c).unwrap().len() <= caps.per_record);
        assert!(c.redacted.contains(&"text:truncated".to_string()));
    }

    #[test]
    fn console_text_is_capped_and_flagged() {
        // The cap is against the SERIALIZED record, so the budget left for text
        // is whatever the ts/tab/level skeleton does not already consume.
        let caps = LogCaps {
            per_record: 100,
            ..Default::default()
        };
        let mut c = ConsoleRecord {
            ts: 1,
            tab_id: "t1".into(),
            level: ConsoleLevel::Log,
            text: "a".repeat(500),
            source: None,
            redacted: vec![],
        };
        assert!(sanitize_console(&mut c, &caps));
        assert!(serde_json::to_vec(&c).unwrap().len() <= caps.per_record);
        assert!(
            !c.text.is_empty(),
            "a 100-byte budget should still fit some text"
        );
        assert!(c.redacted.contains(&"text:truncated".to_string()));
    }

    #[test]
    fn a_console_record_within_budget_is_untouched() {
        let mut c = ConsoleRecord {
            ts: 1,
            tab_id: "t1".into(),
            level: ConsoleLevel::Warn,
            text: "hello".into(),
            source: None,
            redacted: vec![],
        };
        assert!(sanitize_console(&mut c, &LogCaps::default()));
        assert_eq!(c.text, "hello");
        assert!(c.redacted.is_empty());
    }

    #[test]
    fn log_path_refuses_a_panel_id_that_could_escape_the_log_dir() {
        assert!(log_path("../../etc/passwd").is_none());
        assert!(log_path("panel1").is_some());
    }

    #[test]
    fn an_oversize_console_source_is_shed_after_the_text() {
        // codex review C1: emptying `text` alone leaves a page-controlled
        // `source` above the ceiling.
        let caps = LogCaps {
            per_record: 200,
            ..Default::default()
        };
        let mut c = ConsoleRecord {
            ts: 1,
            tab_id: "t1".into(),
            level: ConsoleLevel::Error,
            text: "boom".into(),
            source: Some("x".repeat(40_000)),
            redacted: vec![],
        };
        assert!(sanitize_console(&mut c, &caps));
        assert!(serde_json::to_vec(&c).unwrap().len() <= caps.per_record);
        assert!(c.source.is_none());
    }

    #[test]
    fn a_page_controlled_method_and_redaction_list_are_bounded_too() {
        let caps = LogCaps {
            per_record: 260,
            ..Default::default()
        };
        let mut r = net();
        r.method = "M".repeat(5000);
        r.url = "https://e.com/".into();
        for i in 0..200 {
            r.request_headers
                .push((format!("Authorization{i}"), "z".into()));
        }
        assert!(sanitize_net(&mut r, &caps));
        assert!(serde_json::to_vec(&r).unwrap().len() <= caps.per_record);
        assert!(r.method.len() <= MAX_METHOD);
    }

    #[test]
    fn a_record_that_cannot_be_shed_under_the_cap_is_rejected_not_written() {
        // The skeleton alone exceeds an absurd cap: refuse rather than write an
        // over-budget line.
        let caps = LogCaps {
            per_record: 1,
            ..Default::default()
        };
        let mut r = net();
        assert!(!sanitize_net(&mut r, &caps));

        let mut c = ConsoleRecord {
            ts: 1,
            tab_id: "t1".into(),
            level: ConsoleLevel::Log,
            text: "x".into(),
            source: None,
            redacted: vec![],
        };
        assert!(!sanitize_console(&mut c, &caps));
    }

    #[test]
    fn an_opted_in_body_has_credential_shaped_content_redacted() {
        // codex review C2: truncation alone left a short
        // `{"password":"hunter2"}` body intact in an agent-readable file.
        let caps = LogCaps {
            capture_bodies: true,
            ..Default::default()
        };
        let mut r = net();
        assert!(sanitize_net(&mut r, &caps));
        let body = r.body.as_deref().unwrap();
        assert!(!body.contains("hunter2"), "{body}");
        assert!(body.contains(REDACTED), "{body}");
        assert!(r.redacted.contains(&"body:field".to_string()));
    }

    #[test]
    fn form_encoded_and_bearer_shaped_content_is_redacted_too() {
        let mut s = "user=me&password=hunter2 Authorization: Bearer sk-live-abcdefghij".to_string();
        let hits = redact_text(&mut s);
        assert!(!s.contains("hunter2"), "{s}");
        assert!(!s.contains("sk-live-abcdefghij"), "{s}");
        assert!(hits.contains(&"body:field".to_string()));
        assert!(hits.contains(&"body:bearer".to_string()));
        // The field NAME survives so the record still says what was withheld.
        assert!(s.contains("password="), "{s}");
        assert!(s.contains("user=me"), "{s}");
    }

    #[test]
    fn a_bare_jwt_with_no_field_name_is_still_caught() {
        let mut s = "token is eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdef".to_string();
        let hits = redact_text(&mut s);
        assert!(!s.contains("eyJhbGci"), "{s}");
        assert!(hits.contains(&"body:bearer".to_string()));
    }

    #[test]
    fn ordinary_text_is_left_completely_alone() {
        let mut s = "GET /users/42 returned 3 rows in 12ms".to_string();
        let before = s.clone();
        assert!(redact_text(&mut s).is_empty());
        assert_eq!(s, before);
    }

    #[test]
    fn console_lines_get_the_same_hygiene_as_bodies() {
        let mut c = ConsoleRecord {
            ts: 1,
            tab_id: "t1".into(),
            level: ConsoleLevel::Log,
            text: "{\"password\":\"hunter2\"}".into(),
            source: None,
            redacted: vec![],
        };
        assert!(sanitize_console(&mut c, &LogCaps::default()));
        assert!(!c.text.contains("hunter2"), "{}", c.text);
    }

    #[test]
    fn coverage_is_a_declared_constant_so_the_readout_cannot_overclaim() {
        assert_eq!(NETLOG_COVERAGE, "js+navigation");
    }

    #[test]
    fn defaults_keep_bodies_off() {
        assert!(!LogCaps::default().capture_bodies);
    }
}
