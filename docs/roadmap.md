# Roadmap

## Vision

turm = personal workflow runtime surfaced through a terminal.
Terminal remains the primary surface, but `turm-core` is a workflow engine — services (calendar, messengers, docs, knowledge base), triggers, and AI agents all plug into a shared Event Bus, Action Registry, and Context Service (see [workflow-runtime.md](./workflow-runtime.md)).
Goal: every daily work task — coding, checking meetings, processing notifications, searching personal notes — is driven from turm without context-switching.

## Implementation Phases

### Phase 1: MVP Terminal ✅

- [x] Cargo workspace with turm-core, turm-linux, turm-cli
- [x] GTK4 + VTE4 native terminal
- [x] Shell spawn (from config)
- [x] Font configuration
- [x] Dynamic font scaling (Ctrl+=/−/0)
- [x] Catppuccin Mocha theme
- [x] TOML config loading
- [x] `--init-config` and `--config-path` CLI flags
- [x] Dark theme forced
- [x] Desktop entry + install script

### Phase 2: Background Images ✅

- [x] GPU-rendered background via `gtk4::Picture` + `gdk::Texture`
- [x] GtkOverlay compositing (picture → tint → terminal)
- [x] VTE transparent background (`set_clear_background(false)`)
- [x] Tint overlay via CSS (no Cairo)
- [x] D-Bus interface for dynamic control (SetBackground, ClearBackground, SetTint)
- [x] Shell script for random rotation daemon (`turm-random-bg.sh`)
- [x] Config hot-reload (file watcher)

### Phase 3: Tabs + Panel System ✅

Terminal tabs first, then generalize to support different panel types.

- [x] **Tab model**: `Panel` trait with `TerminalPanel` as first impl
- [x] **TabBar**: new / close / switch / reorder (drag)
- [x] **Split panes**: horizontal / vertical split within a tab
- [x] **Pane resize**: drag dividers
- [x] **Focus tracking**: active pane focus via `EventControllerFocus`
- [x] **Keyboard shortcuts**: Ctrl+Shift+T/W/Tab, Ctrl+Shift+E/O (split), Ctrl+Shift+[1-9], Ctrl+Shift+C/V (copy/paste), Ctrl+Shift+B (tab bar toggle), Ctrl+Shift+F (search)
- [x] **Configurable tab position**: top, bottom, left, right (`[tabs] position` in config)
- [x] **In-terminal search**: Ctrl+Shift+F search bar with VTE regex (next/prev/case toggle)
- [ ] **Panel type registry**: extensible system for registering new panel types

### Phase 4: Control API

Single programmable interface for both human CLI and AI agents.

- [x] CLI tool (turmctl) with clap subcommands
- [x] cmux V2 JSON protocol types
- [x] Unix socket client
- [x] **Socket server** in turm-linux (Unix socket, per-PID path)
- [x] **Command dispatch**: system.ping, background.set/clear/set_tint/next/toggle, tab.new/close/list, split.horizontal/vertical
- [x] **Env var injection**: TURM_SOCKET per terminal session
- [x] **Event stream**: subscribe to terminal output, focus changes, panel lifecycle via `event.subscribe`
- [x] **Query API**: `session.list`, `session.info` (panel details + cursor/dimensions), `tab.info` (extended tab info)

### Phase 5: WebView Panel ✅

Embed browser as a panel type alongside terminals.

- [x] **WebKitGTK panel**: `WebViewPanel` as Panel impl via `webkit6` crate (GTK4-native)
- [x] **PanelVariant enum**: refactored split tree, tab manager, and socket dispatch from `Rc<TerminalPanel>` to `Rc<PanelVariant>`
- [x] **Socket API**: `webview.open`, `webview.navigate`, `webview.back/forward/reload`, `webview.execute_js`, `webview.get_content`
- [x] **Async dispatch**: `webview.execute_js` and `webview.get_content` reply asynchronously from WebKit callbacks
- [x] **Event stream**: `webview.loaded`, `webview.title_changed`, `webview.navigated` events
- [x] **CLI commands**: `turmctl webview open/navigate/back/forward/reload/exec-js/get-content`
- [x] **Side-by-side workflow**: terminal + webview split or tabbed
- [x] **AI agent DOM inspection**: screenshot, query/query-all, get-styles, click, fill, scroll, page-info
- [x] **Pre-built JS snippets**: `webview::js` module with structured JSON-returning DOM helpers
- [x] **Configurable vertical tab width**: `[tabs] width` in config with dynamic CSS hot-reload
- [x] **URL bar / navigation** within panel (UI)
- [x] **DevTools toggle** (UI + socket command `webview.devtools`)
- [ ] **JS ↔ turm bridge**: page scripts can call turm API

### Phase 6: AI Agent Integration

Make turm a first-class environment for AI coding agents.

- [x] **Screen reading API**: `terminal.read` (full screen or row/col range), `terminal.state` (cursor, dimensions, CWD, title)
- [x] **Command execution API**: `terminal.exec` (command + newline), `terminal.feed` (raw PTY input)
- [x] **CWD tracking**: `terminal.cwd_changed` event via OSC 7
- [x] **CLI commands**: `turmctl terminal read/state/exec/feed`
- [x] **Notification channel**: OSC 9/777 notifications via `terminal.notification` event
- [x] **Shell integration signals**: `terminal.shell_precmd` / `terminal.shell_preexec` events
- [x] **Approval workflow**: `agent.approve` shows modal dialog, returns user's choice
- [x] **Context sharing**: `terminal.history` (scrollback), `terminal.context` (state + screen + history)

### Phase: Deployment & Distribution ✅

- [x] `--version` flag for both binaries
- [x] GitHub Actions CI (fmt, clippy, test, build)
- [x] GitHub Actions Release (build + tarball + GitHub Release on tag push)
- [x] Curl-pipeable install script (`install.sh`)
- [x] Self-update via `turmctl update check/apply`
- [x] cargo-release + git-cliff config for versioning and changelogs

### Phase 5.5: Tab Bar Controls

Collapsible tab bar and renamable tabs.

- [x] **Tab bar toggle**: Ctrl+Shift+B toggles between collapsed (icon-only) and expanded mode
- [x] **Collapsed mode**: Icon-only tabs + toggle button (default state)
- [x] **Auto-expand**: Tab bar expands on 1→2 tab transition; user toggle overrides auto behavior
- [x] **Action buttons**: Toggle + add (terminal/browser popover) in tab bar
- [x] **Socket API**: `tabs.toggle_bar`, `tab.rename`
- [x] **CLI commands**: `turmctl tab toggle-bar`, `turmctl tab rename --id <id> <title>`
- [x] **Double-click rename**: Double-click tab label for inline rename
- [x] **Custom titles**: Renamed tabs suppress auto-title updates from terminal/webview
- [x] **Event stream**: `tab.renamed` event

### Phase 7: Polish + Ecosystem

- [x] Theme system (10 built-in themes, hot-reload, all UI components themed)
- [ ] Clipboard integration (OSC 52)
- [ ] URL detection + click-to-open
- [ ] Session persistence / restore
- [x] Plugin system (HTML/JS panels + shell commands via plugin.toml manifest)
- [x] Status bar (Waybar-style bar with plugin modules, left/center/right sections)
- [~] macOS native app (Swift/AppKit) — Phase 1 MVP complete (see below)

### macOS App

Goal: full Linux feature parity. Phase 1 MVP complete; porting remaining Linux features.

**Phase 1 — MVP ✅**
- [x] SwiftTerm integration (LocalProcessTerminalView) via Swift Package Manager
- [x] Shell spawn via PTY (SwiftTerm handles PTY internally)
- [x] TOML config loading (shell, font_family, font_size, theme name)
- [x] All 10 built-in themes (color palette + ANSI colors injected into SwiftTerm)
- [x] Window title update via OSC (setTerminalTitle delegate)
- [x] Font scale zoom (Cmd+= / Cmd+- / Cmd+0 via View menu)
- [x] TURM_SOCKET env var injected into shell
- [x] Process exit → pane/tab/window close (SwiftTerm bug fix via separate DispatchSource)
- [x] Tab bar (custom TabBarView with add/close/switch)
- [x] Split panes (Cmd+D horizontal, Cmd+Shift+D vertical, Cmd+W close pane)
- [x] Unix socket server (IPC with turmctl — same protocol as Linux)
- [x] Full socket API: terminal.exec/feed/state/read/history/context, tab.new/close/switch/list/info/rename, split.horizontal/vertical, session.list/info
- [x] In-terminal search (Cmd+F / Cmd+G / Cmd+Shift+G — SwiftTerm built-in find bar with case/regex/whole-word)
- [x] Background images (NSImageView + tint overlay per pane, config `[background] path/tint`, socket `background.set/clear/set_tint`)

**Phase 2 — WebView Panel ✅**
- [x] WKWebView panel type (WebViewController, macOS native WebKit)
- [x] TurmPanel protocol — TerminalViewController + WebViewController share common interface
- [x] SplitNode uses `any TurmPanel` — terminal and webview can be split side-by-side
- [x] Socket API: webview.open/navigate/back/forward/reload/execute_js/get_content/devtools/state
- [x] Tab title updates via WKNavigationDelegate (posts terminalTitleChanged notification)
- [x] Tab bar supports mixed terminal + webview tabs
- [x] SocketServer upgraded to async handler — execute_js/get_content return after WKWebView callback
- [x] Cmd+Shift+T opens new web tab from menu

**Phase 3 — AI Agent & Shell Integration ✅**
- [x] Event stream (`event.subscribe` — long-lived socket connection streams newline-delimited JSON events)
- [x] EventBus — broadcast hub with per-subscriber buffered channels (EventChannel)
- [x] CWD tracking (OSC 7 → `terminal.cwd_changed` via `hostCurrentDirectoryUpdate` delegate; uses `URL.path` to strip `file://hostname` prefix)
- [x] Shell integration signals (`terminal.shell_precmd` / `terminal.shell_preexec` via socket commands)
- [x] `panel.focused` — emitted on click-to-focus in PaneManager
- [x] `panel.exited` — emitted on process termination
- [x] `panel.title_changed` — emitted on title update (terminal + webview)
- [x] `tab.opened` / `tab.closed` — emitted in TabViewController
- [x] `webview.loaded` / `webview.title_changed` / `webview.navigated` — WKNavigationDelegate
- [x] `agent.approve` — NSAlert sheet modal, returns chosen action async
- [ ] `terminal.output` — PTY output interception not possible via SwiftTerm public API (`feed(byteArray:)` is non-overridable extension method)

**Phase 4 — Tab Bar & UX Polish**
- [x] Tab bar toggle (collapsed icon-only mode, Cmd+Shift+B, socket `tabs.toggle_bar`, event `tab.bar_toggled`)
- [x] Double-click tab rename (inline editing)
- [ ] Pane focus navigation keyboard shortcuts (next/prev pane)
- [ ] Background random rotation (socket `background.next`, config `[background] directory`)
- [x] Theme hot-reload (config file watcher — `ConfigWatcher`, kqueue DispatchSource, applies theme/font/background)

**Phase 5 — Distribution & Ecosystem**
- [ ] Session persistence / restore
- [ ] Clipboard integration (OSC 52)
- [ ] URL detection + click-to-open
- [ ] Plugin system (HTML/JS panels + shell commands via plugin.toml)
- [ ] Status bar (Waybar-style modules)

### Phase 8: Workflow Runtime (in progress)

Reframe `turm-core` as a personal workflow runtime. See [workflow-runtime.md](./workflow-runtime.md) for design.

- [x] **Event Bus** in turm-core (pub/sub with glob pattern matching, bounded mpsc delivery, drop-newest on subscriber overflow, 9 unit tests)
- [ ] **Socket event stream refactor** — existing `event.subscribe` becomes a bus projection
- [~] **Action Registry** in turm-core (name → handler map; sync v1 with 11 unit tests incl. nested-register / nested-invoke deadlock regressions — JSON Schema + async variants deferred until first service provider needs them)
- [ ] **Socket dispatcher migration** — new commands go through registry, existing match kept
- [x] **Context Service** v1 wired in turm-linux (pumped from GTK timer, exposed via `context.snapshot` action + `turmctl context`. `active_panel` + `active_cwd`, per-panel cwd cache, 10 unit tests. Other fields land with their providers.)
- [~] **Trigger engine** wired in turm-linux. `TurmConfig.triggers: Vec<Trigger>` loaded at startup; pumped from GTK timer with **scoped** `subscribe_unbounded(pattern)` per unique trigger `event_kind`, deduplicated through `covering_patterns` so overlapping declarations (e.g. `*` plus `panel.focused`) collapse to a single broader receiver — no double-dispatch on shared events, no OOM hazard from unrelated kinds. Per-event `Context` snapshot for `{context.*}` interpolation. Built-in `system.log` action available as a trigger sink. Config hot-reload runs `engine.set_triggers()` (atomic swap) and `subs.reconcile()` (preserves still-needed receivers' pending events, drops removed patterns, adds new). E2E verified: trigger fires on `terminal.cwd_changed` with `{event.cwd}` interpolation; 5000-line `terminal.output` flood causes zero spurious dispatches (unmatched kinds never enter the trigger queues). **Reach:** since the `TriggerSink` trait + `LiveTriggerSink` landed, every command handled by `socket::dispatch` is trigger-reachable (`event.subscribe` is special-cased earlier in `start_server` and is intentionally not a trigger sink). Registry actions get full sync error semantics; legacy match-arm fallthrough surfaces `ok=false` replies asynchronously via a consumer thread (stderr). See the next entry for details.
- [x] **Trigger reach expansion** via `TriggerSink` trait + `LiveTriggerSink` (turm-linux). `TriggerEngine` now invokes through `Arc<dyn TriggerSink>`. Default impl on `ActionRegistry` (registry-only); `LiveTriggerSink` tries registry first, falls through to `socket::dispatch` for legacy match-arm commands. Triggers can now fire any command handled by `socket::dispatch` (`tab.*`, `terminal.exec`, `webview.*`, `plugin.*`, …; `event.subscribe` is intentionally not reachable since it's special-cased in `start_server` and not a meaningful trigger sink). Fallthrough surfaces failures asynchronously: `LiveTriggerSink::new` spawns a consumer thread that drains a shared reply channel and prints `[turm] trigger fallthrough id=... failed: <code>: <msg>` to stderr for any `ok=false` response (typos, unknown methods, runtime errors). Per-event `fired` count over-counts on fallthrough (counts queueing as success), but misconfiguration is visible. Registry actions retain full sync error semantics. E2E verified: legacy `terminal.exec` trigger writes a marker file on `cd /tmp`; misspelled `terminal.execc` trigger is logged via the consumer thread.
- [ ] Command palette (Ctrl+Shift+P) over Action Registry — orthogonal to plugin pivot, stays in core

> **Architectural pivot (after Phase 8 Trigger reach landed):** all external integrations originally listed under Phase 8 — Google Calendar provider, Slack/Discord gateway, Notion document provider, Knowledge base layer — moved to **service plugins** in Phase 9–13. They are no longer turm-core modules. See [service-plugins.md](./service-plugins.md) for end-state vision, plugin-first decisions, and the detailed plan.

### Phase 9: Service Plugin Protocol & Host

Plugin-first foundation. See [service-plugins.md](./service-plugins.md) for full rationale.

- [x] Manifest extension: `[[services]]` (name, exec, args, activation, restart, **provides**, **subscribes**) parsed in `turm-core::plugin`. `Activation` (OnStartup / OnAction / OnEvent) and `RestartPolicy` (OnCrash / Always / Never) parsed from string form with explicit error messages. 10 unit tests cover defaults, glob extraction, and rejection of malformed inputs.
- [x] Service supervisor in turm-linux (`service_supervisor.rs`) — spawns child via `Command` with stdio piped, owns 3 threads per service (writer, reader, stderr-tail) plus a wait thread that observes exits. Restart policy with exponential backoff (1s → 2s → 4s … capped at 60s). State machine `Stopped → Starting → Running → (Stopped on exit)` with `Failed` for hard rejections. 7 unit tests (`provide_no_conflict_returns_empty_conflicts`, `provide_conflict_resolves_lexically`, `provide_three_way_conflict_collects_all_losers`, `parse_inbound_recognizes_response/request/notification`, `parse_inbound_treats_empty_id_as_notification`, `backoff_grows_then_caps`).
- [x] Initialization handshake — turm→service `initialize` with `{turm_version, protocol_version}`; service replies with `{service_version, provides, subscribes}`. Asymmetric validation applied identically to both fields: every runtime entry must appear in the manifest (superset → drop with warn, subset → degraded mode OK and ENFORCED at dispatch — manifest-approved actions the runtime omitted return `service_degraded` from `invoke_remote`). 5s default init timeout; on miss, supervisor closes outgoing channel AND issues a best-effort `SIGKILL` to the recorded child PID so a plugin that ignores its stdin can't accumulate as an orphaned process across restart attempts.
- [x] Bidirectional RPC over newline-JSON via stdio. turm→service: `initialize`, `initialized`, `action.invoke`, `event.dispatch`. service→turm: `action.invoke` (recursive — runs on a worker thread to avoid reader-thread deadlock), `event.publish`, `log`. Notifications use empty `id`; requests use a non-empty id.
- [x] Lazy activation: `onStartup` (eager-spawned at supervisor init), `onAction:<glob>` (registered handler triggers spawn on first invocation, buffers up to 64 invocations during `Starting`, flushes after init), `onEvent:<glob>` (per-rule subscriber thread on the bus spawns the service on first matching event AND on subsequent matches whenever state is `Stopped`/`Failed`, so init failures don't permanently inert an event-activated service). **Known caveat:** the activation event itself is NOT delivered as `event.dispatch` — that channel is driven exclusively by `subscribes` per the protocol. Authors who need both activation and delivery should declare the same glob in both lists. A future iteration can pre-subscribe `subscribes` patterns at supervisor::new (instead of post-init) so events that arrive during init are buffered and delivered after the handshake.
- [x] Deterministic conflict resolution — `resolve_provides()` walks all enabled plugin manifests in lexical `[plugin].name` order BEFORE any spawn, builds the global action-ownership table, and emits a `ProvideConflict` report. Loser plugins keep all non-conflicting `provides`; conflicting entries are dropped with `[turm] service conflict: …` warnings.
- [x] Mock `turm-plugin-echo` (Rust workspace member): `onStartup`, registers `echo.ping` (round-trips params), publishes `system.heartbeat` every `TURM_ECHO_HEARTBEAT_SECS` seconds (default 30). Manifest at `examples/plugins/echo/plugin.toml`. E2E verified: socket → registry → service → response (round-trip), `system.heartbeat` events visible via `event.subscribe`, supervisor auto-restarts after `pkill -KILL` of the child.
- [x] `turmctl call <method> [--params <json>]` — generic action dispatch from CLI, used as the service-plugin escape hatch and for any registry action without a dedicated subcommand.
- [ ] **Known limitation, deferred:** `invoke_remote` blocks the calling thread for up to the action timeout (default 30s). Socket commands run on the GTK timer thread, so a slow service action stalls the UI for the duration. Acceptable while service actions are fast (echo + KB grep return in <100ms); becomes a real problem once the LLM plugin lands. Fix path: change `ActionRegistry` handler signature to `Fn(Value) -> impl Future<…>` (or a `(reply: Sender<…>)`-style continuation) so dispatch can hand off to a worker. Tracked here so it isn't forgotten when LLM ships.
- [x] `ServiceSupervisor::shutdown_all()` — wired from `window.connect_destroy`. Sends the documented `shutdown` notification to every Running service, drops the writer-channel sender so child stdin closes on EOF, and SIGKILLs any pid still recorded after a 200ms grace window. Idempotent.
- [ ] **Known limitation, deferred:** abrupt parent termination (SIGKILL, segfault, panic before destroy callbacks run) leaves plugin processes orphaned to init. The graceful `connect_destroy → shutdown_all` path covers normal window close. Fix path: register a SIGTERM/SIGINT handler that calls `shutdown_all`, plus optionally `prctl(PR_SET_PDEATHSIG)` in pre_exec (note the latter is thread-scoped on older kernels and needs careful sequencing — needs verification on the deployment kernel before enabling). Tracked here so the orphan-on-crash case isn't forgotten when reliability becomes critical.
- [ ] **Known limitation, deferred:** `subscribes` forwarder threads are spawned per successful init and have no teardown path on crash/restart. A service that crashes and restarts repeatedly accumulates one sleeping forwarder thread + one bus subscription per restart per pattern. For low-traffic patterns those threads sit blocked on `rx.recv()` indefinitely. The bus does GC disconnected receivers lazily when the queue fills, but the forwarder thread itself keeps the receiver alive. Fix path: track per-instance forwarder JoinHandles (or use a Drop sentinel that signals exit), and tear them down from `handle_exit` before the next start. Acceptable for v1 because crash-loops are also bounded by the exponential backoff + the `restart=never` policy.
- [x] [`docs/kb-protocol.md`](./kb-protocol.md) — request/response shapes for `kb.search`/`kb.read`/`kb.append`/`kb.ensure`. Designed so backend swap (grep → FTS5 → embedding → Notion → Obsidian) doesn't break callers. Every documented field is always present in compliant output (`T|null` types use `null`, not omission); forward-compat fields use omission. Hits carry `id` (stable round-trip handle), `score` (relative ordering only), `snippet` (display text), and `match_kind` (always present, value `"filename"`/`"fulltext"`/`"semantic"` plus future additions). Folder conventions: `meetings/` / `people/` / `threads/` / `notes/` are searchable; `.raw/` is a protocol-level search exclusion (still writable by id). `kb.append` requires single-syscall `O_APPEND` writes; `kb.ensure` requires temp-file + `renameat2(RENAME_NOREPLACE)` atomic rename for both exactly-one-creator and no-torn-read. Error codes are split between plugin-origin (`not_found`/`forbidden`/`invalid_id`/`invalid_params`/`not_implemented`/`io_error`) and supervisor-origin (`service_degraded`/`service_unavailable`).
- [ ] First-party `turm-plugin-kb` (Rust): grep + filename over `~/docs`, `onAction:kb.*` lazy

### Phase 10: Calendar (first vertical PoC)

- [ ] `turm-plugin-calendar` (Rust): Google Calendar OAuth + polling, publishes `calendar.event_imminent`
- [ ] Meeting-prep TOML trigger: `kb.ensure` only (creates/refreshes `~/docs/meetings/<event_id>.md`). Panel auto-open deferred — depends on the chained-trigger / composite-action decision tracked in service-plugins.md Open questions, expected after Phase 9 wrap-up.

### Phase 11: Messenger ingestion

- [ ] `turm-plugin-slack`: OAuth + WebSocket gateway, publishes `slack.mention`/`slack.dm`/etc, raw-archive to `~/docs/.raw/slack/...`
- [ ] Derived markdown ingestion (depends on Phase 12 LLM plugin)

### Phase 12: LLM plugin (when desired)

- [ ] `turm-plugin-llm`: registers `llm.complete`/`llm.summarize`/`llm.draft_reply`
- [ ] Per-user secrets store, cost tracking via `llm.usage`

### Phase 13: KB indexing upgrade (when grep is slow)

- [ ] SQLite FTS5 sidecar index, fs-watcher rebuild — KB plugin internal change only, protocol unchanged

## Pending Cleanup

- [x] ~~Remove turm-core/pty.rs and state.rs (VTE handles PTY on Linux, SwiftTerm on macOS)~~
- [x] ~~Unify D-Bus and Socket API — D-Bus removed, socket is the sole IPC~~

## Reference Projects

- `~/dev/cmux/` — Socket protocol, CLI structure, window/workspace model
- `~/kitty-random-bg.sh` — Background rotation logic (ported to turm-random-bg.sh)
- Zellij — Panel/plugin architecture reference
- Wezterm — Lua scripting, multiplexer model
