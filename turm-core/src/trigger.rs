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
use crate::event_bus::{Event, EventBus, pattern_matches};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

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
    /// Phase 14.2 async-correlation primitive. When set, after the
    /// action fires the engine registers a "pending await" entry and
    /// holds the chain until a follow-up event matching
    /// `await.event_kind` + interpolated `await.payload_match`
    /// arrives within `await.timeout_seconds`. On match, a
    /// synthesized `<trigger_name>.awaited` event is published with
    /// the matched event's payload nested under `await:`. Downstream
    /// triggers reference that synthesized event_kind to continue
    /// the chain.
    #[serde(default)]
    pub r#await: Option<AwaitClause>,
}

/// Configuration for the await-primitive on a trigger. Lives next to
/// `Trigger` rather than as a separate top-level concept because each
/// await is intrinsically scoped to one trigger row — there's no
/// shared registry of "wait specs" that triggers can reference.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AwaitClause {
    /// Glob-pattern for the event kind we're waiting for. Same matcher
    /// as `WhenSpec.event_kind`, supports `*`.
    pub event_kind: String,
    /// Field equality requirements on the awaited event's payload.
    /// Each value is interpolated against the ORIGINAL event (the
    /// one that fired this trigger) before matching, so a clause
    /// like `payload_match = { user = "{event.user}" }` filters
    /// awaited events to those that match the originating user.
    /// `action_result` is intentionally NOT exposed here in v1 —
    /// the sink's return value is unreliable for blocking actions
    /// (LiveTriggerSink returns `{queued: true}` and the real result
    /// arrives asynchronously). v2 may grow that via
    /// `<action>.completed` correlation.
    #[serde(default)]
    pub payload_match: Map<String, Value>,
    /// Maximum seconds to wait. After expiry, `on_timeout` decides.
    pub timeout_seconds: u64,
    /// `abort` (default): drop the pending entry, no further events
    /// fire. `fire_with_default`: synthesize the awaited event with
    /// nulls in the `await` slot so downstream triggers run with
    /// degraded data — caller's responsibility to handle null.
    #[serde(default)]
    pub on_timeout: TimeoutPolicy,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutPolicy {
    #[default]
    Abort,
    FireWithDefault,
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

/// In-flight async-correlation entry tracked by the engine after a
/// trigger with an `await` clause fires its action.
///
/// **Two-phase state machine**: a trigger's action dispatch lands in
/// `preflight_awaits` first; promotion to `pending_awaits` waits for
/// the `<action>.completed` event (Phase 14.1 fan-out). Why: blocking
/// and plugin actions return `Ok({queued: true})` synchronously from
/// `LiveTriggerSink` BEFORE the action actually runs, so arming a
/// pending entry on the sink's `Ok` would queue an await even when
/// the action later fails async. `<action>.completed` is the only
/// signal that's true for both sync registry calls AND async-blocking
/// calls — `publish_completion` fires it in both paths. Legacy
/// match-arm actions that don't go through the registry don't fire
/// `.completed`, so awaits on them never arm — documented limitation,
/// same audience as Phase 14.1.
struct PreflightAwait {
    trigger_name: String,
    /// Action name we're waiting for completion of. The completion
    /// event kind is `format!("{action}.completed")`; failure is
    /// `format!("{action}.failed")`.
    action: String,
    await_event_kind: String,
    payload_match: Map<String, Value>,
    original_payload: Value,
    deadline: Instant,
    on_timeout: TimeoutPolicy,
}

/// Holds the data needed to (1) match incoming events against the
/// await's payload_match, (2) build the synthesized
/// `<trigger_name>.awaited` payload when a match arrives or timeout
/// fires. Promoted from `PreflightAwait` once `<action>.completed`
/// confirms the dispatched action actually succeeded.
struct PendingAwait {
    /// Used both to namespace the synthesized event kind
    /// (`<trigger_name>.awaited`) and as a debugging label.
    trigger_name: String,
    await_event_kind: String,
    /// Payload-equality requirements with literal `Value`s — already
    /// interpolated against the original event at registration time.
    /// Comparing JSON `Value`s lets us match scalars exactly (string
    /// vs number vs bool) without re-interpolating per incoming event.
    payload_match: Map<String, Value>,
    /// Original event payload, carried forward into the synthesized
    /// event so downstream interpolation still sees `{event.<orig>}`.
    original_payload: Value,
    deadline: Instant,
    on_timeout: TimeoutPolicy,
}

pub struct TriggerEngine {
    triggers: RwLock<Vec<CompiledTrigger>>,
    sink: Arc<dyn TriggerSink>,
    /// Phase 14.2 async-correlation state — preflight half. Entry
    /// added when a trigger with `await` fires; promoted to
    /// `pending_awaits` when `<action>.completed` arrives, dropped
    /// when `<action>.failed` arrives or the sweep timer expires.
    preflight_awaits: RwLock<Vec<PreflightAwait>>,
    /// Phase 14.2 async-correlation state — armed half. Pending
    /// entries accumulate after promotion from preflight and drain
    /// when matching events arrive or `sweep_pending_awaits` removes
    /// expired ones. Bounded in practice by the number of in-flight
    /// workflows; we don't cap length here, so a runaway producer of
    /// await-bearing triggers would grow unboundedly — acceptable
    /// for now since the only producer is user-authored config.
    pending_awaits: RwLock<Vec<PendingAwait>>,
    /// Optional bus for publishing synthesized `<trigger_name>.awaited`
    /// events. None makes the engine unable to emit awaited events —
    /// triggers with `await` clauses still register pending entries
    /// but no downstream chains can fire. Opt-in (matches
    /// `ActionRegistry::with_completion_bus` shape) so test harnesses
    /// don't need to construct a bus.
    publish_bus: Option<Arc<EventBus>>,
}

impl TriggerEngine {
    pub fn new(sink: Arc<dyn TriggerSink>) -> Self {
        Self {
            triggers: RwLock::new(Vec::new()),
            sink,
            preflight_awaits: RwLock::new(Vec::new()),
            pending_awaits: RwLock::new(Vec::new()),
            publish_bus: None,
        }
    }

    /// Same as `new` but threads in an `EventBus` so the engine can
    /// publish synthesized `<trigger_name>.awaited` events for the
    /// async-correlation primitive (Phase 14.2). Production code on
    /// turm-linux uses this; pure unit tests use `new`.
    pub fn with_publish_bus(sink: Arc<dyn TriggerSink>, bus: Arc<EventBus>) -> Self {
        Self {
            triggers: RwLock::new(Vec::new()),
            sink,
            preflight_awaits: RwLock::new(Vec::new()),
            pending_awaits: RwLock::new(Vec::new()),
            publish_bus: Some(bus),
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
        // Phase 14.2: hot-reload is documented as all-or-nothing
        // (docs/core-lib.md), so any await state from the OLD trigger
        // set must die with the swap — otherwise a removed-or-renamed
        // trigger could still emit `<old_name>.awaited` after the
        // new config takes effect. Volatile state was already
        // documented as restart-loses; reload-loses is the same
        // contract. Pre-Phase-14.2 callers (no `await` clauses) are
        // unaffected because both vectors are always empty.
        self.preflight_awaits.write().unwrap().clear();
        self.pending_awaits.write().unwrap().clear();
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
        // Phase 14.2: do the await-state passes BEFORE iterating
        // triggers. Order matters because a trigger that fires its
        // own action X on the same event we just used to promote
        // could otherwise have its freshly-registered preflight
        // immediately consumed by the same `X.completed` event,
        // arming a downstream await against the WRONG action call.
        // Pass order:
        //   1. promote/drop pre-existing preflights against this
        //      event (handles `<X>.completed` / `<X>.failed` events).
        //   2. match this event against pre-existing pending awaits
        //      and emit `<trigger_name>.awaited` for any hit.
        //   3. iterate triggers, register fresh preflights for
        //      await-bearing matches — these will only see FUTURE
        //      completion events, never the one we just processed.
        self.try_promote_or_drop_preflight(event);
        self.try_match_pending_awaits(event);

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
                    // Phase 14.2: stage a PREFLIGHT entry. Don't arm
                    // pending_awaits directly off the sink's `Ok` —
                    // blocking/plugin actions return
                    // `Ok({queued: true})` synchronously BEFORE the
                    // action actually succeeds. The preflight is
                    // promoted to pending_awaits when
                    // `<action>.completed` arrives (or dropped on
                    // `<action>.failed`).
                    if let Some(aw) = &trigger.r#await {
                        self.register_preflight_await(trigger, aw, event, context);
                    }
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

    /// Stage a preflight-await entry. Interpolates the `payload_match`
    /// against the originating event NOW, so once the action's
    /// `.completed` event promotes this entry, the per-incoming-event
    /// comparison is a pure JSON value check (no per-event
    /// interpolation overhead).
    fn register_preflight_await(
        &self,
        trigger: &Trigger,
        aw: &AwaitClause,
        event: &Event,
        context: Option<&Context>,
    ) {
        let mut interpolated_match = Map::new();
        for (k, v) in &aw.payload_match {
            // Use `interpolate_value_typed` so a single-token string like
            // `"{event.count}"` resolves to the raw JSON value (preserves
            // numbers, booleans, nulls instead of coercing to string).
            // Awaited events compare via JSON value equality; without
            // this, `payload_match = { count = "{event.count}" }` would
            // compare `Value::String("42")` to `Value::Number(42)` and
            // never match.
            interpolated_match.insert(k.clone(), interpolate_value_typed(v, event, context));
        }
        let preflight = PreflightAwait {
            trigger_name: trigger.name.clone(),
            action: trigger.action.clone(),
            await_event_kind: aw.event_kind.clone(),
            payload_match: interpolated_match,
            original_payload: event.payload.clone(),
            deadline: Instant::now() + Duration::from_secs(aw.timeout_seconds),
            on_timeout: aw.on_timeout,
        };
        self.preflight_awaits.write().unwrap().push(preflight);
    }

    /// Promote first matching preflight to pending on `<X>.completed`,
    /// drop first matching preflight on `<X>.failed`. **Trust check**:
    /// only events sourced from `turm.action` (the action_registry's
    /// completion fan-out — see `action_registry::COMPLETION_EVENT_SOURCE`)
    /// can advance the state machine. An unrelated bus producer that
    /// happens to publish `<something>.completed` with the same suffix
    /// is ignored, so an event from outside the registry can't mutate
    /// our await state.
    ///
    /// Match key is
    /// the action name `X` ONLY — neither `<X>.completed` nor
    /// `<X>.failed` carries the originating event/trigger id, so we
    /// can't per-invocation correlate. **FIFO scope is "across ALL
    /// preflights for action X — regardless of trigger AND
    /// regardless of invocation"**:
    ///
    /// - Two different await-bearing triggers that share an action
    ///   share a single FIFO queue at the action level. A completion
    ///   from trigger B's invocation can promote trigger A's
    ///   preflight if A queued first.
    /// - Even a single trigger fired multiple times concurrently
    ///   can mis-correlate. If trigger T fires three times in quick
    ///   succession (preflights p1, p2, p3) and completions arrive
    ///   in order c1, c2, c3, FIFO matches them correctly. If
    ///   completions arrive c2, c1, c3, FIFO promotes p1 with c2's
    ///   payload, p2 with c1's, p3 with c3's — wrong invocation
    ///   correlation, but at least each preflight does end up
    ///   promoted exactly once.
    ///
    /// In practice most use cases have at most one in-flight
    /// invocation per action at a time (Slack-ask-and-wait, for
    /// example, isn't repeatedly fired). The mis-correlation only
    /// hurts when the SAME action is dispatched multiple times in
    /// fast succession AND the awaited follow-up payloads need to
    /// match the specific invocation.
    ///
    /// Closing this fully needs per-invocation correlation tokens in
    /// `<X>.completed`/`.failed` payloads (action_registry change).
    /// Tracked as a slice-2 follow-up.
    fn try_promote_or_drop_preflight(&self, event: &Event) {
        // Trust check: only the action_registry's synthesized
        // completion fan-out can advance the state machine. An
        // unrelated producer publishing a `.completed`-suffixed
        // event would otherwise silently mutate await state.
        if event.source != crate::action_registry::COMPLETION_EVENT_SOURCE {
            return;
        }
        // Strip `.completed` / `.failed` suffix to recover the action
        // name. Anything else is irrelevant to this stage.
        let (action, success) = if let Some(action) = event.kind.strip_suffix(".completed") {
            (action, true)
        } else if let Some(action) = event.kind.strip_suffix(".failed") {
            (action, false)
        } else {
            return;
        };
        let mut pre = self.preflight_awaits.write().unwrap();
        let Some(idx) = pre.iter().position(|p| p.action == action) else {
            return;
        };
        let removed = pre.remove(idx);
        if !success {
            // .failed: chain is broken at the action; don't promote.
            log::debug!(
                "trigger {:?} preflight dropped on {action}.failed",
                removed.trigger_name
            );
            return;
        }
        // .completed: promote to pending. We DON'T re-set the deadline
        // here — the original timeout window covers preflight + pending
        // combined. If `.completed` arrived just before the deadline,
        // the pending gets less time to match the awaited event; that's
        // the user's contract for total wait.
        self.pending_awaits.write().unwrap().push(PendingAwait {
            trigger_name: removed.trigger_name,
            await_event_kind: removed.await_event_kind,
            payload_match: removed.payload_match,
            original_payload: removed.original_payload,
            deadline: removed.deadline,
            on_timeout: removed.on_timeout,
        });
    }

    /// Walk pending awaits and fire `<trigger_name>.awaited` for the
    /// FIRST one whose `event_kind` + `payload_match` accept this
    /// event. Single-consumption: one incoming event resolves at most
    /// one pending entry. When two pendings share identical match
    /// criteria (same Slack user filter, etc.), only the oldest fires
    /// — the runner-up keeps waiting for its own follow-up. This is
    /// the conservative choice; broadcasting one reply to multiple
    /// concurrent prompts would silently double-fire downstream
    /// chains, which is much harder to debug.
    fn try_match_pending_awaits(&self, event: &Event) {
        let mut to_emit: Option<(String, Value)> = None;
        {
            let mut pending = self.pending_awaits.write().unwrap();
            let mut matched_idx: Option<usize> = None;
            for (idx, p) in pending.iter().enumerate() {
                if !pattern_matches(&p.await_event_kind, &event.kind) {
                    continue;
                }
                let mut all_match = true;
                for (k, expected) in &p.payload_match {
                    match event.payload.get(k) {
                        Some(actual) if actual == expected => continue,
                        _ => {
                            all_match = false;
                            break;
                        }
                    }
                }
                if all_match {
                    matched_idx = Some(idx);
                    break;
                }
            }
            if let Some(idx) = matched_idx {
                let p = pending.remove(idx);
                let synthesized = build_awaited_payload(&p.original_payload, &event.payload);
                // Don't publish under the lock — bus.publish may
                // re-enter dispatch (subscribers receive synchronously
                // today, but a subscriber calling back into the engine
                // would deadlock against this write lock).
                to_emit = Some((awaited_kind_for(&p.trigger_name), synthesized));
            }
        }
        if let Some((kind, payload)) = to_emit
            && let Some(bus) = &self.publish_bus
        {
            bus.publish(Event::new(kind, AWAITED_EVENT_SOURCE, payload));
        }
    }

    /// Drop expired entries from BOTH preflight and pending await
    /// pools. The deadline is set at preflight registration time and
    /// carries unchanged through promotion to pending — total wait
    /// (preflight + pending) shares one timeout window. For entries
    /// with `on_timeout = FireWithDefault`, publish a synthesized
    /// `<trigger_name>.awaited` event with `null` in the await slot
    /// so downstream chains can run with a missing-data fallback.
    /// A preflight that expires before `<action>.completed` arrives
    /// (legacy match-arm action that doesn't fire `.completed`,
    /// stalled action) is treated the same way as a timed-out
    /// pending — Abort drops silently, FireWithDefault still emits
    /// the synthesized event with `await: null`.
    ///
    /// Caller invokes this on a timer (e.g. turm-linux's GTK pump
    /// every 50ms) — the engine has no thread of its own.
    pub fn sweep_pending_awaits(&self) {
        let now = Instant::now();
        let mut to_emit: Vec<(String, Value)> = Vec::new();
        {
            let mut preflight = self.preflight_awaits.write().unwrap();
            preflight.retain(|p| {
                if p.deadline > now {
                    return true;
                }
                if matches!(p.on_timeout, TimeoutPolicy::FireWithDefault) {
                    let synthesized = build_awaited_payload(&p.original_payload, &Value::Null);
                    to_emit.push((awaited_kind_for(&p.trigger_name), synthesized));
                }
                false
            });
        }
        {
            let mut pending = self.pending_awaits.write().unwrap();
            pending.retain(|p| {
                if p.deadline > now {
                    return true;
                }
                if matches!(p.on_timeout, TimeoutPolicy::FireWithDefault) {
                    let synthesized = build_awaited_payload(&p.original_payload, &Value::Null);
                    to_emit.push((awaited_kind_for(&p.trigger_name), synthesized));
                }
                false
            });
        }
        if let Some(bus) = &self.publish_bus {
            for (kind, payload) in to_emit {
                bus.publish(Event::new(kind, AWAITED_EVENT_SOURCE, payload));
            }
        }
    }

    /// Diagnostic accessor — number of pending await entries currently
    /// armed (post-promotion). Used by tests; not load-bearing in
    /// production.
    pub fn pending_await_count(&self) -> usize {
        self.pending_awaits.read().unwrap().len()
    }

    /// Diagnostic accessor — number of preflight entries currently
    /// waiting for `<action>.completed`. Used by tests.
    pub fn preflight_await_count(&self) -> usize {
        self.preflight_awaits.read().unwrap().len()
    }
}

/// Source label for synthesized `<trigger_name>.awaited` events.
/// Distinct from action_registry's `turm.action` and bus producers'
/// own kinds so consumers can identify origin if needed.
const AWAITED_EVENT_SOURCE: &str = "turm.trigger.await";

fn awaited_kind_for(trigger_name: &str) -> String {
    format!("{trigger_name}.awaited")
}

/// Build the payload of a synthesized `<trigger_name>.awaited` event.
/// Carries the originating trigger's event payload at top level (so
/// `{event.<orig>}` interpolation keeps working in downstream
/// triggers) and nests the matched event's payload under `await:`
/// (so `{event.await.<field>}` reads the reply data via the dot-path
/// interpolator).
fn build_awaited_payload(original: &Value, awaited: &Value) -> Value {
    let mut obj = match original {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    obj.insert("await".to_string(), awaited.clone());
    Value::Object(obj)
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

/// Type-preserving interpolation for `await.payload_match` values.
/// Differs from `interpolate_value` (which is used for action params)
/// in one place: when a string is exactly `{<token>}` with nothing
/// else, the raw resolved JSON `Value` is returned instead of the
/// string-coerced form. That's necessary because await match runs
/// pure JSON-value equality against the awaited event's payload —
/// `payload_match = { count = "{event.count}" }` against `event.count =
/// 42` (a number) would otherwise compare `Value::String("42")` to
/// `Value::Number(42)` and never match. Mixed templates like
/// `{ "Hello {event.name}" }` still string-coerce because there's no
/// single source value to preserve. Unresolved single-token cases
/// fall back to the literal `{token}` string (same fail-loud posture
/// as flat-token resolution).
fn interpolate_value_typed(template: &Value, event: &Event, context: Option<&Context>) -> Value {
    match template {
        Value::String(s) => {
            // Single-token-only path: extract `{...}` if the string is
            // exactly one token wrapper and nothing else.
            if let Some(token) = single_token(s)
                && let Some(value) = resolve_token_value(token, event, context)
            {
                return value.clone();
            }
            Value::String(interpolate_string(s, event, context))
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| interpolate_value_typed(v, event, context))
                .collect(),
        ),
        Value::Object(obj) => {
            let mut out = Map::new();
            for (k, v) in obj {
                out.insert(k.clone(), interpolate_value_typed(v, event, context));
            }
            Value::Object(out)
        }
        _ => template.clone(),
    }
}

/// If `s` is exactly `{<token>}` with no surrounding text and no
/// embedded close-brace, return the inner token. Used by the typed
/// payload_match interpolator to know when raw-value substitution is
/// safe vs when string-coercion is the only viable path.
fn single_token(s: &str) -> Option<&str> {
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    if inner.contains('{') || inner.contains('}') {
        return None;
    }
    Some(inner)
}

/// Like `resolve_token` but returns the raw JSON `Value` reference
/// instead of stringifying. Only used by the typed payload_match
/// interpolator.
fn resolve_token_value<'a>(
    token: &str,
    event: &'a Event,
    context: Option<&'a Context>,
) -> Option<&'a Value> {
    if let Some(field) = token.strip_prefix("event.") {
        return resolve_dot_path(&event.payload, field);
    }
    // `context.X` resolution returns the raw payload field if we
    // wrap it as a Value. ContextService doesn't expose a typed
    // surface (it's just two Option<String>s today), so for context
    // tokens we fall back to the string interpolator's view by
    // returning None — caller will then string-coerce, which matches
    // the existing context surface. Adding typed context resolution
    // is a future extension when richer context data lands.
    let _ = context;
    None
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
        return resolve_dot_path(&event.payload, field).map(json_scalar_to_string);
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

/// Walk a dot-separated path (`foo.bar.baz`) into a JSON `Value`.
/// Top-level field is a single segment (legacy shape: `event.id`);
/// nested objects use additional dots (`event.payload.text`,
/// `event.await.thread_ts`). Stops at the first non-object hop —
/// e.g. `event.tags.0` won't index into an array, returning `None`
/// instead. That keeps the surface intentionally narrow until a
/// real call site asks for array/index access.
fn resolve_dot_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.') {
        match cur {
            Value::Object(map) => {
                cur = map.get(seg)?;
            }
            _ => return None,
        }
    }
    Some(cur)
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
    fn interpolates_nested_event_paths() {
        // Phase 14.2 prep: dot-path access through nested objects so
        // `{event.await.text}` and `{event.action_result.thread_ts}`
        // can resolve. Pre-14.2 only top-level keys worked.
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "answer": "{event.await.text}",
                "deep": "{event.a.b.c}",
            }),
            condition: None,
            r#await: None,
        };
        let result = t.interpolate(
            &evt(
                "todo-ask.awaited",
                json!({
                    "await": { "text": "PROJ-42" },
                    "a": { "b": { "c": "ok" } },
                }),
            ),
            None,
        );
        assert_eq!(result["answer"], json!("PROJ-42"));
        assert_eq!(result["deep"], json!("ok"));
    }

    #[test]
    fn dot_path_unresolved_keeps_literal_for_safety() {
        // When a dot-path bottoms out (missing intermediate object,
        // or non-object hop), the resolver returns None. The
        // interpolator preserves the literal `{token}` so a
        // misconfigured trigger surfaces as a visible action error
        // rather than silently substituting empty.
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({"v": "{event.missing.path}"}),
            condition: None,
            r#await: None,
        };
        let result = t.interpolate(&evt("any", json!({"present": 1})), None);
        assert_eq!(result["v"], json!("{event.missing.path}"));
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
            r#await: None,
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
                r#await: None,
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
                r#await: None,
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
                r#await: None,
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
                r#await: None,
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

    // -- Phase 15.2: end-to-end Vision Flow 3 chain --

    /// Drives a 3-trigger chain modeling the killer demo:
    ///   `todo.start_requested` (with linked_jira)
    ///     → `git.worktree_add` (sanitize_jira branch)
    ///     → `git.worktree_add.completed` (auto-emitted)
    ///     → `claude.start` (with workspace_path interpolated)
    ///
    /// Every step is a real `[[triggers]]` row; the actions are
    /// mocks that record their interpolated params so we can
    /// assert end-to-end data flow without spawning real
    /// subprocesses (claude.start needs GTK; git.worktree_add
    /// needs a real repo). The relevant integration surface
    /// here IS the engine + bus + registry plumbing — that's
    /// what Phase 15.2 wires together.
    #[test]
    fn phase_15_2_killer_demo_chain_with_jira() {
        use crate::event_bus::EventBus;
        use std::time::Duration;

        let bus = Arc::new(EventBus::new());
        let registry = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));

        // Captures so we can assert what each action received.
        let worktree_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let claude_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

        // Mock git.worktree_add — returns the canonical
        // `{workspace, path, branch, base}` shape so the
        // registry's auto-published `git.worktree_add.completed`
        // carries the same payload trigger-3 will interpolate.
        // Mirrors the real plugin's sanitize_jira semantics so
        // the test asserts the lowercased branch flows through
        // to claude.start (NOT just that the flag was set on
        // the params).
        {
            let recorder = worktree_calls.clone();
            registry.register("git.worktree_add", move |params| {
                recorder.lock().unwrap().push(params.clone());
                let workspace = params
                    .get("workspace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let raw_branch = params
                    .get("branch")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let sanitize = params
                    .get("sanitize_jira")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let branch = if sanitize {
                    raw_branch.to_ascii_lowercase()
                } else {
                    raw_branch
                };
                Ok(json!({
                    "workspace": workspace,
                    "path": format!("/tmp/wt/{branch}"),
                    "branch": branch,
                    "base": "main",
                }))
            });
        }

        // Mock claude.start — records params; returns a stub
        // shape matching Phase 18.1's response so any further
        // chained trigger has data to interpolate from.
        {
            let recorder = claude_calls.clone();
            registry.register("claude.start", move |params| {
                recorder.lock().unwrap().push(params.clone());
                Ok(json!({
                    "panel_id": "panel-test",
                    "tab": 1,
                    "tmux_session": "wt-feature",
                    "workspace_path": params
                        .get("workspace_path")
                        .cloned()
                        .unwrap_or(Value::Null),
                }))
            });
        }

        let engine = TriggerEngine::new(registry as Arc<dyn TriggerSink>);
        engine.set_triggers(vec![
            // Trigger 1: with-jira branch
            Trigger {
                name: "with-jira".into(),
                when: WhenSpec {
                    event_kind: "todo.start_requested".into(),
                    payload_match: Map::new(),
                },
                action: "git.worktree_add".into(),
                params: json!({
                    "workspace": "{event.workspace}",
                    "branch": "{event.linked_jira}",
                    // NOTE: the engine doesn't yet have a
                    // sanitize_jira flag because the production
                    // chain delegates that to the git plugin.
                    // We still pass `sanitize_jira = true`
                    // through interpolation so the captured
                    // params reflect the real-world TOML.
                    "sanitize_jira": true,
                }),
                condition: Some("event.linked_jira != null".to_string()),
                r#await: None,
            },
            Trigger {
                name: "without-jira".into(),
                when: WhenSpec {
                    event_kind: "todo.start_requested".into(),
                    payload_match: Map::new(),
                },
                action: "git.worktree_add".into(),
                params: json!({
                    "workspace": "{event.workspace}",
                    "branch": "todo-{event.id}",
                }),
                condition: Some("event.linked_jira == null".to_string()),
                r#await: None,
            },
            Trigger {
                name: "claude-after-worktree".into(),
                when: WhenSpec {
                    event_kind: "git.worktree_add.completed".into(),
                    payload_match: Map::new(),
                },
                action: "claude.start".into(),
                params: json!({"workspace_path": "{event.path}"}),
                condition: None,
                r#await: None,
            },
        ]);

        // Subscribe to the chained event before dispatching so
        // we can manually re-pump it through the engine. In the
        // live system, turm-linux's pump_state drains
        // `git.worktree_add.completed` once per GTK tick.
        let rx = bus.subscribe_unbounded("git.worktree_add.completed");

        // Fire originating event with linked_jira set.
        let originating = Event::new(
            "todo.start_requested",
            "test",
            json!({
                "id": "T-20260427",
                "workspace": "myrepo",
                "linked_jira": "PROJ-456",
                "title": "feature work",
            }),
        );
        engine.dispatch(&originating, None);

        // Trigger 1 fired (with-jira), trigger 2 skipped (cond false).
        let calls = worktree_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "only the with-jira trigger should run");
        assert_eq!(calls[0]["workspace"], "myrepo");
        assert_eq!(calls[0]["branch"], "PROJ-456");
        assert_eq!(calls[0]["sanitize_jira"], true);

        // Re-pump the auto-emitted git.worktree_add.completed
        // through the engine so trigger-3 fires on it.
        let completion = match rx.recv_timeout(Duration::from_millis(200)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected git.worktree_add.completed, got {other:?}"),
        };
        engine.dispatch(&completion, None);

        let claude_seen = claude_calls.lock().unwrap().clone();
        assert_eq!(
            claude_seen.len(),
            1,
            "claude.start should have been invoked once"
        );
        // workspace_path comes from the worktree result's path,
        // and the path was computed AFTER the mock applied
        // sanitize_jira's lowercasing — same shape as the real
        // plugin.
        assert_eq!(
            claude_seen[0]["workspace_path"], "/tmp/wt/proj-456",
            "claude.start should receive the LOWERCASED worktree path"
        );
    }

    /// Same chain, no `linked_jira` → trigger 2 (without-jira)
    /// fires instead, branch is `todo-<id>`.
    #[test]
    fn phase_15_2_killer_demo_chain_without_jira() {
        use crate::event_bus::EventBus;
        use std::time::Duration;

        let bus = Arc::new(EventBus::new());
        let registry = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));

        let worktree_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let claude_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

        {
            let recorder = worktree_calls.clone();
            registry.register("git.worktree_add", move |params| {
                recorder.lock().unwrap().push(params.clone());
                let branch = params
                    .get("branch")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                Ok(json!({
                    "workspace": "myrepo",
                    "path": format!("/tmp/wt/{branch}"),
                    "branch": branch,
                    "base": "main",
                }))
            });
        }
        {
            let recorder = claude_calls.clone();
            registry.register("claude.start", move |params| {
                recorder.lock().unwrap().push(params.clone());
                Ok(json!({"workspace_path": params.get("workspace_path").cloned()}))
            });
        }

        let engine = TriggerEngine::new(registry as Arc<dyn TriggerSink>);
        engine.set_triggers(vec![
            Trigger {
                name: "with-jira".into(),
                when: WhenSpec {
                    event_kind: "todo.start_requested".into(),
                    payload_match: Map::new(),
                },
                action: "git.worktree_add".into(),
                params: json!({
                    "workspace": "{event.workspace}",
                    "branch": "{event.linked_jira}",
                }),
                condition: Some("event.linked_jira != null".to_string()),
                r#await: None,
            },
            Trigger {
                name: "without-jira".into(),
                when: WhenSpec {
                    event_kind: "todo.start_requested".into(),
                    payload_match: Map::new(),
                },
                action: "git.worktree_add".into(),
                params: json!({
                    "workspace": "{event.workspace}",
                    "branch": "todo-{event.id}",
                }),
                condition: Some("event.linked_jira == null".to_string()),
                r#await: None,
            },
            Trigger {
                name: "claude-after-worktree".into(),
                when: WhenSpec {
                    event_kind: "git.worktree_add.completed".into(),
                    payload_match: Map::new(),
                },
                action: "claude.start".into(),
                params: json!({"workspace_path": "{event.path}"}),
                condition: None,
                r#await: None,
            },
        ]);

        let rx = bus.subscribe_unbounded("git.worktree_add.completed");

        // No linked_jira in payload (omitted entirely; the
        // todo plugin emits null in this case).
        let originating = Event::new(
            "todo.start_requested",
            "test",
            json!({
                "id": "T-20260427",
                "workspace": "myrepo",
                "linked_jira": Value::Null,
                "title": "personal",
            }),
        );
        engine.dispatch(&originating, None);

        let calls = worktree_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "only the without-jira trigger should run");
        assert_eq!(calls[0]["branch"], "todo-T-20260427");

        let completion = match rx.recv_timeout(Duration::from_millis(200)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected git.worktree_add.completed, got {other:?}"),
        };
        engine.dispatch(&completion, None);

        let claude_seen = claude_calls.lock().unwrap().clone();
        assert_eq!(claude_seen.len(), 1);
        assert_eq!(claude_seen[0]["workspace_path"], "/tmp/wt/todo-T-20260427");
    }

    /// The `examples/triggers/vision-flow-3.toml` file ships as
    /// the documented Phase 15.2 reference config. If it stops
    /// parsing — e.g. because someone renamed a field or
    /// changed the condition DSL — users copy-pasting it would
    /// hit a config-load error at turm startup. Pin it.
    #[test]
    fn phase_15_2_example_toml_parses_cleanly() {
        // Path is relative to the workspace root; cargo runs
        // tests with the per-crate dir as CWD, so step out.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples/triggers/vision-flow-3.toml");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        #[derive(serde::Deserialize)]
        struct File {
            #[serde(default)]
            triggers: Vec<Trigger>,
        }
        let parsed: File =
            toml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

        // Sanity: 3 active triggers (the optional log row is commented out).
        assert_eq!(parsed.triggers.len(), 3);
        let names: Vec<&str> = parsed.triggers.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"vision3-start-with-jira"));
        assert!(names.contains(&"vision3-start-without-jira"));
        assert!(names.contains(&"vision3-claude-after-worktree"));

        // The condition strings must compile under the same
        // condition DSL the engine uses — set_triggers calls
        // condition::parse() and silently drops triggers whose
        // condition fails to parse. Catch that here so the
        // example doesn't silently fail in the field.
        let bus_sink = ActionRegistry::new();
        let engine = TriggerEngine::new(Arc::new(bus_sink));
        engine.set_triggers(parsed.triggers);
        assert_eq!(
            engine.count(),
            3,
            "all three triggers should compile (no condition::parse drops)"
        );
    }

    /// Ensures `git.worktree_add.failed` does NOT fire
    /// claude.start. The chain only progresses on success.
    #[test]
    fn phase_15_2_chain_halts_on_worktree_failure() {
        use crate::event_bus::EventBus;
        use std::time::Duration;

        let bus = Arc::new(EventBus::new());
        let registry = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));

        registry.register("git.worktree_add", |_| {
            Err(crate::action_registry::invalid_params("branch_exists"))
        });
        let claude_calls = Arc::new(AtomicUsize::new(0));
        {
            let c = claude_calls.clone();
            registry.register("claude.start", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }

        let engine = TriggerEngine::new(registry as Arc<dyn TriggerSink>);
        engine.set_triggers(vec![
            Trigger {
                name: "kick".into(),
                when: WhenSpec {
                    event_kind: "todo.start_requested".into(),
                    payload_match: Map::new(),
                },
                action: "git.worktree_add".into(),
                params: json!({"workspace": "x", "branch": "y"}),
                condition: None,
                r#await: None,
            },
            Trigger {
                name: "claude-after-worktree".into(),
                when: WhenSpec {
                    event_kind: "git.worktree_add.completed".into(),
                    payload_match: Map::new(),
                },
                action: "claude.start".into(),
                params: Value::Null,
                condition: None,
                r#await: None,
            },
        ]);

        // Subscribe to BOTH possible chained events to confirm
        // only `failed` was emitted, not `completed`.
        let completed_rx = bus.subscribe_unbounded("git.worktree_add.completed");
        let failed_rx = bus.subscribe_unbounded("git.worktree_add.failed");

        engine.dispatch(&Event::new("todo.start_requested", "test", json!({})), None);

        // failed event lands.
        let failed = match failed_rx.recv_timeout(Duration::from_millis(200)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected failed event, got {other:?}"),
        };
        assert_eq!(failed.kind, "git.worktree_add.failed");

        // completed event does NOT land.
        match completed_rx.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Timeout => {}
            other => panic!("completed event should NOT fire on Err: {other:?}"),
        }

        // claude.start never invoked.
        engine.dispatch(&failed, None);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 0);
    }

    // -- Phase 14.2: await primitive --
    //
    // The shape under test: a trigger with `await` registers a pending
    // entry on action success, then drains when an event matching the
    // await's `event_kind` + interpolated `payload_match` arrives.
    // The synthesized `<trigger_name>.awaited` payload exposes the
    // matched event's payload under `await` for downstream interpolation.

    fn mk_engine_with_bus() -> (
        Arc<ActionRegistry>,
        TriggerEngine,
        Arc<crate::event_bus::EventBus>,
    ) {
        let reg = Arc::new(ActionRegistry::new());
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let engine = TriggerEngine::with_publish_bus(reg.clone(), bus.clone());
        (reg, engine, bus)
    }

    fn trig_with_await(name: &str, action: &str, when_kind: &str, aw: AwaitClause) -> Trigger {
        Trigger {
            name: name.into(),
            when: WhenSpec {
                event_kind: when_kind.into(),
                payload_match: Map::new(),
            },
            action: action.into(),
            params: Value::Null,
            condition: None,
            r#await: Some(aw),
        }
    }

    #[test]
    fn await_registers_preflight_on_dispatch_then_promotes_on_completed() {
        // Two-phase: dispatch lands in preflight; `<action>.completed`
        // promotes to pending. Without the `.completed` event the
        // entry never arms (which is the whole point — sink Ok is
        // unreliable for blocking actions).
        // `mk_engine_with_bus` doesn't enable completion fan-out, so
        // construct directly with a registry that has the bus wired.
        let bus2 = Arc::new(crate::event_bus::EventBus::new());
        let reg2 = Arc::new(ActionRegistry::with_completion_bus(bus2.clone()));
        reg2.register("noop", |_p| Ok(json!({"ok": true})));
        let engine = TriggerEngine::with_publish_bus(reg2, bus2.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        // We need the engine to also see the `noop.completed` event
        // that the registry publishes synchronously. Subscribe a
        // receiver, drain it, and re-dispatch into the engine — same
        // pattern turm-linux's pump uses.
        let completed_rx = bus2.subscribe("noop.completed");
        engine.dispatch(&evt("todo.start_requested", json!({"id": "T-1"})), None);
        // Preflight registered, no pending yet.
        assert_eq!(engine.preflight_await_count(), 1);
        assert_eq!(engine.pending_await_count(), 0);
        // The registry already published `noop.completed` on the bus.
        // Drain it and feed back into engine.dispatch — that's how
        // turm-linux's pump bridges between bus and engine.
        let completed = match completed_rx.try_recv() {
            Some(e) => e,
            None => panic!("expected noop.completed on bus"),
        };
        engine.dispatch(&completed, None);
        // Now pending is armed.
        assert_eq!(engine.preflight_await_count(), 0);
        assert_eq!(engine.pending_await_count(), 1);
    }

    #[test]
    fn await_does_not_register_when_action_fails_synchronously() {
        // Sync registry failure: sink returns Err immediately.
        // Preflight never gets staged, pending never armed.
        let (reg, engine, _bus) = mk_engine_with_bus();
        reg.register("fails", |_p| Err(invalid_params("boom".to_string())));
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "fails",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        engine.dispatch(&evt("todo.start_requested", json!({"id": "T-1"})), None);
        assert_eq!(engine.preflight_await_count(), 0);
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn await_preflight_dropped_on_synthetic_failed_event() {
        // Direct exercise of the `<X>.failed` drop branch: stage a
        // preflight (success path), then dispatch a synthetic
        // `<action>.failed` event. The preflight should be dropped.
        // Models the behavior turm-linux's pump produces when a
        // blocking action returns Ok({queued: true}) sync and later
        // emits `<action>.failed` async via the supervisor.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        // Drain the success completion the registry published (we
        // don't want it to promote our preflight; we want to test
        // the .failed drop path explicitly).
        let _ = bus.subscribe("noop.completed").try_recv();
        engine.dispatch(&evt("todo.start_requested", json!({})), None);
        assert_eq!(engine.preflight_await_count(), 1);
        // Dispatch a synthetic .failed sourced as the real registry
        // would (action_registry::COMPLETION_EVENT_SOURCE) so the
        // trust check accepts it.
        let failed = Event::new(
            "noop.failed",
            crate::action_registry::COMPLETION_EVENT_SOURCE,
            json!({}),
        );
        engine.dispatch(&failed, None);
        assert_eq!(engine.preflight_await_count(), 0);
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn awaited_event_fires_on_match_with_payload_namespaced() {
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        let rx = bus.subscribe("ask.awaited");
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(
            &evt("todo.start_requested", json!({"id": "T-1", "title": "x"})),
            None,
        );
        // Pump the registry's completion event into the engine.
        let completed = completed_rx.try_recv().expect("noop.completed");
        engine.dispatch(&completed, None);
        // Awaited event arrives.
        engine.dispatch(&evt("slack.dm", json!({"text": "PROJ-42"})), None);
        // Synthesized event published.
        let received = match rx.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected awaited event, got {other:?}"),
        };
        assert_eq!(received.kind, "ask.awaited");
        assert_eq!(received.payload["id"], "T-1");
        assert_eq!(received.payload["title"], "x");
        assert_eq!(received.payload["await"]["text"], "PROJ-42");
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn await_payload_match_interpolated_against_original_event() {
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        let mut pm = Map::new();
        pm.insert("user".into(), Value::String("{event.user}".into()));
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: pm,
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        let rx = bus.subscribe("ask.awaited");
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.start_requested", json!({"user": "U_M"})), None);
        let completed = completed_rx.try_recv().expect("noop.completed");
        engine.dispatch(&completed, None);
        // Wrong user — must NOT match.
        engine.dispatch(
            &evt("slack.dm", json!({"user": "U_OTHER", "text": "no"})),
            None,
        );
        assert_eq!(
            engine.pending_await_count(),
            1,
            "non-matching event must leave pending intact"
        );
        // Right user — must match.
        engine.dispatch(
            &evt("slack.dm", json!({"user": "U_M", "text": "yes"})),
            None,
        );
        let received = match rx.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected awaited event, got {other:?}"),
        };
        assert_eq!(received.payload["await"]["text"], "yes");
    }

    #[test]
    fn sweep_drops_expired_pendings_with_abort() {
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 0, // immediate expiry
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.start_requested", json!({})), None);
        let completed = completed_rx.try_recv().expect("noop.completed");
        engine.dispatch(&completed, None);
        assert_eq!(engine.pending_await_count(), 1);
        // Sleep a tick past the deadline so Instant::now() > deadline.
        std::thread::sleep(Duration::from_millis(5));
        engine.sweep_pending_awaits();
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn sweep_fires_default_event_on_timeout_when_policy_set() {
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 0,
                on_timeout: TimeoutPolicy::FireWithDefault,
            },
        )]);
        let rx = bus.subscribe("ask.awaited");
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.start_requested", json!({"id": "T-9"})), None);
        let completed = completed_rx.try_recv().expect("noop.completed");
        engine.dispatch(&completed, None);
        std::thread::sleep(Duration::from_millis(5));
        engine.sweep_pending_awaits();
        let received = match rx.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected awaited timeout event, got {other:?}"),
        };
        assert_eq!(received.payload["id"], "T-9");
        assert!(
            received.payload["await"].is_null(),
            "fire_with_default sets await to null"
        );
    }

    #[test]
    fn sweep_drops_preflight_when_action_never_completes() {
        // A trigger fires its action, but `<action>.completed` never
        // arrives (legacy match-arm action that doesn't go through
        // the registry, or stalled action). Preflight expires and
        // is dropped.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        // No completion bus on this registry so noop.completed is
        // never published — simulates the legacy/stalled case.
        let reg = Arc::new(ActionRegistry::new());
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 0,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        engine.dispatch(&evt("todo.start_requested", json!({})), None);
        assert_eq!(engine.preflight_await_count(), 1);
        std::thread::sleep(Duration::from_millis(5));
        engine.sweep_pending_awaits();
        assert_eq!(engine.preflight_await_count(), 0);
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn one_event_satisfies_only_one_pending_when_criteria_overlap() {
        // Two preflights with identical match criteria: a single
        // matching event should resolve only ONE of them. Broadcasting
        // one reply to multiple concurrent prompts would silently
        // double-fire downstream chains.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        let mut pm = Map::new();
        pm.insert("user".into(), Value::String("U_M".into()));
        engine.set_triggers(vec![
            trig_with_await(
                "ask-1",
                "noop",
                "todo.a",
                AwaitClause {
                    event_kind: "reply".into(),
                    payload_match: pm.clone(),
                    timeout_seconds: 60,
                    on_timeout: TimeoutPolicy::Abort,
                },
            ),
            trig_with_await(
                "ask-2",
                "noop",
                "todo.b",
                AwaitClause {
                    event_kind: "reply".into(),
                    payload_match: pm,
                    timeout_seconds: 60,
                    on_timeout: TimeoutPolicy::Abort,
                },
            ),
        ]);
        let rx_1 = bus.subscribe("ask-1.awaited");
        let rx_2 = bus.subscribe("ask-2.awaited");
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.a", json!({})), None);
        engine.dispatch(&evt("todo.b", json!({})), None);
        let c1 = completed_rx.try_recv().expect("completion 1");
        engine.dispatch(&c1, None);
        let c2 = completed_rx.try_recv().expect("completion 2");
        engine.dispatch(&c2, None);
        assert_eq!(engine.pending_await_count(), 2);
        // ONE matching event arrives.
        engine.dispatch(&evt("reply", json!({"user": "U_M", "text": "hi"})), None);
        // First-staged pending fires; second remains.
        let received = match rx_1.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected ask-1.awaited, got {other:?}"),
        };
        assert_eq!(received.payload["await"]["text"], "hi");
        match rx_2.recv_timeout(Duration::from_millis(20)) {
            crate::event_bus::RecvOutcome::Timeout => {}
            other => panic!("ask-2.awaited should NOT fire on the same event: {other:?}"),
        }
        assert_eq!(engine.pending_await_count(), 1);
    }

    #[test]
    fn promote_drop_ignores_events_not_sourced_from_action_registry() {
        // An event with kind `noop.completed` but source != "turm.action"
        // (e.g. a user-published event mimicking the suffix) must NOT
        // advance the await state machine. Only the registry's
        // synthetic completion fan-out is trusted.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "ask",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        // Drain the real completion the registry will publish.
        let _ = bus.subscribe("noop.completed").try_recv();
        engine.dispatch(&evt("todo.start_requested", json!({})), None);
        assert_eq!(engine.preflight_await_count(), 1);
        // Synthetic event with the right kind but WRONG source.
        let spoofed = evt("noop.completed", json!({}));
        // `evt` helper sets source = "test", so this is the spoof shape.
        assert_eq!(spoofed.source, "test");
        engine.dispatch(&spoofed, None);
        assert_eq!(
            engine.preflight_await_count(),
            1,
            "spoofed completion must not advance the state machine"
        );
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn await_payload_match_preserves_numeric_types() {
        // payload_match = { count = "{event.count}" } where event.count
        // is a number (not a string) must compare against awaited.count
        // as a number. interpolate_value would have coerced to
        // Value::String("42") and missed Value::Number(42) on the
        // awaited side. interpolate_value_typed unwraps the single
        // token and preserves the raw JSON Value.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        let mut pm = Map::new();
        pm.insert("count".into(), Value::String("{event.count}".into()));
        engine.set_triggers(vec![trig_with_await(
            "ask-num",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "reply".into(),
                payload_match: pm,
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        let rx = bus.subscribe("ask-num.awaited");
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.start_requested", json!({"count": 42})), None);
        let completed = completed_rx.try_recv().expect("noop.completed");
        engine.dispatch(&completed, None);
        // Awaited event with count=42 (number, not string) — must match.
        engine.dispatch(&evt("reply", json!({"count": 42, "text": "ok"})), None);
        let received = match rx.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Event(e) => e,
            other => panic!("expected ask-num.awaited, got {other:?}"),
        };
        assert_eq!(received.payload["await"]["text"], "ok");
    }

    #[test]
    fn dispatching_completion_does_not_consume_freshly_registered_preflight() {
        // Round-3 ordering fix: a dispatch event of `X.completed` that
        // also matches a trigger which fires action X must NOT have
        // its newly-registered preflight consumed by the same event.
        // The preflight should wait for a FUTURE completion.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "chained-ask",
            "noop",
            "noop.completed", // fires ON a completion event
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        // Manually feed a `noop.completed` event in. Trigger fires
        // (action = noop), registers preflight. The same event must
        // NOT immediately promote its own preflight.
        let synthetic_completed = evt("noop.completed", json!({"ok": true}));
        engine.dispatch(&synthetic_completed, None);
        assert_eq!(
            engine.preflight_await_count(),
            1,
            "preflight registered by current dispatch must survive — promotion should wait for the NEXT completion"
        );
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn set_triggers_clears_in_flight_await_state() {
        // Round-3 hot-reload contract: replacing the trigger list
        // must also drop preflight + pending entries that referenced
        // the old config. Otherwise a removed trigger could still
        // emit `<old_name>.awaited` after reload.
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        engine.set_triggers(vec![trig_with_await(
            "old-trigger",
            "noop",
            "todo.start_requested",
            AwaitClause {
                event_kind: "slack.dm".into(),
                payload_match: Map::new(),
                timeout_seconds: 60,
                on_timeout: TimeoutPolicy::Abort,
            },
        )]);
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.start_requested", json!({})), None);
        let completed = completed_rx.try_recv().expect("noop.completed");
        engine.dispatch(&completed, None);
        assert_eq!(engine.pending_await_count(), 1);
        // Reload with a different trigger set.
        engine.set_triggers(vec![]);
        assert_eq!(engine.preflight_await_count(), 0);
        assert_eq!(engine.pending_await_count(), 0);
    }

    #[test]
    fn multiple_pendings_only_matched_one_fires() {
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let reg = Arc::new(ActionRegistry::with_completion_bus(bus.clone()));
        reg.register("noop", |_p| Ok(json!({})));
        let engine = TriggerEngine::with_publish_bus(reg, bus.clone());
        let mut pm_a = Map::new();
        pm_a.insert("id".into(), Value::String("{event.id}".into()));
        let mut pm_b = Map::new();
        pm_b.insert("id".into(), Value::String("{event.id}".into()));
        engine.set_triggers(vec![
            trig_with_await(
                "ask-a",
                "noop",
                "todo.a",
                AwaitClause {
                    event_kind: "reply".into(),
                    payload_match: pm_a,
                    timeout_seconds: 60,
                    on_timeout: TimeoutPolicy::Abort,
                },
            ),
            trig_with_await(
                "ask-b",
                "noop",
                "todo.b",
                AwaitClause {
                    event_kind: "reply".into(),
                    payload_match: pm_b,
                    timeout_seconds: 60,
                    on_timeout: TimeoutPolicy::Abort,
                },
            ),
        ]);
        let rx_a = bus.subscribe("ask-a.awaited");
        let rx_b = bus.subscribe("ask-b.awaited");
        let completed_rx = bus.subscribe("noop.completed");
        engine.dispatch(&evt("todo.a", json!({"id": "A"})), None);
        engine.dispatch(&evt("todo.b", json!({"id": "B"})), None);
        // Both preflights staged; promote both via two completed events.
        let c1 = completed_rx.try_recv().expect("first noop.completed");
        engine.dispatch(&c1, None);
        let c2 = completed_rx.try_recv().expect("second noop.completed");
        engine.dispatch(&c2, None);
        assert_eq!(engine.pending_await_count(), 2);
        // Reply matching B's id only.
        engine.dispatch(&evt("reply", json!({"id": "B", "text": "ok-B"})), None);
        match rx_b.recv_timeout(Duration::from_millis(50)) {
            crate::event_bus::RecvOutcome::Event(e) => {
                assert_eq!(e.payload["await"]["text"], "ok-B");
            }
            other => panic!("expected ask-b.awaited, got {other:?}"),
        }
        // ask-a should NOT have fired.
        match rx_a.recv_timeout(Duration::from_millis(20)) {
            crate::event_bus::RecvOutcome::Timeout => {}
            other => panic!("ask-a.awaited should NOT fire: {other:?}"),
        }
        assert_eq!(engine.pending_await_count(), 1);
    }
}
