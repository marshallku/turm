# Service Plugins: turm as a Plugin-First Personal Workflow Runtime

> Status: planning doc. Captures end-state vision, the architectural pivot to plugin-first, decisions and rationale, and the concrete roadmap.

## End picture

At maturity, turm is a **plugin host** for personal workflow automation. The binaries shipped (`turm`, `turmctl`) own the runtime primitives only — Event Bus, Action Registry, Trigger Engine, Context Service, Plugin Loader. Every domain integration (KB, Calendar, Slack, Discord, Notion, LLM, …) lives outside `turm-core`/`turm-linux` as a service plugin: an independent process with its own release cadence, language choice, dependencies, and credentials. The user can swap KB backends (file-based vs Notion vs Obsidian-vault), pick which messenger they want ingested, and write their own integrations without rebuilding turm.

The pattern is identical to VSCode (extensions in a separate process), Neovim (remote plugins via msgpack-rpc), and the LSP ecosystem (servers as separate processes, JSON-RPC over stdio): a small core, a documented protocol, and pluggable everything else. Triggers in TOML config wire bus events to actions; whether the action is satisfied by a built-in handler or a service plugin is invisible to the user.

A working day in this end state:
1. `turm` starts. KB plugin spawns lazily on first `kb.*` call. Calendar plugin starts on `onStartup` because it needs to poll. Slack plugin starts on `onStartup` to keep its WebSocket alive.
2. 10 minutes before a meeting, Calendar plugin publishes `calendar.event_imminent`. A user-defined trigger fires `kb.ensure` (handled by KB plugin) which creates-or-returns `~/docs/meetings/<event_id>.md`, and a follow-up step (chained trigger or composite-action helper, TBD post-Phase 9; see Open questions) opens that path in a WebView panel.
3. A Slack mention arrives. Slack plugin publishes `slack.mention`. A trigger writes the raw thread to `~/docs/.raw/slack/...` and asks the LLM plugin to write a derived summary at `~/docs/threads/<topic>.md`.
4. The user types `turmctl draft-reply <topic>`. CLI invokes a registry action that asks the LLM plugin (with KB context as input) to produce a draft reply, opening it in the active terminal.

None of this requires changes to `turm-core`. New integrations are new plugins.

## Why this evolution

The Phase 8 work (`docs/workflow-runtime.md`) gave us the runtime — primitives that are conceptually identical to a plugin host. What was missing was the **service plugin** model: a long-running supervised process that publishes events and serves actions. The Phase 7 plugin system (`docs/plugins.md`) handles UI panels (HTML/JS in WebView) and shell-script commands (one-shot subprocess), but cannot host providers like a Calendar polling daemon or a Slack WebSocket gateway.

Without service plugins, every integration would need to live inside `turm-core` or `turm-linux`. Calendar OAuth flow, Slack WebSocket, Notion API, LLM credential management — all in core. That makes turm a kitchen-sink monolith, locks the user to whatever backends turm chooses (no Obsidian-vault KB, no per-user LLM provider), and makes contribution from outside the main repo very hard.

Plugin-first solves all of this and aligns with how every comparable mature tool (editors and IDEs) architects their integration surface.

## Context: what's already built

### Phase 8 runtime primitives (turm-core)
- `EventBus`: pub/sub. `subscribe(pattern)` (bounded, drop-newest), `subscribe_unbounded(pattern)` (lossless wire contract). 11 unit tests.
- `ActionRegistry`: `Send + Sync` `Fn(Value) -> Result<Value, ResponseError>` handlers, `Arc<dyn Fn>` for safe nested invoke. 12 unit tests.
- `ContextService`: live snapshot from bus events. 10 unit tests.
- `TriggerEngine`: TOML triggers, `{event.*}/{context.*}` interpolation, `covering_patterns` dedup, `TriggerSink` trait. 19 unit tests.

### Phase 7 plugin system (turm-linux)
- `~/.config/turm/plugins/<name>/plugin.toml` manifest with:
  - `[[panels]]` — HTML/JS in WebView, `turm.call()` / `turm.on()` JS bridge
  - `[[commands]]` — fork-per-call shell scripts; auto-registered as `plugin.<name>.<cmd>` socket method
  - `[[modules]]` — status bar widgets

### What's missing for service plugins
- Long-running supervised process model (current `[[commands]]` are one-shot)
- Plugin → bus event publish (current panels can only receive via `turm.on`)
- Explicit, manifest-declared action ownership (current is implicit via `plugin.<name>.<cmd>` autonaming)
- Lazy activation lifecycle (current panels activate on user open, commands on call — no abstraction over startup-vs-on-demand for service plugins)

## Decisions and rationale

### D1: Plugin-first, not core-first
**Decision.** KB, Calendar, Slack, Notion, LLM are service plugins, not modules in `turm-core` / `turm-linux`. The KB *action protocol* lives in `turm-core` as documented contract; the *implementation* is a plugin (`turm-plugin-kb`).

**Rationale.** This is the user's stated mental model (VSCode-style). It also matches how every successful editor/IDE architects integrations: VSCode (extensions in extension host process), Neovim (remote plugins), Zed (WASM extensions). Building integrations into core forecloses on user choice (KB backend swap), couples release cadence, and makes third-party contribution painful.

**Alternative considered:** ship KB in core first, refactor later. Rejected because "later" rarely happens and because the protocol decisions made in core become silent constraints on every future plugin. Better to design the boundary first.

### D2: Subprocess + stdin/stdout + newline-JSON
**Decision.** Service plugins are subprocesses. Communication via newline-delimited JSON over stdin/stdout, reusing the existing cmux V2 protocol (already used by socket clients, plugin commands).

**Rationale.** Industry standard:
- LSP: JSON-RPC 2.0 over stdio (the dominant pattern for editor extensions)
- VSCode language servers: stdio + JSON
- Neovim remote plugins: msgpack-rpc, transport-agnostic but stdio is canonical
- WASM (Zed) is the alternative but adds significant complexity (Wasmtime runtime, WIT compilation step) that personal-scale turm doesn't yet need

cmux V2 newline-delimited JSON is already proven in turm. Sticking with it avoids two protocols. LSP-style `Content-Length` header framing is unnecessary as long as JSON serializers don't emit raw newlines (serde_json doesn't).

**Sources:** LSP spec 3.17, VSCode Language Server Extension Guide, Neovim channel docs, Zed extension architecture (see Sources at end).

### D3: Lazy activation by default
**Decision.** Service plugins declare an `activation` event; turm spawns the process only when that event fires. Examples:
- `onStartup` — for plugins that must run from boot (Calendar polling, Slack WebSocket)
- `onAction:kb.*` — spawn on first `kb.*` action invocation
- `onEvent:slack.*` — spawn when something triggers an event in that namespace

**Rationale.** This is what VSCode does. Earlier in this design conversation I (Claude) argued for eager startup on the grounds that lazy adds complexity and 5-10 plugins is small enough. Internet research validated that VSCode chose lazy from the start, and progressively made it MORE lazy (1.74+ implicit activation from `contributes` declarations). Reasons:
- Startup time scales with N plugins
- Memory cost of plugins user doesn't currently use
- Crash blast radius — a plugin that's not running can't crash

The implementation cost of lazy is small: a state machine (`not-started` / `starting` / `ready`) + first-call request buffering. ~200 LOC.

**Alternative considered:** eager-on-startup. Rejected after research showed it's the wrong default at any non-trivial plugin count.

### D4: Manifest-declared capabilities, deterministic conflict resolution
**Decision.** Each `[[services]]` declares its full capability set IN THE MANIFEST: `provides = ["kb.search", "kb.read"]` and `subscribes = ["calendar.event_imminent"]`. The manifest is the source of truth. At plugin load time (BEFORE spawning any process), turm:

1. Walks every enabled plugin's manifest, building a global action-ownership table.
2. **Detects conflicts deterministically by lexical order of plugin name.** When two enabled plugins both declare `provides = ["kb.search"]`, the alphabetically-earlier plugin name wins; the loser's `kb.search` registration is skipped (its other declared actions still register normally). A warning is logged identifying the conflict and which plugin won.
3. Spawns plugins per their `activation` rule.
4. The plugin's `initialize` response is checked asymmetrically against the manifest, applied identically to BOTH `provides` AND `subscribes`:
   - **Subset OK (degraded mode):** the runtime may declare FEWER entries than the manifest. Use case: the plugin started but a backend dependency failed (e.g. `kb-search` library missing) — it can still serve `kb.read` and `kb.append`. turm only wires up what the runtime actually declared.
   - **Superset rejected:** the runtime may NOT declare entries beyond the manifest. Any extras are dropped with a warning; the plugin keeps running for its manifest-approved set. Reason: the pre-spawn ownership analysis (D4 step 1) must stay accurate. Letting plugins claim more at runtime would invalidate the conflict resolution turm already committed to.
   
   This applies symmetrically: provides-superset would let a plugin grab actions another plugin won (already resolved); subscribes-superset would force turm to forward event kinds the global subscription set didn't account for.

**Plugin identity is the manifest `[plugin].name` field** (consistent with the existing plugin contract in [plugins.md](./plugins.md)), NOT the directory. Lexical comparison is on that name. User controls precedence by:

1. Enabling/disabling plugins (the primary lever — most users will simply not install both).
2. Editing the manifest `[plugin].name` of a plugin to change its sort key (e.g. rename `[plugin].name` from `kb-obsidian` to `aaa-kb-obsidian` to force it to win).

Future enhancement: explicit `~/.config/turm/plugin-precedence.toml` if name-based control proves too indirect.

**Rationale.**
- **Manifest as source of truth** lets turm validate the whole plugin set BEFORE spawning anything. No race conditions, no "depends on which plugin started first" weirdness.
- **Lexical name ordering** is deterministic across runs and OS / filesystem variations. Filesystem mtime, install order, or process startup order would all be fragile.
- **Initialize response confirms manifest** so a plugin can't silently expand its capability set after manifest review. The plugin can declare LESS than its manifest at runtime (degraded mode) but can't claim more than the manifest authorized.

VSCode is more permissive at runtime (last-registered command wins) but VSCode also requires manifest declaration of contributions, so the failure mode is mostly identical: predictable from manifest inspection. turm's stricter "manifest is the truth" is safer for the personal-use single-machine scenario where a stale-process race is more annoying than helpful.

### D5: Initialization handshake (LSP style)
**Decision.** When turm spawns a service, the first exchange is an `initialize` request from turm (`{turm_version, protocol_version}`), and the service responds with its full capability snapshot (`{provides, subscribes, version}`). Then `initialized` notification flows. Only after this does normal RPC begin.

**Rationale.** LSP's pattern, validated for 8+ years across hundreds of language servers and clients. Benefits:
- Version negotiation: turm and plugin can refuse incompatible versions cleanly
- Capability discovery: turm doesn't infer; plugin declares
- Clear lifecycle phase boundary: setup vs running

Without this, `action.register` calls trickling in unordered creates ambiguity (when is the plugin "ready"?).

### D6: KB action protocol in turm-core, implementation in plugin
**Decision.** `turm-core` ships a `docs/kb-protocol.md` defining what `kb.search` / `kb.read` / `kb.append` / `kb.ensure` accept and return. **No KB code in turm-core.** A first-party `turm-plugin-kb` (separate Cargo crate or even separate repo) implements grep over `~/docs`. Other backends (Notion, Obsidian) are alternative plugins.

The KB `id` is a logical `<folder>/<filename>`-style path-like key, the same shape across every backend. FS backends use it as a relative path; non-FS backends translate it to their internal UUIDs / vault IDs. This shape is load-bearing for the rest of the protocol surface (parent-folder auto-create on `kb.ensure`, `.raw/` search exclusion, `kb.search.folder` prefix filter, caller-constructed ids like `meetings/{event.id}.md` in triggers) — those affordances only work if every backend agrees on the path-like shape. See [kb-protocol.md](./kb-protocol.md) Design constraints (2) and (3) for the precise contract.

**Rationale.** LSP's design split: the protocol defines what's possible; servers implement it. This decouples the contract from any specific implementation. `~/docs` is the user's chosen backend; making it a plugin means others can swap it without modifying core. And the contract becomes a stable boundary that triggers, AI agents, and command palette all rely on without caring who serves it.

### D7: Backward compatibility with existing `[[commands]]`
**Decision.** The `[[services]]` model is purely additive. Existing shell-script `[[commands]]` plugins keep working unchanged.

**Rationale.** Users have already invested in the existing plugin format. We're adding a new lifecycle option, not migrating commands. Old plugin = subprocess-per-call (current); new service plugin = supervised long-running process. Both can coexist in one plugin.toml.

### D8: Defer LLM as an action
**Decision.** No `claude.complete` (or similar) plugin shipped in the first vertical. The Calendar PoC opens the relevant note for the user; LLM-driven prep is a future plugin.

**Rationale.** LLM adds:
- Recurring API cost ($)
- Credential management complexity
- Network failure modes
- A new dependency on output quality

Calendar + KB without LLM is already useful — auto-opening the right note before a meeting is high signal. LLM amplifies but doesn't unlock the workflow. Better to ship the surface first, add LLM when the user feels its absence.

**Tradeoff (acknowledged):** "smart" demos arrive later. The system feels less impressive at first. Acceptable given turm is for the user's own use, not external demo.

### D9: Defer KB indexing upgrades; design contract to allow them
**Decision.** First KB plugin is grep + filename match over `~/docs`. No SQLite FTS, no embeddings. But the action protocol (D6) is shaped so that `kb.search` returns ranked `KbHit { id, score, snippet }` results — already compatible with FTS or vector search later. Internal storage of the plugin can change without breaking the protocol.

**Rationale.** Personal scale (~10k docs) is fine for grep. Indexing matters only when grep gets slow on every search. Building it now is premature — but designing the action contract NOT to preclude it is essential.

## Architecture

### Manifest extension

```toml
# ~/.config/turm/plugins/kb/plugin.toml

[plugin]
name = "kb"
title = "Knowledge Base"
version = "1.0.0"

[[services]]
name = "main"                      # service identifier within the plugin
exec = "turm-plugin-kb"            # binary in PATH or relative to plugin dir
activation = "onAction:kb.*"       # lazy: spawn on first kb.* action
restart = "on-crash"               # other: "never", "always"
provides = ["kb.search", "kb.read", "kb.append", "kb.ensure"]  # actions this service handles
subscribes = []                    # bus event-kind globs the service wants forwarded
# alt: activation = "onStartup"    # eager (calendar polling, slack gateway)
# alt: activation = "onEvent:foo.*"
```

Existing sections (`[[panels]]`, `[[commands]]`, `[[modules]]`) coexist unchanged.

### Initialization handshake

```
turm spawns the service binary.

turm → service (stdin):
{
  "id": "init-1",
  "method": "initialize",
  "params": {
    "turm_version": "0.x.y",
    "protocol_version": 1
  }
}

service → turm (stdout):
{
  "id": "init-1",
  "ok": true,
  "result": {
    "service_version": "1.0.0",
    "provides": ["kb.search", "kb.read", "kb.append", "kb.ensure"],
    "subscribes": []
  }
}

turm → service:
{
  "id": "init-2",
  "method": "initialized",
  "params": {}
}

# Normal RPC begins.
```

If `provides` conflicts with another loaded plugin, turm logs a warning and skips registration of the conflicting names; non-conflicting names still register.

### Bidirectional RPC (newline-JSON over stdio)

#### turm → service

| Method | Params | Notes |
|---|---|---|
| `initialize` | `{turm_version, protocol_version}` | first message |
| `initialized` | `{}` | ack of init |
| `action.invoke` | `{name, params}` | service is the registered handler |
| `event.dispatch` | `{kind, source, timestamp_ms, payload}` | matches a `subscribes` pattern |
| `shutdown` | `{}` | clean stop request |

#### service → turm

| Method | Params | Notes |
|---|---|---|
| `event.publish` | `{kind, payload}` | publishes to bus; turm fills source/timestamp |
| `action.invoke` | `{name, params}` | call ANOTHER service's action |
| `log` | `{level, message}` | stderr-style logging routed via turm |

### Lifecycle and supervision

**States** per service: `Stopped` → `Starting` → `Running` → (`Crashed` | `Stopped`) → restart-or-stay-stopped.

**Activation events** trigger a transition `Stopped → Starting`:
- `onStartup` — fires immediately at turm boot
- `onAction:<glob>` — fires when an action matching the glob is invoked
- `onEvent:<glob>` — fires when an event matching the glob is published (rare)

**During `Starting`**, requests for the service are buffered (bounded, e.g. 64 deep). When `initialized` arrives, buffer drains in arrival order. If the service doesn't initialize within a timeout (5s default), it's marked failed; pending invokes return `ResponseError { code: "service_unavailable" }`.

**`restart` policies**:
- `on-crash` — restart only on non-zero exit; back off exponentially on repeated failures (1s, 2s, 4s, capped at 60s)
- `always` — restart even on clean exit
- `never` — log and stay stopped

## Roadmap

Each "Turn N.x" is one commit-sized unit (codex review + save.sh).

### Phase 9: Service Plugin Protocol & Host

**9.1 Protocol design + supervisor + mock echo plugin** (turn 1, this phase)
- Implement service-plugin manifest parsing in `turm-core::plugin` — `[[services]]` with `name`, `exec`, `activation`, `restart`, **`provides`**, **`subscribes`**.
- Add supervisor in `turm-linux` (spawn, monitor stdio, restart on policy)
- Wire initialization handshake: turm→service `initialize` carrying `{turm_version, protocol_version}`; service→turm reply with capability snapshot covering both `provides` and `subscribes`. Compare reply to manifest with the same asymmetric rule applied identically to both fields: subset OK (degraded), superset rejected with warn — plugin keeps serving its manifest-approved set so the pre-spawn ownership/subscription analysis stays accurate.
- Wire bidirectional RPC: turm→service `action.invoke` / `event.dispatch`; service→turm `event.publish` / `log`
- Lazy activation: `onStartup`, `onAction:<glob>`, `onEvent:<glob>`. Buffer requests during `Starting`. Init timeout → `service_unavailable`.
- Deterministic conflict resolution: walk all enabled plugin manifests at load time, build the global action-ownership table BEFORE spawning anything; on `provides` collision, the alphabetically-earlier `[plugin].name` wins, others skip just the conflicting entry (rest of their declarations register normally). Warn loudly with both names + the conflicting action.
- Mock plugin: a Rust binary `turm-plugin-echo` with `activation = "onStartup"`, registers action `echo.ping`, publishes `system.heartbeat` every 30s. Practically useful as a debug heartbeat. Verifies protocol shape.

**9.2 KB action protocol** (turn 2) — DONE, see [kb-protocol.md](./kb-protocol.md)
- ✅ `docs/kb-protocol.md` ships request/response shapes for `kb.search`/`kb.read`/`kb.append`/`kb.ensure` plus shared error codes. Backend-agnostic: hit `id` is the stable round-trip handle; `score` is relative-only; `path` is best-effort (FS backends populate, others null); `match_kind` is forward-compat for FTS5 / vector / semantic search.
- ✅ Folder conventions documented (`meetings/`, `people/`, `threads/`, `notes/`, `.raw/`).
- ✅ Forward-compat notes pin down which fields are reserved for backward-compat additions vs which require a protocol version bump.

**9.3 First-party KB plugin** (turn 3)
- `turm-plugin-kb` Rust binary: registers `kb.*` actions, grep + filename search over `~/docs`
- Lazy activation `onAction:kb.*`
- E2E: `turmctl call kb.search "meeting"` returns hits

### Phase 10: Calendar (first vertical PoC)

**10.1 Calendar plugin scaffold** (turn 1)
- `turm-plugin-calendar` Rust binary
- Google Calendar OAuth flow (device-code or browser-redirect)
- Polling loop, publishes `calendar.event_imminent` at lead times configured in plugin's own settings file
- `activation = "onStartup"`

**10.2 Meeting-prep trigger** (turn 2)
- TOML trigger: on `calendar.event_imminent`, call `kb.ensure` to get-or-create `~/docs/meetings/<event_id>.md`. **Auto-opening the panel is deferred** until the chained-trigger / composite-action decision (see Open questions). v1 of this slice creates/refreshes the note; the user opens it. After the chain mechanism lands, the trigger configuration upgrades to also call `webview.open` with the returned path.
- E2E: launch turm, fake or wait for a real calendar event, observe `~/docs/meetings/<event_id>.md` created/refreshed (the kb plugin's primary effect for this slice). Panel auto-open is NOT part of this E2E — it ships after the chained-trigger / composite-action decision lands.

### Phase 11: Messenger ingestion

**11.1 Slack plugin** (Discord pattern is the same once Slack works)
- Slack OAuth + WebSocket (RTM/Events API)
- Publishes `slack.mention`, `slack.dm`, etc. with payload including thread URL
- Stores raw message JSON to `~/docs/.raw/slack/...` (fidelity)

**11.2 Derived markdown ingestion trigger** (depends on Phase 12 LLM plugin existing)
- TOML trigger: on `slack.mention`, call LLM action to summarize thread, write derived markdown to `~/docs/threads/<topic>.md`

### Phase 12: LLM plugin

**12.1 `turm-plugin-llm`** (shipped — see roadmap.md for details)
- Single primitive `llm.complete` for text generation; higher-level
  patterns (`summarize`, `draft_reply`) collapse into trigger config
  with different system prompts on top of the same call.
- Anthropic provider only for v1. Multi-provider abstraction
  deferred to 12.2 — adding it before a second provider is
  committed is premature.
- Credentials: `ANTHROPIC_API_KEY` env or keyring (Linux Secret
  Service / macOS Keychain), with plaintext 0600 fallback gated
  by `TURM_LLM_REQUIRE_SECURE_STORE`. NOT the abandoned
  `~/.config/turm/secrets.toml` design — every credential-bearing
  plugin has converged on the keyring-or-plaintext pattern.
- `llm.usage` aggregates token counts from a JSONL log at
  `$XDG_DATA_HOME/turm/llm-usage-<account>.jsonl`. No USD cost
  computation — pricing tables would go stale fast; users compute
  cost in their own dashboards using `llm.usage` × current rates.
- `llm.auth_status` mirrors the slack/calendar shape.

**12.2 (deferred)** — multi-provider, streaming SSE, per-action
timeout override.

**12.3 (deferred)** — Phase 11.3-style derived markdown ingestion
that composes `kb.search` + `kb.read` + `llm.complete` +
`kb.ensure`. Needs the chained-trigger mechanism (still open).

### Phase 13: KB indexing upgrade (when grep is slow)

- SQLite FTS5 sidecar index, rebuilt on file change (filesystem watcher)
- KB plugin internal change only — protocol unchanged

## Open questions

- **Plugin distribution.** First-party plugins ship in the turm git repo as separate Cargo crates? Or fully external repos? Initially: same repo, separate crates. Distribution mechanism (registry, install command) is post-MVP.
- **Service plugin in non-Rust languages.** Protocol over stdio is language-agnostic, so a Python or Node plugin works. Need to publish a small "protocol client" library at some point. Defer until first non-Rust contributor needs it.
- ~~**Authentication / per-user secrets.** Calendar OAuth, Slack tokens. Where do they live? Probably `~/.config/turm/secrets.toml`...~~ **Resolved (Phase 10–12)**: every credential-bearing plugin uses the `keyring` crate (Linux Secret Service via D-Bus, macOS Keychain) with plaintext 0600 fallback at `$XDG_CONFIG_HOME/turm/<plugin>-token-<account>.json`. Account label scoped via env var (`TURM_<PLUGIN>_ACCOUNT`). `<PLUGIN>_REQUIRE_SECURE_STORE=1` opt-in refuses plaintext fallback. The `~/.config/turm/secrets.toml` shared-file design was abandoned — per-plugin keyring entries are simpler and prevent one plugin's credentials from leaking through another's process boundary.
- **Multi-instance turm.** Currently socket per PID. Each turm spawns its own copies of all service plugins? Or shared daemons? For v1, per-instance. Revisit if plugin spawn cost matters.
- **Chained triggers / composite actions.** The current trigger engine (Phase 8) maps one event to one action with `{event.*}/{context.*}` interpolation. Common workflows want a sequence — e.g. "on `calendar.event_imminent`, call `kb.ensure` → use its returned path to call `webview.open`". Three possible designs (decision deferred to Phase 9 wrap-up): (a) chained triggers — the result of one action is published as a synthetic event another trigger consumes; (b) composite actions — a built-in helper like `workflow.open_kb_doc` does both calls; (c) trigger templates with multi-step `actions = [...]`. (a) is most extensible, (c) most readable. **Until this decision is made, the Phase 10 meeting-prep slice ships strictly with `kb.ensure` only — no inline workaround.** The user opens the created file themselves; auto-open lands as a follow-up trigger config update once the chain mechanism is implemented.

## Sources (research informing this plan)

- [VSCode: Our Approach to Extensibility](https://vscode-docs.readthedocs.io/en/stable/extensions/our-approach/)
- [VSCode: Activation Events](https://code.visualstudio.com/api/references/activation-events)
- [VSCode: Language Server Extension Guide](https://code.visualstudio.com/api/language-extensions/language-server-extension-guide)
- [LSP Specification 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/)
- [Neovim RPC and Channels](https://neovim.io/doc/user/channel.html)
- [Zed: Life of a Zed Extension](https://zed.dev/blog/zed-decoded-extensions)
- [Zed: Extensions System](https://deepwiki.com/zed-industries/zed/13-extensions-system)
- [Supervisord: Subprocesses](https://supervisord.org/subprocess.html)

## Cross-references inside the repo

- Overall workflow-runtime vision: [workflow-runtime.md](./workflow-runtime.md)
- Existing plugin system (UI panels + shell commands): [plugins.md](./plugins.md)
- Numbered architectural decisions: [decisions.md](./decisions.md)
- Phase checklist: [roadmap.md](./roadmap.md)
- turm-core module reference: [core-lib.md](./core-lib.md)
