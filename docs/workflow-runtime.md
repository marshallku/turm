# Workflow Runtime

turm's long-term identity: a personal workflow runtime that happens to surface through a terminal. Services (calendar, messengers, docs, knowledge base), triggers, and the AI agent all plug into three shared abstractions in `turm-core` so that adding an integration is `1 new source + N actions`, not `1 new source × N consumers of wiring`.

## Why three abstractions

Without shared primitives, each new service needs its own:

- event stream plumbing (source → socket → subscribers)
- action invocation surface (socket command, CLI subcommand, plugin JS call, AI tool)
- "what is the user doing right now" logic

Three abstractions collapse this fan-out:

| Abstraction | Collapses |
|---|---|
| **Event Bus** | All inbound signals (shell, VTE, calendar ticks, slack events, git hooks, timers) |
| **Action Registry** | All outbound effects (spawn tab, send message, create event, query doc) |
| **Context Service** | All "current state" queries (cwd, active tab, next meeting, recent mentions) |

Every new integration registers sources with the bus, actions with the registry, and optionally contributes to context. Triggers, UI panels, plugins, and the AI agent all _consume_ these three — nothing else.

## Event Bus

Pub/sub hub in `turm-core`. All lifetime events flow through it; the socket `event.subscribe` stream becomes a thin projection to external clients rather than the event system itself.

### Event shape

```rust
pub struct Event {
    pub kind: String,          // "terminal.cwd_changed", "calendar.event_imminent", "slack.mention"
    pub source: String,        // "shell", "calendar", "slack", …
    pub timestamp: SystemTime,
    pub payload: serde_json::Value,
}
```

`kind` is a dotted string, not an enum — plugins and external integrations must publish new event kinds without modifying the core crate.

### Publish / subscribe

```rust
pub trait EventBus: Send + Sync {
    fn publish(&self, event: Event);
    fn subscribe(&self, pattern: &str) -> EventReceiver;        // glob on kind, e.g. "calendar.*"
    fn subscribe_filtered(&self, f: Box<dyn Fn(&Event) -> bool + Send>) -> EventReceiver;
}
```

Delivery is non-blocking via bounded `mpsc` channels per subscriber (already the pattern used in `turm-macos/EventBus.swift`). Slow subscribers never block publishers; when a subscriber's buffer is full the incoming event is dropped for that subscriber with a warn log (`try_send` semantics). Disconnected subscribers are cleaned up lazily on the next publish.

### Relationship to existing systems

- Existing socket `event.subscribe` remains the external API; internally it becomes `bus.subscribe("*")` and serializes events to JSON over the socket.
- VTE signal handlers, focus controllers, tab manager — all refactored to publish through the bus instead of calling socket broadcast directly.
- Platform UIs subscribe to relevant kinds on the GTK / main thread via the existing `mpsc → glib::timeout_add_local` bridging pattern.

## Action Registry

Name → handler map. Every capability turm exposes (today's socket commands, plugin shell commands, future service actions) registers here. The socket dispatcher, CLI, plugin `turm.call()`, keybindings, triggers, and the AI agent all resolve through the same registry.

### Action shape

```rust
pub struct ActionSpec {
    pub name: String,               // "tab.new", "calendar.create_event", "plugin.notes.search"
    pub title: String,              // human-readable for command palette
    pub params_schema: JsonSchema,  // for AI agent tool use + command palette validation
    pub context_scope: Scope,       // requires active terminal id, global, plugin-scoped, …
}

pub trait ActionHandler: Send + Sync {
    fn invoke(&self, params: Value, ctx: &ActionContext) -> BoxFuture<Result<Value, ActionError>>;
}
```

Handlers are async so that service calls (HTTP, WebSocket) are non-blocking without reinventing completion-based wrappers per call site.

### Invocation sources

| Source | Path |
|---|---|
| Socket command | dispatcher looks up `method` in registry |
| CLI (`turmctl`) | subcommands map to action names |
| Plugin JS bridge | `turm.call(name, params)` |
| Keybinding | config maps key → `{action, params}` |
| Trigger | config rule's `action` field |
| AI agent | registry emits JSON Schemas as tool definitions |
| Command palette (future) | fuzzy search over registry titles |

Existing socket commands migrate _incrementally_. The dispatcher keeps its hard-coded match for a while; new commands register through the registry from day one. No big-bang refactor.

## Context Service

Centralized read model of "what is the user doing right now." Services contribute, any consumer can query. This is what makes triggers and the AI agent feel aware rather than blind.

### Shape

```rust
pub struct Context {
    pub active_panel: Option<PanelRef>,       // terminal / webview / plugin
    pub active_cwd: Option<PathBuf>,
    pub active_shell_cmd: Option<String>,     // last preexec'd command
    pub recent_commits: Vec<CommitRef>,       // cwd-scoped, populated lazily
    pub upcoming_events: Vec<CalendarEvent>,  // next few hours
    pub unread_mentions: Vec<MentionRef>,     // slack / discord
    pub open_documents: Vec<DocRef>,          // notion / obsidian recent
    // extensible — each contributor registers a provider
}
```

### Contributors

Services register as providers that either subscribe to bus events or poll on a schedule, writing into their slot of the Context:

```rust
pub trait ContextProvider: Send + Sync {
    fn name(&self) -> &str;
    fn refresh(&self, ctx: &mut Context);
}
```

The preferred pattern: every provider subscribes to its own event kinds on the bus and updates the Context incrementally. Polling is the fallback for services without push (e.g., polling Calendar every 2 minutes).

### Consumers

- **Triggers**: `when = { event_starts_in = "5m" }` resolves against `upcoming_events`
- **AI agent**: Phase 6's `terminal.context` grows into `workflow.context` with all fields
- **Command palette**: context-aware suggestions (e.g., show Notion docs matching cwd's project name)
- **Status bar plugins**: "next meeting in 23m" module reads from context

## Triggers (config-driven automation)

Triggers are the user-facing feature that makes the three abstractions worth their weight. A trigger is `(event pattern or schedule) → action`, defined declaratively in config and hot-reloadable (turm already has config hot-reload).

```toml
[[triggers]]
name = "meeting-prep"
when = { event_kind = "calendar.event_imminent", minutes = 10 }
action = "plugin.notion.open_event_doc"
params = { event_id = "{event.id}" }

[[triggers]]
name = "slack-alert"
when = { event_kind = "slack.mention", channel = "alerts" }
action = "terminal.exec"
params = { command = "echo {text} >> ~/pings.log" }
```

Parameter interpolation (`{event.foo}`, `{context.bar}`) is the only logic triggers need for v1. Users who want conditional logic can point at a plugin action that does the filtering.

## Mapping to existing code

| Current | Becomes |
|---|---|
| `turm-linux/socket.rs` event broadcast | thin `EventBus::publish` caller; external subscribers consume through bus |
| `turm-linux/socket.rs` `dispatch()` match | Action Registry lookup (with hard-coded commands migrating incrementally) |
| Phase 6 `terminal.context` | one slot of Context Service output |
| Plugin `turm.call()` | already an Action Registry consumer — no change in surface |
| Plugin `turm.on()` | already an Event Bus consumer |
| `turm-macos/EventBus.swift` | platform-native mirror of the core bus — already shaped correctly |

Most existing features are already shaped correctly; the refactor is about unifying the internal plumbing that they all share.

## First vertical PoC

One concrete workflow that exercises all three abstractions end-to-end:

1. Google Calendar provider (OAuth + polling) publishes `calendar.event_imminent` events to the bus at 10 / 5 / 1 min before each meeting.
2. Provider also maintains `Context.upcoming_events`.
3. Trigger rule: `calendar.event_imminent (10m)` → action `workflow.meeting_prep`.
4. Action `workflow.meeting_prep` (built-in): opens the meeting link in a new tab and opens the Notion page whose title matches the event title in a WebView panel split.
5. User verifies: 10 minutes before a real meeting, two panels appear automatically.

This PoC forces implementation of all three abstractions at minimal-viable level. Every later integration reuses the plumbing.

## Non-goals (v1)

- No DSL for trigger conditions beyond equality and time windows. Complex conditions belong in plugins.
- No multi-user / shared state. Single user, single machine.
- No natural-language trigger authoring. TOML files only for now.
- Knowledge base semantic search is _later_ — built on top of Context Service, not a parallel system.

## Scope guardrails

- **Never build service clients from scratch** when a mature web UI exists. Embed in a WebView panel instead (`webkit6` on Linux, `WKWebView` on macOS — the existing `PanelVariant::WebView`).
- **Implement native event streams** only for persistent push (WebSocket gateways, webhooks). Everything else polls via provider.
- **Knowledge base is the last layer.** Do not start it until all three abstractions plus at least two real service integrations are in place.
