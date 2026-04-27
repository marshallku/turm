//! Config-driven `event → action` automation.
//!
//! v1 design (see `docs/workflow-runtime.md`):
//! - Triggers are declared declaratively in TOML / JSON as `[[triggers]]`.
//! - `when` matches an event kind (glob) plus optional payload-field equality.
//! - `params` may contain `{event.X}` / `{context.X}` interpolation tokens.
//! - Action invocations go through an `Arc<dyn TriggerSink>`. Default impl
//!   on `ActionRegistry` gives synchronous error semantics for registered
//!   actions. Platforms can plug a wider sink (e.g. `turm-linux`'s
//!   `LiveTriggerSink` falls through to `socket::dispatch`, which gives
//!   ASYNCHRONOUS error visibility for legacy match-arm commands). Either
//!   way, errors are surfaced — one bad trigger cannot poison the dispatcher.
//!
//! This module is the pure primitive — no bus subscription, no config
//! loading, no thread management. The platform layer is responsible for
//! pumping events into `dispatch()`.

use crate::action_registry::{ActionRegistry, ActionResult};
use crate::condition;
use crate::context::Context;
use crate::event_bus::{Event, pattern_matches};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, RwLock};

/// Pluggable action invoker for the trigger engine. Default impl on
/// `ActionRegistry` covers registry-only reach. Platform integrations can
/// implement this to widen reach (e.g. routing unregistered actions through
/// `socket::dispatch` so legacy match-arm commands become trigger-reachable).
pub trait TriggerSink: Send + Sync {
    fn dispatch_action(&self, action: &str, params: Value) -> ActionResult;
}

impl TriggerSink for ActionRegistry {
    fn dispatch_action(&self, action: &str, params: Value) -> ActionResult {
        self.invoke(action, params)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Trigger {
    pub name: String,
    pub when: WhenSpec,
    pub action: String,
    #[serde(default)]
    pub params: Value,
    /// Optional boolean expression evaluated AFTER `when` matches.
    /// See `crate::condition` for grammar. Compiled once at
    /// `set_triggers` time; a parse error drops the offending trigger
    /// (the rest of the set still loads). At evaluation time, an
    /// `Err` from the evaluator (type mismatch on ordering, etc.)
    /// is logged and treated as "trigger does not match" — never
    /// fires the action on a misconfigured condition.
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhenSpec {
    /// Glob pattern matched against `event.kind`. Required.
    pub event_kind: String,
    /// Any other keys in the `when` table are treated as payload-field
    /// equality requirements. `{ event_kind = "slack.mention", channel = "alerts" }`
    /// matches `slack.mention` events whose payload has `channel == "alerts"`.
    #[serde(flatten)]
    pub payload_match: Map<String, Value>,
}

impl Trigger {
    /// Match `when` (event_kind glob + payload-equality) only. The
    /// `condition` clause is evaluated separately by `TriggerEngine`
    /// against the pre-compiled AST so we don't re-parse on every
    /// fired event. Useful as a primitive for tests + diagnostics.
    pub fn matches(&self, event: &Event) -> bool {
        if !pattern_matches(&self.when.event_kind, &event.kind) {
            return false;
        }
        for (key, expected) in &self.when.payload_match {
            match event.payload.get(key) {
                Some(actual) if actual == expected => continue,
                _ => return false,
            }
        }
        true
    }

    pub fn interpolate(&self, event: &Event, context: Option<&Context>) -> Value {
        interpolate_value(&self.params, event, context)
    }
}

/// Engine-internal compiled form. `set_triggers` parses each
/// `Trigger.condition` string into an AST once so per-event dispatch
/// stays cheap. Triggers whose condition fails to parse are dropped
/// at compile time with a `log::warn` — the rest of the set still
/// loads.
#[derive(Debug, Clone)]
struct CompiledTrigger {
    trigger: Trigger,
    condition: Option<condition::Expr>,
}

pub struct TriggerEngine {
    triggers: RwLock<Vec<CompiledTrigger>>,
    sink: Arc<dyn TriggerSink>,
}

impl TriggerEngine {
    pub fn new(sink: Arc<dyn TriggerSink>) -> Self {
        Self {
            triggers: RwLock::new(Vec::new()),
            sink,
        }
    }

    /// Replace the trigger list atomically. Used on startup and on config
    /// hot-reload. Concurrent dispatch sees either the old or the new full
    /// list, never a half-applied state.
    ///
    /// Each trigger's `condition` (if present) is compiled here.
    /// A parse failure drops THAT trigger and is logged; the rest of
    /// the set still loads so a single typo can't disable the entire
    /// trigger config.
    pub fn set_triggers(&self, triggers: Vec<Trigger>) {
        let compiled: Vec<CompiledTrigger> = triggers
            .into_iter()
            .filter_map(|t| {
                let parsed = match &t.condition {
                    None => None,
                    Some(src) => match condition::parse(src) {
                        Ok(e) => Some(e),
                        Err(err) => {
                            log::warn!(
                                "trigger {:?} condition parse error, dropping trigger: {err}",
                                t.name
                            );
                            return None;
                        }
                    },
                };
                Some(CompiledTrigger {
                    trigger: t,
                    condition: parsed,
                })
            })
            .collect();
        *self.triggers.write().unwrap() = compiled;
    }

    pub fn count(&self) -> usize {
        self.triggers.read().unwrap().len()
    }

    pub fn names(&self) -> Vec<String> {
        self.triggers
            .read()
            .unwrap()
            .iter()
            .map(|t| t.trigger.name.clone())
            .collect()
    }

    /// Match every trigger against `event`, interpolate params, invoke
    /// the corresponding action via the configured `TriggerSink`. Sink
    /// errors returned synchronously are logged here. Returns the number
    /// of triggers that fired (counts a synchronous `Ok` from the sink —
    /// note that some sinks, e.g. `LiveTriggerSink`'s legacy fallthrough,
    /// return `Ok` on queueing without waiting for the underlying action's
    /// outcome; those failures are surfaced asynchronously by the sink
    /// itself).
    pub fn dispatch(&self, event: &Event, context: Option<&Context>) -> usize {
        // Snapshot the trigger list under a short read lock so a concurrent
        // `set_triggers` can't observe partial iteration. Triggers are small
        // and infrequent, so cloning is cheap.
        let snapshot: Vec<CompiledTrigger> = self.triggers.read().unwrap().clone();
        let mut fired = 0;
        for ct in &snapshot {
            let trigger = &ct.trigger;
            if !trigger.matches(event) {
                continue;
            }
            // Condition check: bail before the action invocation if
            // the user-supplied predicate is false or evaluation
            // errored. An eval error is treated as "doesn't match"
            // rather than firing on a misconfigured condition — the
            // safer default.
            if let Some(expr) = &ct.condition {
                match condition::eval(expr, event, context) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(err) => {
                        log::warn!(
                            "trigger {:?} condition eval error for event {:?}, skipping: {err}",
                            trigger.name,
                            event.kind
                        );
                        continue;
                    }
                }
            }
            let params = trigger.interpolate(event, context);
            match self.sink.dispatch_action(&trigger.action, params) {
                Ok(_) => {
                    fired += 1;
                    log::debug!(
                        "trigger {:?} fired action {:?} for event {:?}",
                        trigger.name,
                        trigger.action,
                        event.kind
                    );
                }
                Err(err) => {
                    log::warn!(
                        "trigger {:?} action {:?} failed for event {:?}: code={} msg={}",
                        trigger.name,
                        trigger.action,
                        event.kind,
                        err.code,
                        err.message
                    );
                }
            }
        }
        fired
    }
}

/// Reduce a set of trigger `event_kind` patterns to the minimal set that
/// covers all of them — used by the platform layer to compute a deduplicated
/// list of bus subscriptions. Without this, declaring overlapping patterns
/// (e.g. `*` plus `panel.focused`) would cause the same bus event to be
/// delivered to multiple receivers and trigger every matching action once
/// per delivery instead of once per event.
///
/// Rules (matching `event_bus::pattern_matches`):
/// - `*` covers every pattern → if `*` is present, it's the only result.
/// - `foo.*` covers `foo.X`, `foo.X.Y`, and `foo.X.*` (any narrower pattern
///   under the same dotted prefix).
/// - Exact patterns cover only themselves; they survive only when no other
///   pattern in the set covers them.
pub fn covering_patterns<I, S>(patterns: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut set: Vec<String> = patterns.into_iter().map(Into::into).collect();
    set.sort();
    set.dedup();
    if set.iter().any(|p| p == "*") {
        return vec!["*".to_string()];
    }
    let mut result = Vec::new();
    for p in &set {
        let covered = set
            .iter()
            .any(|other| other != p && pattern_covers(other, p));
        if !covered {
            result.push(p.clone());
        }
    }
    result
}

fn pattern_covers(broader: &str, narrower: &str) -> bool {
    if broader == "*" {
        return true;
    }
    let Some(prefix) = broader.strip_suffix(".*") else {
        return false;
    };
    if let Some(narr_prefix) = narrower.strip_suffix(".*") {
        // `prefix.*` covers `prefix.X.*`, `prefix.X.Y.*`, etc.
        narr_prefix.len() > prefix.len()
            && narr_prefix.starts_with(prefix)
            && narr_prefix.as_bytes()[prefix.len()] == b'.'
    } else {
        // `prefix.*` covers `prefix.X`, `prefix.X.Y`, etc. (exact targets)
        narrower.len() > prefix.len()
            && narrower.starts_with(prefix)
            && narrower.as_bytes()[prefix.len()] == b'.'
    }
}

fn interpolate_value(template: &Value, event: &Event, context: Option<&Context>) -> Value {
    match template {
        Value::String(s) => Value::String(interpolate_string(s, event, context)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| interpolate_value(v, event, context))
                .collect(),
        ),
        Value::Object(obj) => {
            let mut out = Map::new();
            for (k, v) in obj {
                out.insert(k.clone(), interpolate_value(v, event, context));
            }
            Value::Object(out)
        }
        _ => template.clone(),
    }
}

fn interpolate_string(s: &str, event: &Event, context: Option<&Context>) -> String {
    let mut result = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close_rel) = after_open.find('}') {
            let token = &after_open[..close_rel];
            if let Some(val) = resolve_token(token, event, context) {
                result.push_str(&val);
            } else {
                // Unresolvable token: keep the literal `{token}` so misconfigured
                // triggers fail loudly in their target action rather than
                // silently substituting empty string.
                result.push('{');
                result.push_str(token);
                result.push('}');
            }
            rest = &after_open[close_rel + 1..];
        } else {
            // Unclosed `{` — append the remainder verbatim.
            result.push_str(&rest[open..]);
            return result;
        }
    }
    result.push_str(rest);
    result
}

fn resolve_token(token: &str, event: &Event, context: Option<&Context>) -> Option<String> {
    if let Some(field) = token.strip_prefix("event.") {
        return event.payload.get(field).map(json_scalar_to_string);
    }
    if let Some(field) = token.strip_prefix("context.") {
        let ctx = context?;
        return match field {
            "active_panel" => ctx.active_panel.clone(),
            "active_cwd" => ctx
                .active_cwd
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            _ => None,
        };
    }
    None
}

fn json_scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_registry::invalid_params;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn evt(kind: &str, payload: Value) -> Event {
        Event::new(kind, "test", payload)
    }

    fn mk_engine() -> (Arc<ActionRegistry>, TriggerEngine) {
        let reg = Arc::new(ActionRegistry::new());
        let engine = TriggerEngine::new(reg.clone());
        (reg, engine)
    }

    #[test]
    fn matches_exact_kind() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "calendar.event_imminent".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: Value::Null,
            condition: None,
        };
        assert!(t.matches(&evt("calendar.event_imminent", json!({}))));
        assert!(!t.matches(&evt("calendar.event_started", json!({}))));
    }

    #[test]
    fn matches_glob_kind() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "calendar.*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: Value::Null,
            condition: None,
        };
        assert!(t.matches(&evt("calendar.event_imminent", json!({}))));
        assert!(t.matches(&evt("calendar.event_created", json!({}))));
        assert!(!t.matches(&evt("slack.mention", json!({}))));
    }

    #[test]
    fn payload_match_required() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "slack.mention".into(),
                payload_match: {
                    let mut m = Map::new();
                    m.insert("channel".into(), json!("alerts"));
                    m
                },
            },
            action: "noop".into(),
            params: Value::Null,
            condition: None,
        };
        assert!(t.matches(&evt(
            "slack.mention",
            json!({"channel": "alerts", "text": "hi"})
        )));
        assert!(!t.matches(&evt("slack.mention", json!({"channel": "general"}))));
        assert!(!t.matches(&evt("slack.mention", json!({})))); // missing field
    }

    #[test]
    fn interpolates_event_payload_fields() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "id": "{event.id}",
                "msg": "got {event.id} from {event.source}",
            }),
            condition: None,
        };
        let result = t.interpolate(
            &evt(
                "calendar.event_imminent",
                json!({"id": "abc", "source": "x"}),
            ),
            None,
        );
        // event.source resolves from payload (we publish "test" as source but
        // tokens look up payload, not the top-level Event::source field).
        assert_eq!(result["id"], json!("abc"));
        assert_eq!(result["msg"], json!("got abc from x"));
    }

    #[test]
    fn interpolates_context_fields() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({"cmd": "echo {context.active_cwd} :: {context.active_panel}"}),
            condition: None,
        };
        let ctx = Context {
            active_panel: Some("panel-1".into()),
            active_cwd: Some(PathBuf::from("/tmp/work")),
        };
        let result = t.interpolate(&evt("any", json!({})), Some(&ctx));
        assert_eq!(result["cmd"], json!("echo /tmp/work :: panel-1"));
    }

    #[test]
    fn unresolved_tokens_kept_as_literals() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "a": "{event.missing}",
                "b": "{unknown}",
                "c": "no braces",
                "d": "unclosed {brace",
            }),
            condition: None,
        };
        let result = t.interpolate(&evt("any", json!({})), None);
        assert_eq!(result["a"], json!("{event.missing}"));
        assert_eq!(result["b"], json!("{unknown}"));
        assert_eq!(result["c"], json!("no braces"));
        assert_eq!(result["d"], json!("unclosed {brace"));
    }

    #[test]
    fn interpolation_walks_nested_arrays_and_objects() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "list": ["{event.a}", "x", {"deep": "{event.b}"}],
                "n": 42,
                "b": true,
            }),
            condition: None,
        };
        let result = t.interpolate(&evt("any", json!({"a": "A", "b": "B"})), None);
        assert_eq!(result["list"][0], json!("A"));
        assert_eq!(result["list"][1], json!("x"));
        assert_eq!(result["list"][2]["deep"], json!("B"));
        assert_eq!(result["n"], json!(42));
        assert_eq!(result["b"], json!(true));
    }

    #[test]
    fn dispatch_invokes_matching_action_with_interpolated_params() {
        let (reg, engine) = mk_engine();
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        {
            let c = captured.clone();
            reg.register("record", move |params| {
                c.lock().unwrap().push(params);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "calendar.event_imminent".into(),
                payload_match: Map::new(),
            },
            action: "record".into(),
            params: json!({"id": "{event.id}"}),
            condition: None,
        }]);
        let fired = engine.dispatch(
            &evt("calendar.event_imminent", json!({"id": "evt-9"})),
            None,
        );
        assert_eq!(fired, 1);
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], json!({"id": "evt-9"}));
    }

    #[test]
    fn dispatch_skips_non_matching_triggers() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("bump", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![Trigger {
            name: "only_slack".into(),
            when: WhenSpec {
                event_kind: "slack.*".into(),
                payload_match: Map::new(),
            },
            action: "bump".into(),
            params: Value::Null,
            condition: None,
        }]);
        engine.dispatch(&evt("calendar.event_imminent", json!({})), None);
        engine.dispatch(&evt("terminal.cwd_changed", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        engine.dispatch(&evt("slack.mention", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn action_error_is_logged_not_propagated() {
        let (reg, engine) = mk_engine();
        reg.register("fail", |_| Err(invalid_params("nope")));
        engine.set_triggers(vec![Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "any".into(),
                payload_match: Map::new(),
            },
            action: "fail".into(),
            params: Value::Null,
            condition: None,
        }]);
        // Should not panic. fired count is 0 because the action returned Err.
        let fired = engine.dispatch(&evt("any", json!({})), None);
        assert_eq!(fired, 0);
    }

    #[test]
    fn unknown_action_is_logged_not_propagated() {
        let (_reg, engine) = mk_engine();
        engine.set_triggers(vec![Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "any".into(),
                payload_match: Map::new(),
            },
            action: "no_such_action".into(),
            params: Value::Null,
            condition: None,
        }]);
        let fired = engine.dispatch(&evt("any", json!({})), None);
        assert_eq!(fired, 0);
    }

    // -- Condition integration --

    fn trig_with_condition(name: &str, condition: Option<&str>) -> Trigger {
        Trigger {
            name: name.into(),
            when: WhenSpec {
                event_kind: "calendar.event_imminent".into(),
                payload_match: Map::new(),
            },
            action: "fire".into(),
            params: Value::Null,
            condition: condition.map(str::to_string),
        }
    }

    #[test]
    fn condition_skips_trigger_when_false() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("fire", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![trig_with_condition(
            "skip-declined",
            Some(r#"event.my_response_status != "declined""#),
        )]);
        // Declined event: trigger should NOT fire.
        let fired = engine.dispatch(
            &evt(
                "calendar.event_imminent",
                json!({"my_response_status": "declined"}),
            ),
            None,
        );
        assert_eq!(fired, 0);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        // Accepted event: trigger SHOULD fire.
        let fired = engine.dispatch(
            &evt(
                "calendar.event_imminent",
                json!({"my_response_status": "accepted"}),
            ),
            None,
        );
        assert_eq!(fired, 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn condition_eval_error_skips_trigger_safely() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("fire", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        // `>` on a non-numeric payload field — eval errors at runtime.
        engine.set_triggers(vec![trig_with_condition(
            "bad-cond",
            Some(r#"event.title > "5""#),
        )]);
        let fired = engine.dispatch(
            &evt(
                "calendar.event_imminent",
                json!({"title": "weekly meeting"}),
            ),
            None,
        );
        // Eval error → safe default is "doesn't match" — fire count
        // stays zero rather than firing on a misconfigured condition.
        assert_eq!(fired, 0);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn condition_parse_error_drops_only_bad_trigger() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("fire", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![
            trig_with_condition("good", None),
            trig_with_condition("broken", Some("foo == bar baz")), // garbage
        ]);
        // Only the good trigger should be live.
        assert_eq!(engine.count(), 1);
        assert_eq!(engine.names(), vec!["good".to_string()]);
        let fired = engine.dispatch(&evt("calendar.event_imminent", json!({})), None);
        assert_eq!(fired, 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn condition_with_context_ref() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("fire", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![trig_with_condition(
            "only-when-panel-1",
            Some(r#"context.active_panel == "panel-1""#),
        )]);
        // Wrong panel → skip
        let ctx_other = Context {
            active_panel: Some("panel-9".into()),
            active_cwd: None,
        };
        engine.dispatch(&evt("calendar.event_imminent", json!({})), Some(&ctx_other));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        // Right panel → fire
        let ctx = Context {
            active_panel: Some("panel-1".into()),
            active_cwd: None,
        };
        engine.dispatch(&evt("calendar.event_imminent", json!({})), Some(&ctx));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn condition_round_trips_through_toml() {
        let toml_src = r#"
            name = "test"
            action = "fire"
            condition = "event.x != \"y\""

            [when]
            event_kind = "k"
        "#;
        let t: Trigger = toml::from_str(toml_src).unwrap();
        assert_eq!(t.condition.as_deref(), Some(r#"event.x != "y""#));
    }

    #[test]
    fn set_triggers_replaces_existing_atomically() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("bump", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        let make = |kind: &str| Trigger {
            name: kind.into(),
            when: WhenSpec {
                event_kind: kind.into(),
                payload_match: Map::new(),
            },
            action: "bump".into(),
            params: Value::Null,
            condition: None,
        };
        engine.set_triggers(vec![make("a"), make("b")]);
        assert_eq!(engine.count(), 2);
        engine.dispatch(&evt("a", json!({})), None);
        engine.dispatch(&evt("b", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 2);

        engine.set_triggers(vec![make("c")]);
        assert_eq!(engine.count(), 1);
        engine.dispatch(&evt("a", json!({})), None);
        engine.dispatch(&evt("b", json!({})), None);
        // No further bumps: a/b triggers are gone.
        assert_eq!(count.load(Ordering::SeqCst), 2);
        engine.dispatch(&evt("c", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn covering_dedupes_exact_duplicates() {
        let out = covering_patterns(vec!["foo.bar", "foo.bar"]);
        assert_eq!(out, vec!["foo.bar"]);
    }

    #[test]
    fn covering_star_subsumes_everything() {
        let out = covering_patterns(vec!["*", "panel.focused", "calendar.*"]);
        assert_eq!(out, vec!["*"]);
    }

    #[test]
    fn covering_glob_subsumes_exact_under_same_prefix() {
        let mut out = covering_patterns(vec!["panel.*", "panel.focused", "panel.exited"]);
        out.sort();
        assert_eq!(out, vec!["panel.*"]);
    }

    #[test]
    fn covering_glob_subsumes_deeper_glob() {
        // `foo.*` covers `foo.bar.*` (both globs, latter is narrower).
        let out = covering_patterns(vec!["foo.*", "foo.bar.*"]);
        assert_eq!(out, vec!["foo.*"]);
    }

    #[test]
    fn covering_keeps_disjoint_patterns() {
        let mut out = covering_patterns(vec!["panel.*", "calendar.*", "terminal.cwd_changed"]);
        out.sort();
        assert_eq!(
            out,
            vec![
                "calendar.*".to_string(),
                "panel.*".to_string(),
                "terminal.cwd_changed".to_string(),
            ]
        );
    }

    #[test]
    fn covering_does_not_match_substring_namespaces() {
        // `panel.*` must NOT cover `panelfoo` or `panelfoo.bar` — the dot
        // separator is significant.
        let mut out = covering_patterns(vec!["panel.*", "panelfoo.bar"]);
        out.sort();
        assert_eq!(out, vec!["panel.*".to_string(), "panelfoo.bar".to_string()]);
    }

    #[test]
    fn deserializes_from_toml_round_trip() {
        let toml_src = r#"
            name = "meeting-prep"
            action = "plugin.notion.open_event_doc"
            params = { event_id = "{event.id}", lead_minutes = 10 }

            [when]
            event_kind = "calendar.event_imminent"
            minutes = 10
        "#;
        let t: Trigger = toml::from_str(toml_src).unwrap();
        assert_eq!(t.name, "meeting-prep");
        assert_eq!(t.action, "plugin.notion.open_event_doc");
        assert_eq!(t.when.event_kind, "calendar.event_imminent");
        // The non-`event_kind` field under `[when]` becomes a payload match.
        assert_eq!(t.when.payload_match["minutes"], json!(10));
        // `params` interpolates as a normal Value tree.
        assert_eq!(t.params["event_id"], json!("{event.id}"));
        assert_eq!(t.params["lead_minutes"], json!(10));
    }

    // -- Phase 14.1: end-to-end chained trigger via completion fan-out --

    /// E2E for the killer-demo shape: an originating event fires
    /// trigger A which invokes action `step1`. The registry's
    /// completion fan-out publishes `step1.completed` onto the
    /// bus. Trigger B on `step1.completed` invokes action `step2`,
    /// which is what we assert ran. Without Phase 14.1 the second
    /// step would never have anything to listen to.
    #[test]
    fn phase_14_1_chained_triggers_compose_via_completion_event() {
        use crate::event_bus::{EventBus, RecvOutcome};
        use std::time::Duration;

        let bus = Arc::new(EventBus::new());
        let registry = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));

        // step1 returns a payload that step2 will interpolate from.
        registry.register("step1", |params| {
            Ok(json!({
                "echoed": params,
                "marker": "from-step1",
            }))
        });

        // step2 records the params it was invoked with so we can
        // assert the chain wired the data through correctly.
        let step2_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let recorder = step2_calls.clone();
            registry.register("step2", move |params| {
                recorder.lock().unwrap().push(params);
                Ok(json!(null))
            });
        }

        let engine = TriggerEngine::new(registry as Arc<dyn TriggerSink>);
        engine.set_triggers(vec![
            Trigger {
                name: "trigger-a".into(),
                when: WhenSpec {
                    event_kind: "user.kicked_off".into(),
                    payload_match: Map::new(),
                },
                action: "step1".into(),
                params: json!({ "id": "{event.id}" }),
                condition: None,
            },
            Trigger {
                name: "trigger-b".into(),
                when: WhenSpec {
                    event_kind: "step1.completed".into(),
                    payload_match: Map::new(),
                },
                action: "step2".into(),
                params: json!({ "marker": "{event.marker}" }),
                condition: None,
            },
        ]);

        // Subscribe to the bus before dispatching so we can drive
        // trigger-b ourselves on whatever the registry publishes.
        // Pattern matches the platform layer's pump loop.
        let rx = bus.subscribe_unbounded("step1.completed");

        // Fire the originating event manually. trigger-a fires
        // step1; the registry then auto-publishes
        // step1.completed; we read it from the bus and re-dispatch
        // through engine.dispatch(), which fires trigger-b.
        let originating = Event::new("user.kicked_off", "test", json!({"id": "abc"}));
        engine.dispatch(&originating, None);

        // Pull the completion event the registry published.
        let completion = match rx.recv_timeout(Duration::from_millis(200)) {
            RecvOutcome::Event(e) => e,
            other => panic!("expected step1.completed, got {other:?}"),
        };
        assert_eq!(completion.kind, "step1.completed");
        assert_eq!(completion.payload["marker"], "from-step1");

        // Re-pump: feed the completion event through the engine.
        // Trigger-b matches and runs step2.
        engine.dispatch(&completion, None);

        let step2_invocations = step2_calls.lock().unwrap();
        assert_eq!(step2_invocations.len(), 1);
        assert_eq!(step2_invocations[0]["marker"], json!("from-step1"));
    }

    /// Same shape as above but the first step FAILS — verify
    /// `step1.failed` lights up a recovery trigger.
    #[test]
    fn phase_14_1_failed_event_drives_recovery_trigger() {
        use crate::action_registry::invalid_params;
        use crate::event_bus::{EventBus, RecvOutcome};
        use std::time::Duration;

        let bus = Arc::new(EventBus::new());
        let registry = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        registry.register("flaky", |_| Err(invalid_params("nope")));
        let recovery_calls = Arc::new(AtomicUsize::new(0));
        {
            let c = recovery_calls.clone();
            registry.register("recovery", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }

        let engine = TriggerEngine::new(registry as Arc<dyn TriggerSink>);
        engine.set_triggers(vec![
            Trigger {
                name: "kick".into(),
                when: WhenSpec {
                    event_kind: "go".into(),
                    payload_match: Map::new(),
                },
                action: "flaky".into(),
                params: Value::Null,
                condition: None,
            },
            Trigger {
                name: "on-fail".into(),
                when: WhenSpec {
                    event_kind: "flaky.failed".into(),
                    payload_match: Map::new(),
                },
                action: "recovery".into(),
                params: Value::Null,
                condition: None,
            },
        ]);

        let rx = bus.subscribe_unbounded("flaky.failed");
        engine.dispatch(&Event::new("go", "test", json!({})), None);

        let failed = match rx.recv_timeout(Duration::from_millis(200)) {
            RecvOutcome::Event(e) => e,
            other => panic!("expected flaky.failed, got {other:?}"),
        };
        assert_eq!(failed.kind, "flaky.failed");
        assert_eq!(failed.payload["code"], "invalid_params");

        engine.dispatch(&failed, None);
        assert_eq!(recovery_calls.load(Ordering::SeqCst), 1);
    }
}
