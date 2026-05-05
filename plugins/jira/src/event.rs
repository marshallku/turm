//! Ticket model + ADF utilities.
//!
//! `Ticket` is the in-memory shape derived from a Jira `/search` or
//! `/issue/<key>` response. It's deliberately a flat struct (not a
//! re-export of the raw JSON) so `to_payload_json` can produce a
//! stable wire shape that triggers can interpolate `{event.X}` against
//! without worrying about which field nests where in Jira's response.
//!
//! ADF (Atlassian Document Format) walking lives here too because
//! it's the same logic used in two places: `adf_to_plain_text` for
//! the `comment.body` field of `jira.comment_added` events, and
//! `adf_contains_mention_of` for detecting `jira.mention` events.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone)]
pub struct Ticket {
    pub key: String,
    pub summary: String,
    pub status_name: String,
    pub assignee_account_id: Option<String>,
    pub assignee_display: Option<String>,
    pub reporter_account_id: Option<String>,
    pub project_key: String,
    /// Direct browser-friendly URL: `<base>/browse/<key>`.
    pub url: String,
    pub updated: DateTime<Utc>,
    /// The verbatim issue JSON. Carried through to event payloads so
    /// triggers / `nestctl` can reach uncommonly-needed fields without
    /// us having to model every Jira field.
    pub raw_json: Value,
}

/// One snapshot per ticket, kept by the poller across ticks.
/// Diffing against the prior snapshot is what produces the four
/// event kinds. We keep the raw `updated` string from Jira so dedup
/// can use it as a stable per-update discriminator without
/// round-tripping through chrono.
///
/// `last_comment_created_iso` is the high-water mark of comment
/// `created` timestamps we've already emitted events for — a
/// timestamp-based watermark rather than a count, because Jira's
/// comment indices SHIFT when a comment is deleted. A count-based
/// `startAt` would skip new comments after a deletion (the new
/// comment now sits at offset N-1 instead of N, but `startAt=N`
/// passes right over it). Watermark-based filtering is robust
/// against arbitrary insertion/deletion patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketSnapshot {
    pub updated_iso: String,
    pub status_name: String,
    pub assignee_account_id: Option<String>,
    /// Latest `created` timestamp of any comment we've emitted
    /// events for. None on first sight — first sight does NOT emit
    /// historical comment events; it just records the high water
    /// mark from the inline comments so the next tick has a baseline.
    pub last_comment_created_iso: Option<String>,
}

impl Ticket {
    /// Snapshot the non-comment fields. The poller fills in
    /// `last_comment_created_iso` separately from
    /// process_new_comments since it needs to walk the inline
    /// comments to know the high water mark.
    pub fn snapshot_without_comments(&self) -> TicketSnapshot {
        TicketSnapshot {
            updated_iso: self.updated.to_rfc3339(),
            status_name: self.status_name.clone(),
            assignee_account_id: self.assignee_account_id.clone(),
            last_comment_created_iso: None,
        }
    }
}

/// Walk an issue's inline `fields.comment.comments[]` (returned by
/// `*all` searches) and return the parsed comments in chronological
/// order. Jira returns them oldest-first by default. Empty when
/// `comment` field is absent or malformed.
pub fn extract_inline_comments(raw: &Value) -> Vec<Comment> {
    let comments_arr = raw
        .get("fields")
        .and_then(|f| f.get("comment"))
        .and_then(|c| c.get("comments"))
        .and_then(Value::as_array);
    comments_arr
        .map(|arr| arr.iter().filter_map(parse_comment).collect())
        .unwrap_or_default()
}

/// Map a Jira `/search` or `/issue/<key>` response (a single issue)
/// into our `Ticket` struct. `base_url` is needed to build the browser
/// URL; the Jira API doesn't include a `/browse/<key>` link in the
/// response. Returns `None` when essential fields are missing — a
/// malformed issue should drop quietly rather than poison the whole
/// tick.
pub fn from_jira_json(raw: &Value, base_url: &str) -> Option<Ticket> {
    let key = raw.get("key").and_then(Value::as_str)?.to_string();
    let fields = raw.get("fields")?;

    let summary = fields
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("(no summary)")
        .to_string();

    let status_name = fields
        .get("status")
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();

    let assignee_account_id = fields
        .get("assignee")
        .and_then(|a| a.get("accountId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let assignee_display = fields
        .get("assignee")
        .and_then(|a| a.get("displayName"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let reporter_account_id = fields
        .get("reporter")
        .and_then(|r| r.get("accountId"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let project_key = fields
        .get("project")
        .and_then(|p| p.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            // Jira always returns project on issues, but if the
            // caller filtered `fields=` to exclude it, fall back to
            // the issue key prefix. Better than empty.
            key.split_once('-').map(|(p, _)| p).unwrap_or("")
        })
        .to_string();

    let updated_str = fields.get("updated").and_then(Value::as_str)?;
    let updated = parse_jira_timestamp(updated_str)?;

    let url = format!("{}/browse/{key}", base_url.trim_end_matches('/'));

    Some(Ticket {
        key,
        summary,
        status_name,
        assignee_account_id,
        assignee_display,
        reporter_account_id,
        project_key,
        url,
        updated,
        raw_json: raw.clone(),
    })
}

/// Parse a Jira timestamp string. Jira Cloud emits ISO 8601 with
/// the `+0000`/`+0900` offset format (no colon between hours and
/// minutes). `chrono::DateTime::parse_from_rfc3339` requires strict
/// RFC 3339 (`+00:00`), so we use the more permissive `%Y-%m-%dT%H:%M:%S%.f%z`
/// format which accepts both. Falls back to RFC 3339 in case Jira ever
/// changes its mind.
pub fn parse_jira_timestamp(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    None
}

/// Build the common envelope for every `jira.*` event (without
/// per-kind extras). Triggers reach fields via `{event.key}`,
/// `{event.summary}`, etc.
pub fn to_payload_json(t: &Ticket) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("key".to_string(), json!(t.key));
    m.insert("summary".to_string(), json!(t.summary));
    m.insert("status".to_string(), json!(t.status_name));
    m.insert(
        "assignee_account_id".to_string(),
        match &t.assignee_account_id {
            Some(s) => json!(s),
            None => Value::Null,
        },
    );
    m.insert(
        "assignee_display".to_string(),
        match &t.assignee_display {
            Some(s) => json!(s),
            None => Value::Null,
        },
    );
    m.insert(
        "reporter_account_id".to_string(),
        match &t.reporter_account_id {
            Some(s) => json!(s),
            None => Value::Null,
        },
    );
    m.insert("project_key".to_string(), json!(t.project_key));
    m.insert("url".to_string(), json!(t.url));
    m.insert("updated".to_string(), json!(t.updated.to_rfc3339()));
    // Verbatim Jira issue payload — matches the slack/discord
    // `event_json` convention so triggers reach uncommonly-needed
    // fields without us having to model every Jira field.
    m.insert("event_json".to_string(), t.raw_json.clone());
    m
}

// ============================================================
//                ADF (Atlassian Document Format)
// ============================================================

/// Walk an ADF document tree and concatenate all `text` node values
/// into a plain string. Paragraph boundaries become `\n\n`; line
/// breaks (hardBreak nodes) become `\n`. Mention nodes contribute
/// their `attrs.text` (which is `@DisplayName`). Other inline marks
/// (bold/italic/etc.) are stripped — the goal is interpolation-ready
/// text, not faithful rendering.
pub fn adf_to_plain_text(adf: &Value) -> String {
    let mut out = String::new();
    walk_adf(adf, &mut out);
    out.trim_end().to_string()
}

fn walk_adf(node: &Value, out: &mut String) {
    if !node.is_object() {
        return;
    }
    let node_type = node.get("type").and_then(Value::as_str).unwrap_or("");
    match node_type {
        "text" => {
            if let Some(t) = node.get("text").and_then(Value::as_str) {
                out.push_str(t);
            }
        }
        "hardBreak" => out.push('\n'),
        "mention" => {
            if let Some(t) = node
                .get("attrs")
                .and_then(|a| a.get("text"))
                .and_then(Value::as_str)
            {
                out.push_str(t);
            }
        }
        _ => {}
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for child in children {
            walk_adf(child, out);
        }
        if matches!(node_type, "paragraph" | "heading") {
            out.push_str("\n\n");
        } else if matches!(node_type, "listItem") {
            out.push('\n');
        }
    }
}

/// Walk an ADF tree looking for a `mention` node whose `attrs.id`
/// matches the given account_id. Used to detect `jira.mention` events
/// from `jira.comment_added` payloads.
pub fn adf_contains_mention_of(adf: &Value, account_id: &str) -> bool {
    if !adf.is_object() {
        return false;
    }
    if adf.get("type").and_then(Value::as_str) == Some("mention")
        && let Some(id) = adf
            .get("attrs")
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str)
        && id == account_id
    {
        return true;
    }
    if let Some(children) = adf.get("content").and_then(Value::as_array) {
        for child in children {
            if adf_contains_mention_of(child, account_id) {
                return true;
            }
        }
    }
    false
}

/// Parse a single comment from `/issue/<key>/comment`'s `comments[]`
/// array into a flat shape suitable for interpolation. Returns None
/// when essential fields are missing (id/body/created).
pub fn parse_comment(c: &Value) -> Option<Comment> {
    let id = c.get("id").and_then(Value::as_str)?.to_string();
    let body_adf = c.get("body").cloned().unwrap_or(Value::Null);
    let body_text = adf_to_plain_text(&body_adf);
    let created = c
        .get("created")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let author_account_id = c
        .get("author")
        .and_then(|a| a.get("accountId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let author_display = c
        .get("author")
        .and_then(|a| a.get("displayName"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(Comment {
        id,
        body_text,
        body_adf,
        created,
        author_account_id,
        author_display,
    })
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub id: String,
    pub body_text: String,
    pub body_adf: Value,
    pub created: String,
    pub author_account_id: Option<String>,
    pub author_display: Option<String>,
}

/// Serialize a `Comment` for the per-event `comment` sub-object.
pub fn comment_to_json(c: &Comment) -> Value {
    json!({
        "id": c.id,
        "body": c.body_text,
        "body_adf": c.body_adf,
        "created": c.created,
        "author_account_id": c.author_account_id,
        "author_display": c.author_display,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_issue() -> Value {
        json!({
            "key": "PROJ-42",
            "fields": {
                "summary": "Improve search UX",
                "status": { "name": "In Progress" },
                "assignee": {
                    "accountId": "5b-me",
                    "displayName": "Marshall Ku"
                },
                "reporter": {
                    "accountId": "5b-them",
                    "displayName": "Other Person"
                },
                "project": { "key": "PROJ" },
                "updated": "2026-05-05T10:00:00.000+0000",
                "comment": { "total": 3 }
            }
        })
    }

    #[test]
    fn from_jira_json_canonical() {
        let t = from_jira_json(&sample_issue(), "https://x.atlassian.net").unwrap();
        assert_eq!(t.key, "PROJ-42");
        assert_eq!(t.summary, "Improve search UX");
        assert_eq!(t.status_name, "In Progress");
        assert_eq!(t.assignee_account_id.as_deref(), Some("5b-me"));
        assert_eq!(t.assignee_display.as_deref(), Some("Marshall Ku"));
        assert_eq!(t.reporter_account_id.as_deref(), Some("5b-them"));
        assert_eq!(t.project_key, "PROJ");
        assert_eq!(t.url, "https://x.atlassian.net/browse/PROJ-42");
    }

    #[test]
    fn from_jira_json_handles_missing_assignee() {
        let mut raw = sample_issue();
        raw["fields"]["assignee"] = Value::Null;
        let t = from_jira_json(&raw, "https://x.atlassian.net").unwrap();
        assert!(t.assignee_account_id.is_none());
        assert!(t.assignee_display.is_none());
    }

    #[test]
    fn extract_inline_comments_walks_search_response() {
        let raw = json!({
            "key": "PROJ-1",
            "fields": {
                "comment": {
                    "comments": [
                        {
                            "id": "10001",
                            "body": { "type": "doc", "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "first" }] }
                            ]},
                            "created": "2026-05-05T10:00:00.000+0000",
                            "author": { "accountId": "5b-them", "displayName": "Other" }
                        },
                        {
                            "id": "10002",
                            "body": { "type": "doc", "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "second" }] }
                            ]},
                            "created": "2026-05-05T11:00:00.000+0000",
                            "author": { "accountId": "5b-them", "displayName": "Other" }
                        }
                    ],
                    "total": 2
                }
            }
        });
        let comments = extract_inline_comments(&raw);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].id, "10001");
        assert_eq!(comments[0].body_text, "first");
        assert_eq!(comments[1].id, "10002");
    }

    #[test]
    fn extract_inline_comments_empty_when_field_missing() {
        let raw = json!({ "key": "x", "fields": {} });
        assert!(extract_inline_comments(&raw).is_empty());
    }

    #[test]
    fn from_jira_json_returns_none_on_missing_essentials() {
        let raw = json!({ "key": "PROJ-1" }); // no fields
        assert!(from_jira_json(&raw, "https://x.atlassian.net").is_none());
        let raw = json!({ "fields": {} }); // no key
        assert!(from_jira_json(&raw, "https://x.atlassian.net").is_none());
    }

    #[test]
    fn to_payload_json_full_envelope() {
        let t = from_jira_json(&sample_issue(), "https://x.atlassian.net").unwrap();
        let m = to_payload_json(&t);
        assert_eq!(m["key"], "PROJ-42");
        assert_eq!(m["summary"], "Improve search UX");
        assert_eq!(m["status"], "In Progress");
        assert_eq!(m["assignee_account_id"], "5b-me");
        assert_eq!(m["project_key"], "PROJ");
        assert_eq!(m["url"], "https://x.atlassian.net/browse/PROJ-42");
        // event_json carries the verbatim Jira payload
        assert_eq!(m["event_json"]["key"], "PROJ-42");
    }

    #[test]
    fn snapshot_captures_diff_relevant_fields() {
        let t = from_jira_json(&sample_issue(), "https://x.atlassian.net").unwrap();
        let s = t.snapshot_without_comments();
        assert_eq!(s.status_name, "In Progress");
        assert_eq!(s.assignee_account_id.as_deref(), Some("5b-me"));
        // last_comment_created_iso starts None — poller fills it in
        // after walking inline comments.
        assert!(s.last_comment_created_iso.is_none());
        assert!(s.updated_iso.starts_with("2026-05-05T10:00:00"));
    }

    #[test]
    fn adf_to_plain_text_simple_paragraph() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "Hello world" }]
            }]
        });
        assert_eq!(adf_to_plain_text(&adf), "Hello world");
    }

    #[test]
    fn adf_to_plain_text_multi_paragraph_with_mention() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "Hey " },
                        { "type": "mention", "attrs": { "id": "5b-me", "text": "@Marshall" } },
                        { "type": "text", "text": ", look here" }
                    ]
                },
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "Second paragraph" }]
                }
            ]
        });
        let txt = adf_to_plain_text(&adf);
        assert!(txt.contains("Hey @Marshall, look here"), "got {txt:?}");
        assert!(txt.contains("Second paragraph"), "got {txt:?}");
    }

    #[test]
    fn adf_contains_mention_finds_target_account() {
        let adf = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Hi " },
                    { "type": "mention", "attrs": { "id": "5b-me", "text": "@Marshall" } }
                ]
            }]
        });
        assert!(adf_contains_mention_of(&adf, "5b-me"));
        assert!(!adf_contains_mention_of(&adf, "5b-other"));
    }

    #[test]
    fn adf_contains_mention_returns_false_on_no_mentions() {
        let adf = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "no mentions here" }]
            }]
        });
        assert!(!adf_contains_mention_of(&adf, "5b-me"));
    }

    #[test]
    fn adf_contains_mention_handles_nested_lists() {
        // Mentions can be nested inside list items.
        let adf = json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "mention", "attrs": { "id": "5b-me", "text": "@me" } }
                        ]
                    }]
                }]
            }]
        });
        assert!(adf_contains_mention_of(&adf, "5b-me"));
    }

    #[test]
    fn parse_comment_canonical() {
        let raw = json!({
            "id": "10042",
            "body": {
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "looks good" }]
                }]
            },
            "created": "2026-05-05T11:00:00.000+0000",
            "author": {
                "accountId": "5b-them",
                "displayName": "Other Person"
            }
        });
        let c = parse_comment(&raw).unwrap();
        assert_eq!(c.id, "10042");
        assert_eq!(c.body_text, "looks good");
        assert_eq!(c.author_account_id.as_deref(), Some("5b-them"));
        assert_eq!(c.author_display.as_deref(), Some("Other Person"));
    }

    #[test]
    fn parse_comment_returns_none_on_missing_id() {
        let raw = json!({ "body": "x" });
        assert!(parse_comment(&raw).is_none());
    }

    #[test]
    fn parse_jira_timestamp_handles_both_offset_forms() {
        // Jira's actual emission format (no colon in offset).
        let dt = parse_jira_timestamp("2026-05-05T10:00:00.000+0000").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-05-05T10:00:00+00:00");
        // RFC 3339 strict (with colon) — fallback path.
        let dt = parse_jira_timestamp("2026-05-05T10:00:00+09:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-05-05T01:00:00+00:00");
        // Invalid junk.
        assert!(parse_jira_timestamp("not-a-date").is_none());
    }

    #[test]
    fn comment_to_json_includes_both_text_and_adf() {
        let raw = json!({
            "id": "10042",
            "body": {
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "looks good" }]
                }]
            },
            "created": "2026-05-05T11:00:00.000+0000",
            "author": { "accountId": "5b-them", "displayName": "Other" }
        });
        let c = parse_comment(&raw).unwrap();
        let v = comment_to_json(&c);
        assert_eq!(v["id"], "10042");
        assert_eq!(v["body"], "looks good");
        assert_eq!(v["body_adf"]["type"], "doc");
        assert_eq!(v["author_account_id"], "5b-them");
    }
}
