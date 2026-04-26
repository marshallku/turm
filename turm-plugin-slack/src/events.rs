//! Map raw Slack `events_api` payloads to `slack.mention` /
//! `slack.dm` turm events.
//!
//! Slack delivers a wide variety of message types over Socket Mode:
//! channel messages, DMs, edits, deletions, joins, bot messages,
//! thread replies, channel-renames, app_home, etc. The plugin filters
//! aggressively so triggers only fire on signal — actual human
//! mentions and direct messages — without each user having to
//! handle the full diversity in their `[[triggers]]` config.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackEvent {
    Mention(MessageFields),
    Dm(MessageFields),
    /// Full-fidelity firehose: emitted for EVERY events_api frame
    /// regardless of filtering. Carries the raw inner `event` object
    /// (blocks, files, attachments, edits, joins — everything Slack
    /// sends) so a `kb.append` trigger can archive the firehose
    /// without further plugin work. Users who only want
    /// mention/DM triggers ignore this kind. Wire shape includes
    /// the outer envelope's `team_id` / `event_id` for routing
    /// the archive into per-workspace folders.
    Raw(RawEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFields {
    pub user: String,
    pub channel: String,
    pub text: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub team_id: Option<String>,
    pub event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    /// Slack event type (`message`, `app_mention`, `channel_rename`,
    /// etc.) — surfaced separately so triggers can match on it
    /// without parsing `event_json`.
    pub event_type: String,
    /// Channel id IF the underlying event has one (most do).
    /// `null` for events like `team_join` that aren't channel-scoped.
    pub channel: Option<String>,
    /// Message timestamp / event ID for archival deduping. Slack
    /// guarantees the (channel, ts) pair is unique per message.
    pub ts: Option<String>,
    pub team_id: Option<String>,
    pub event_id: Option<String>,
    /// The raw nested `event` object verbatim — no field is
    /// stripped so the archive captures full Slack fidelity.
    /// Trigger access notes:
    /// - `[triggers.condition]` supports nested ref paths
    ///   (`event.event_json.subtype`, `event.event_json.bot_id`)
    ///   so users can filter on inner fields.
    /// - `params` interpolation (`{event.X}`) only resolves
    ///   top-level keys; `{event.event_json}` interpolates the
    ///   whole inner object as JSON, which is exactly what the
    ///   archive trigger wants. Nested-key interpolation in
    ///   params is a future trigger-DSL extension.
    pub event_json: Value,
}

impl SlackEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            SlackEvent::Mention(_) => "slack.mention",
            SlackEvent::Dm(_) => "slack.dm",
            SlackEvent::Raw(_) => "slack.raw",
        }
    }

    pub fn payload_json(&self) -> Value {
        match self {
            SlackEvent::Mention(f) | SlackEvent::Dm(f) => json!({
                "user": f.user,
                "channel": f.channel,
                "text": f.text,
                "ts": f.ts,
                "thread_ts": f.thread_ts,
                "team_id": f.team_id,
                "event_id": f.event_id,
            }),
            SlackEvent::Raw(r) => json!({
                "event_type": r.event_type,
                "channel": r.channel,
                "ts": r.ts,
                "team_id": r.team_id,
                "event_id": r.event_id,
                "event_json": r.event_json,
            }),
        }
    }
}

/// Top-level entrypoint: examine an `events_api` envelope payload
/// and produce zero or more turm-shaped events.
///
/// Returns BOTH:
/// - The `slack.raw` event (always emitted — full firehose for
///   archive triggers).
/// - Optionally one of `slack.mention` / `slack.dm` if the payload
///   passes the aggressive filter (real human mention or DM).
///
/// `payload` is the value of the outer frame's `payload` key, which
/// itself contains `event_id`, `team_id`, `event`, etc. (Slack's
/// "Events API outer wrapper.")
pub fn from_events_api_payload(payload: &Value) -> Vec<SlackEvent> {
    let Some(event) = payload.get("event") else {
        return Vec::new();
    };
    let event_id = payload
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let team_id = payload
        .get("team_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut out = Vec::with_capacity(2);
    // Raw firehose first so the archive trigger sees an event even
    // when the filter would have dropped it (channel renames,
    // joins, edits, etc. are valuable historical context).
    out.push(SlackEvent::Raw(build_raw(
        event,
        event_id.clone(),
        team_id.clone(),
    )));
    if let Some(filtered) = classify_event(event, event_id, team_id) {
        out.push(filtered);
    }
    out
}

fn build_raw(event: &Value, event_id: Option<String>, team_id: Option<String>) -> RawEvent {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let channel = event
        .get("channel")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ts = event
        .get("ts")
        .and_then(Value::as_str)
        .map(str::to_string);
    RawEvent {
        event_type,
        channel,
        ts,
        team_id,
        event_id,
        event_json: event.clone(),
    }
}

fn classify_event(
    event: &Value,
    event_id: Option<String>,
    team_id: Option<String>,
) -> Option<SlackEvent> {
    let event_type = event.get("type")?.as_str()?;
    match event_type {
        "app_mention" => {
            // Defensive: skip bot-originated mentions and edits.
            // Slack normally won't send these for app_mention but
            // keeping the filter symmetric with the DM path means a
            // future Slack delivery rule change can't accidentally
            // turn the plugin into a self-loop generator.
            if event.get("subtype").is_some() {
                return None;
            }
            if event.get("bot_id").is_some() {
                return None;
            }
            let f = parse_message_fields(event, event_id, team_id)?;
            Some(SlackEvent::Mention(f))
        }
        "message" => {
            // Filter aggressively. Slack sends edits, deletions, joins,
            // pinned-messages, and bot-broadcasts all under
            // `type=message` — only DMs from a real user without a
            // subtype should reach turm as `slack.dm`.
            //
            // - `subtype` present → edit / delete / join / file_share
            //   etc. Skip; users can layer in handling later via the
            //   raw archive (Phase 11.2) if they want.
            // - `bot_id` present → message was sent by a bot, including
            //   our own self-loops if the bot user happens to chat in
            //   the channel. Skip.
            // - `channel_type != "im"` → not a direct message. Skip.
            if event.get("subtype").is_some() {
                return None;
            }
            if event.get("bot_id").is_some() {
                return None;
            }
            let channel_type = event
                .get("channel_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            if channel_type != "im" {
                return None;
            }
            let f = parse_message_fields(event, event_id, team_id)?;
            Some(SlackEvent::Dm(f))
        }
        _ => None,
    }
}

fn parse_message_fields(
    event: &Value,
    event_id: Option<String>,
    team_id: Option<String>,
) -> Option<MessageFields> {
    Some(MessageFields {
        user: event.get("user").and_then(Value::as_str)?.to_string(),
        channel: event.get("channel").and_then(Value::as_str)?.to_string(),
        text: event
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        ts: event.get("ts").and_then(Value::as_str)?.to_string(),
        thread_ts: event
            .get("thread_ts")
            .and_then(Value::as_str)
            .map(str::to_string),
        team_id,
        event_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with(event: Value) -> Value {
        json!({
            "event_id": "Ev0PV52K21",
            "team_id": "T0123",
            "event": event,
        })
    }

    /// Test helper: pick the filtered event (mention/dm) out of the
    /// returned vec, asserting that exactly one filtered + one raw
    /// were emitted. Returns the filtered event.
    fn expect_filtered(out: Vec<SlackEvent>) -> SlackEvent {
        assert_eq!(out.len(), 2, "expected raw + filtered, got {out:?}");
        assert!(matches!(out[0], SlackEvent::Raw(_)), "first event must be Raw");
        out.into_iter().nth(1).unwrap()
    }

    /// Test helper: assert only `slack.raw` was emitted (filter
    /// rejected). Returns the raw event.
    fn expect_raw_only(out: Vec<SlackEvent>) -> RawEvent {
        assert_eq!(out.len(), 1, "expected raw only, got {out:?}");
        match out.into_iter().next().unwrap() {
            SlackEvent::Raw(r) => r,
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn parses_app_mention() {
        let p = payload_with(json!({
            "type": "app_mention",
            "user": "U999",
            "channel": "C123",
            "text": "<@U800> ping?",
            "ts": "1700000000.000100",
        }));
        match expect_filtered(from_events_api_payload(&p)) {
            SlackEvent::Mention(f) => {
                assert_eq!(f.user, "U999");
                assert_eq!(f.channel, "C123");
                assert_eq!(f.text, "<@U800> ping?");
                assert_eq!(f.ts, "1700000000.000100");
                assert_eq!(f.team_id.as_deref(), Some("T0123"));
            }
            other => panic!("expected Mention, got {other:?}"),
        }
    }

    #[test]
    fn parses_dm() {
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "user": "U999",
            "channel": "D123",
            "text": "hi there",
            "ts": "1700000000.000200",
        }));
        match expect_filtered(from_events_api_payload(&p)) {
            SlackEvent::Dm(f) => {
                assert_eq!(f.user, "U999");
                assert_eq!(f.channel, "D123");
                assert_eq!(f.text, "hi there");
            }
            other => panic!("expected Dm, got {other:?}"),
        }
    }

    #[test]
    fn skips_channel_message_filter_but_emits_raw() {
        // type=message + channel_type=channel → ordinary channel
        // chatter, not a DM. Must NOT emit slack.dm — but still
        // emit slack.raw so archive triggers can capture it.
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "channel",
            "user": "U999",
            "channel": "C123",
            "text": "team standup",
            "ts": "1700000000.000300",
        }));
        let raw = expect_raw_only(from_events_api_payload(&p));
        assert_eq!(raw.event_type, "message");
        assert_eq!(raw.channel.as_deref(), Some("C123"));
    }

    #[test]
    fn skips_message_with_subtype() {
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "subtype": "message_changed",
            "user": "U999",
            "channel": "D123",
            "text": "edited",
            "ts": "1700000000.000400",
        }));
        let raw = expect_raw_only(from_events_api_payload(&p));
        assert_eq!(raw.event_json["subtype"], "message_changed");
    }

    #[test]
    fn skips_bot_message() {
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "bot_id": "B000",
            "channel": "D123",
            "text": "automated",
            "ts": "1700000000.000500",
            "user": "U999",
        }));
        expect_raw_only(from_events_api_payload(&p));
    }

    #[test]
    fn skips_bot_mention() {
        let p = payload_with(json!({
            "type": "app_mention",
            "bot_id": "B000",
            "user": "U999",
            "channel": "C123",
            "text": "<@U800>",
            "ts": "1700000000.000700",
        }));
        expect_raw_only(from_events_api_payload(&p));
    }

    #[test]
    fn skips_mention_with_subtype() {
        let p = payload_with(json!({
            "type": "app_mention",
            "subtype": "message_changed",
            "user": "U999",
            "channel": "C123",
            "text": "<@U800>",
            "ts": "1700000000.000800",
        }));
        expect_raw_only(from_events_api_payload(&p));
    }

    #[test]
    fn unknown_event_type_emits_raw_only() {
        // channel_rename (and other non-message events) emits raw
        // but not a filtered event — the archive can capture it.
        let p = payload_with(json!({
            "type": "channel_rename",
            "channel": "C123",
        }));
        let raw = expect_raw_only(from_events_api_payload(&p));
        assert_eq!(raw.event_type, "channel_rename");
        assert_eq!(raw.channel.as_deref(), Some("C123"));
        assert!(raw.ts.is_none());
    }

    #[test]
    fn raw_preserves_full_event_payload() {
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "channel": "D123",
            "user": "U999",
            "text": "complex",
            "ts": "1700000000.000900",
            "blocks": [{"type": "rich_text", "elements": []}],
            "files": [{"id": "F123", "name": "diagram.png"}],
        }));
        let out = from_events_api_payload(&p);
        let raw = match &out[0] {
            SlackEvent::Raw(r) => r.clone(),
            other => panic!("expected Raw first, got {other:?}"),
        };
        // Raw event MUST carry the unmodified blocks/files arrays so
        // archive triggers see full Slack fidelity.
        assert_eq!(raw.event_json["blocks"][0]["type"], "rich_text");
        assert_eq!(raw.event_json["files"][0]["id"], "F123");
        // Event id and team_id come from the OUTER envelope, not
        // the inner event.
        assert_eq!(raw.event_id.as_deref(), Some("Ev0PV52K21"));
        assert_eq!(raw.team_id.as_deref(), Some("T0123"));
    }

    #[test]
    fn missing_event_field_returns_empty() {
        // Truly malformed payload (no `event` key) → no events at all.
        let p = json!({"event_id": "x", "team_id": "T0"});
        assert!(from_events_api_payload(&p).is_empty());
    }

    #[test]
    fn captures_thread_ts() {
        let p = payload_with(json!({
            "type": "app_mention",
            "user": "U999",
            "channel": "C123",
            "text": "in thread",
            "ts": "1700000000.000600",
            "thread_ts": "1700000000.000500",
        }));
        match expect_filtered(from_events_api_payload(&p)) {
            SlackEvent::Mention(f) => {
                assert_eq!(f.thread_ts.as_deref(), Some("1700000000.000500"));
            }
            other => panic!("expected Mention, got {other:?}"),
        }
    }

    #[test]
    fn payload_json_includes_all_fields() {
        let f = MessageFields {
            user: "U999".into(),
            channel: "C123".into(),
            text: "hi".into(),
            ts: "1700.000".into(),
            thread_ts: Some("1700.000".into()),
            team_id: Some("T0".into()),
            event_id: Some("Ev0".into()),
        };
        let v = SlackEvent::Mention(f).payload_json();
        assert_eq!(v["user"], "U999");
        assert_eq!(v["channel"], "C123");
        assert_eq!(v["text"], "hi");
        assert_eq!(v["thread_ts"], "1700.000");
        assert_eq!(v["team_id"], "T0");
        assert_eq!(v["event_id"], "Ev0");
    }

    #[test]
    fn missing_required_fields_for_filter_still_emits_raw() {
        // No `user` field → filter rejects (can't build
        // MessageFields). But raw is still useful for archive.
        let p = payload_with(json!({
            "type": "app_mention",
            "channel": "C123",
            "text": "hi",
            "ts": "1700.000",
        }));
        expect_raw_only(from_events_api_payload(&p));
    }
}
