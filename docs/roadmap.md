# Roadmap

## Vision

turm = a single work tool that fuses **terminal (Vim) + Calendar + Slack + Jira + Todo + git workspaces + Claude Code spawn** into one orchestratable surface. The terminal stays the primary editing surface; everything else plugs into a shared workflow runtime (Event Bus, Action Registry, Context Service, Trigger Engine — see [workflow-runtime.md](./workflow-runtime.md)).

Concretely the user wants flows like:

1. **Calendar event imminent** → KB note auto-created with title / start / location interpolated into the frontmatter; the calendar event payload also exposes `event.attendees`, `event.description`, `event.recurring_id`, etc. for users who want a richer template. *(Building blocks shipped — Phase 9.3 KB plugin + Phase 10.1 Calendar plugin; example trigger config at `examples/plugins/calendar/triggers.example.toml`.)*
2. **Slack mention/DM** → archived to KB with full fidelity, optionally summarized via LLM. *(Archive shipped — Phase 11.2; LLM-summarize step blocked on chained-trigger work — Phase 14.)*
3. **Todo with `start` action** → optional Slack message asking for context → wait for reply containing Jira ticket # → **git worktree** created (`~/dev/<workspace>-worktrees/<jira-id>/`) → tmux session opened in the worktree path (attach-or-create so it persists across turm restarts) → Claude Code spawned **inside the tmux session with a pre-filled prompt** built from the Todo body + Jira summary + linked KB notes. Worktree (not plain branch) so the original repo stays clean and the user can have parallel branches concurrently in different turm tabs. *(Not yet expressible — needs Phases 15.1, 17, 18, 16, 14.1, 15.2, 14.2 in that order — see "Recommended execution order" below.)*
4. **Jira ticket assigned** → Todo auto-created, frontmatter `linked_jira` populated with the ticket key. *(Not yet expressible — needs Phases 15 + 16. Cross-linking back to related Slack threads is a future enhancement that depends on Phase 11.3's derived ingestion landing first.)*

Flows 1–2 are end-to-end working today (with the LLM step in 2 deferred). Flows 3–4 require composite/chained workflows + Todo + Jira + git plugins, all currently missing — see Phases 14–18.

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
- [x] **Resolved (Phase 9.4):** `ActionRegistry` now distinguishes sync from blocking handlers. New API: `register_blocking` (same handler signature as `register`, just flags the entry); `try_dispatch(self: &Arc<Self>, name, params, on_done) -> bool` that runs sync handlers inline (callback fires synchronously on caller thread) and spawns a worker thread for blocking handlers (callback fires from worker). Existing `invoke` / `try_invoke` retained for tests + explicit-block contexts; new `is_blocking()` for diagnostic branches. `service_supervisor` registers every plugin action via `register_blocking` because `invoke_remote` parks waiting for the stdio reply. **Caller migration:** `socket::dispatch` uses `try_dispatch` uniformly — its reply path is already channel-based so sync vs blocking is invisible to the CLI client. `LiveTriggerSink::dispatch_action` deliberately branches on `is_blocking()` to preserve the pre-Phase-9.4 trigger contract: sync handlers keep their synchronous error path so `TriggerEngine`'s `log::warn` and `fired` count remain accurate; blocking handlers go through `try_dispatch` and return `Ok({queued})` because the engine can't await a worker without re-introducing the GTK stall. 11 new unit tests across both crates: caller-thread inline for sync, worker-thread observation for blocking with `<40ms` return assertion, error propagation through both, `is_blocking` flag accuracy, register-vs-register_blocking overwrite, plus four LiveTriggerSink tests covering sync-Ok / sync-Err / blocking-fast-return / unknown-action-fallthrough.
- [ ] **Known limitation, Phase 9.4 ships with O(n) threads under blocking burst:** `try_dispatch` spawns a dedicated OS thread per blocking call, and the supervisor's `dispatch_invocation` already spawns a waiter thread per invocation. Under a burst of N concurrent slow plugin actions that's `2N` sleeping threads. Acceptable for v1 traffic (a few triggers/min + occasional `turmctl call`); becomes a real cost when the LLM plugin lands and triggers fan out to many concurrent completions. Fix path: shared thread pool (rayon, or hand-rolled bounded-channel worker pool) for the registry, and reuse for the supervisor. Tracked here so it isn't forgotten.
- [x] `ServiceSupervisor::shutdown_all()` — wired from `window.connect_destroy`. Sends the documented `shutdown` notification to every Running service, drops the writer-channel sender so child stdin closes on EOF, and SIGKILLs any pid still recorded after a 200ms grace window. Idempotent.
- [~] **Phase 9.5 orphan-on-crash hardening — partially shipped, partially rolled back**: (a) Linux `prctl(PR_SET_PDEATHSIG, SIGTERM)` was originally added in the spawn `pre_exec` hook so the kernel would SIGTERM each plugin whenever its parent (turm) died for any reason. **Removed at Phase 18.x** because of a Linux quirk that's well-documented but easy to miss: the kernel signal fires when the **THREAD** that called `fork()` exits, not when the parent process exits. `spawn_service_async` runs each spawn on a fresh worker thread that returns as soon as `start_service` finishes — so every onStartup plugin received SIGTERM moments after init succeeded and crash-looped under restart=on-crash policy. (b) `glib::unix_signal_add_local` SIGTERM/SIGINT handlers in `app.rs` continue to fire `shutdown_all` on cooperative signals (Ctrl-C / `kill <pid>` SIGTERM) — that path is preserved. **Net effect today**: cooperative shutdown still reaps plugins cleanly; on turm SIGKILL / segfault, plugin children become init-reparented orphans until `shutdown_all` fires (a normal exit) or the user notices stray `turm-plugin-*` processes. Acceptable for single-user desktop. **To re-introduce crash-safe child reaping cleanly**: spawn every plugin from a dedicated long-lived "spawner" thread (so the fork-thread never exits while turm is alive), OR use `pidfd_open` + epoll instead of pdeathsig. Tracked here as an open follow-up on the supervisor.
- [x] **Resolved (Phase 9.5):** `subscribes` forwarder thread leak. Per-instance `forwarder_stop: Arc<AtomicBool>` + `forwarder_handles: Mutex<Vec<JoinHandle>>` tracking added to `ServiceHandle`. Forwarder threads now poll `rx.recv_timeout(200ms)` (new `EventReceiver::recv_timeout` API in turm-core) and check the stop flag between waits — so a fresh start has up to ~200ms shutdown latency per forwarder, not unbounded. `handle_exit` flips the stop flag, takes the JoinHandles vec, joins each before releasing the state lock. Pre-Phase-9.5 a crash-loop accumulated 1 thread + 1 bus subscription per restart per `subscribes` pattern; now the bookkeeping is bounded to (number of currently-Running instances × patterns). 3 new tests on `EventReceiver::recv_timeout` (event-when-available, timeout-when-idle, disconnected-when-bus-dropped).
- [x] [`docs/kb-protocol.md`](./kb-protocol.md) — request/response shapes for `kb.search`/`kb.read`/`kb.append`/`kb.ensure`. Designed so backend swap (grep → FTS5 → embedding → Notion → Obsidian) doesn't break callers. Every documented field is always present in compliant output (`T|null` types use `null`, not omission); forward-compat fields use omission. Hits carry `id` (stable round-trip handle), `score` (relative ordering only), `snippet` (display text), and `match_kind` (always present, value `"filename"`/`"fulltext"`/`"semantic"` plus future additions). Folder conventions: `meetings/` / `people/` / `threads/` / `notes/` are searchable; `.raw/` is a protocol-level search exclusion (still writable by id). `kb.append` requires single-syscall `O_APPEND` writes; `kb.ensure` requires temp-file + `renameat2(RENAME_NOREPLACE)` atomic rename for both exactly-one-creator and no-torn-read. Error codes are split between plugin-origin (`not_found`/`forbidden`/`invalid_id`/`invalid_params`/`not_implemented`/`io_error`) and supervisor-origin (`service_degraded`/`service_unavailable`).
- [x] **Protocol clarification:** Phase 9.2's kb-protocol.md folder note originally claimed embedded nul on `kb.search.folder` returns `forbidden`, while the shared error table treated nul as a shape problem (`invalid_id`-class). Phase 9.3 implementation surfaced the inconsistency; resolved by splitting `folder` errors along the same shape-vs-trust-boundary axis as the rest of the protocol — empty/nul → `invalid_params` (shape), `..` / absolute → `forbidden` (trust boundary). Doc + impl now agree.
- [x] First-party `turm-plugin-kb` (Rust workspace member, Linux-only via `compile_error!` gate): grep + filename over `~/docs` (override via `TURM_KB_ROOT`, force-canonicalized to absolute on construction), `onAction:kb.*` lazy. All 4 actions (`kb.search`/`kb.read`/`kb.append`/`kb.ensure`) implement the protocol's atomicity contract: `kb.ensure` uses temp-file + `renameat2(RENAME_NOREPLACE)` (verified E2E with 5 concurrent calls — exactly one returns `created=true`); `kb.append` uses single-syscall `O_APPEND` write via `libc::write` (short-write surfaces as `io_error` rather than retrying); `kb.append` with `ensure=true` on a winner-create path embeds the payload in the temp file BEFORE the atomic rename so a concurrent reader never sees a created-but-empty file. Trust-boundary defense: `validate_id`/`validate_folder` reject `..`/absolute/nul; `resolve_within_root` canonicalizes the existing prefix and verifies it stays under `root_canonical` (catches symlinks placed before the call); `O_NOFOLLOW` on read/append opens catches a leaf-symlink swap inside the TOCTOU window. Filename score uses BASENAME only (querying `meetings` doesn't auto-promote files under `meetings/`). Search walks skip symlinks entirely (no follow during recursion or read), `.raw/` is excluded from search but writable by id, search-root read failures surface as `io_error` while per-file failures stay silent. Type-strict params: non-string `folder`/`default_template` and non-bool `ensure` return `invalid_params`. 16 unit tests; E2E verified against a sandbox `/tmp/turm-kb-test.*` dir.
- [ ] **Known limitation, deferred:** the symlink-escape defense closes the lexical traversal path, the canonicalize-time symlink path, and the leaf-swap TOCTOU (`O_NOFOLLOW`), but a swap of an INTERMEDIATE directory component for a symlink between the `resolve_within_root` check and the open/rename is theoretically still exploitable by a concurrent local actor. For a single-user personal KB that's an accepted risk; closing the residual window cleanly requires `openat2(..., RESOLVE_BENEATH, ...)` (Linux 5.6+, no libc binding yet — would need `libc::syscall` with `SYS_openat2`). Tracked here so an adversarial threat model would re-open this.

### Phase 10: Calendar (first vertical PoC)

**10.1 — Calendar plugin + UI panel ✅**

- [x] First-party `turm-plugin-calendar` (Rust workspace member, **Unix-only** via `compile_error!` gate — Linux + macOS, matching turm's full platform matrix; the `keyring` crate's mock fallback on platforms with no native backend would silently lose tokens otherwise): Google Calendar OAuth 2.0 device-code flow + read-only polling. Two run modes: `auth` subcommand for interactive OAuth (prints user_code + verification URL to stderr, polls until consent), and default RPC mode that speaks the service-plugin protocol over stdio. Plugin starts even without stored credentials so the user can run `turm-plugin-calendar auth` while turm is already up — the poller silently skips ticks until tokens appear.
- [x] **Token storage with secure-by-default fallback**: `keyring` crate (Linux Secret Service via D-Bus / macOS Keychain) is preferred. On keyring failure (no D-Bus session, headless server, etc.), falls back to plaintext at `$XDG_CONFIG_HOME/turm/calendar-token-<account>.json` (mode 0600 via `create_new` + atomic rename, with per-call atomic counter so concurrent saves can't collide on a pid-derived temp path) with stderr warning on every open. Set `TURM_CALENDAR_REQUIRE_SECURE_STORE=1` to refuse the plaintext fallback — token operations then return errors instead of writing plaintext, while RPC init still succeeds (plugin runs in a degraded "auth-required" mode rather than failing the supervisor handshake). `TURM_CALENDAR_ACCOUNT` is validated against a strict charset (ASCII alphanumeric + `_-.@`) so a malicious value cannot escape the config dir via path traversal.
- [x] **Polling daemon**: configurable `TURM_CALENDAR_LEAD_MINUTES` (comma-separated list, default `10`), `TURM_CALENDAR_POLL_SECS` (default 60), `TURM_CALENDAR_LOOKAHEAD_HOURS` (default 24). First tick runs immediately at startup (no leading sleep) so an event whose firing-time happens to fall within the first poll cycle isn't permanently missed; subsequent ticks sleep `poll_interval`. Each tick fetches `events.list` paginated through `nextPageToken` with `singleEvents=true&orderBy=startTime` (so recurring events arrive pre-expanded with per-instance start times) over the window `[now - max_lead, now + lookahead_hours]`. **Firing rule**: for each `(event, lead)` pair, fire iff `firing_time <= now < event.start` AND `now <= firing_time + max(2 × poll_interval, 120s)` (the catchup bound prevents stale fires — without it a 60-min lead on an event 9 min away would fire as a 51-min-late "catchup", which lost its meaning). The dedupe key `(event_id, lead_minutes)` enforces exactly-once across the consecutive ticks where `now` sits inside the firing band. Dedupe set is bounded by a 4096-entry cap to prevent unbounded growth over long sessions (worst case: re-fire a few boundary events after flush, accepted trade).
- [x] **Rich event payload** so triggers can branch on metadata: `id`, `recurring_id` (same value across all instances of a recurring series — exactly what triggers want for "fire only on this weekly meeting"), `title`, `start_time`/`end_time` (RFC 3339), `all_day`, `my_response_status` (`accepted`/`declined`/`tentative`/`needsAction`/`null`), `attendees[]`, `organizer`, `location`, `description`, `conference_url` (extracted from `conferenceData.entryPoints`, prefers video entry), `html_link` (direct calendar.google.com URL).
- [x] **Token refresh on 401**: gcal client wraps `TokenStore`, refreshes via `oauth::refresh` ~30s before server-reported expiry (clock-skew margin), retries the failing request once. A second 401 is fatal — caller must re-run `turm-plugin-calendar auth` (refresh_token revoked).
- [x] Provides `calendar.list_events` (validates optional `lookahead_hours` param: must be in `[1, 8760]`, otherwise `invalid_params`), `calendar.event_details` (lookup by id), `calendar.auth_status` (returns `{configured, authenticated, store_kind, account}` — `configured=false` whenever any required env validation failed at startup (missing `CLIENT_ID`/`SECRET` is the canonical case but a malformed `LEAD_MINUTES` or `POLL_SECS` falls into the same bucket — `Config::minimal()` is used uniformly for any parse error so the plugin never silently runs on partially-validated env). `authenticated=false` is independent and means env is OK but no tokens are stored. When `configured=false`, every Google-touching action returns `not_authenticated` upfront — without that early-return a stale token from a previous good run could make `list_events` succeed once and break confusingly on the next refresh.
- [x] **No new turm-host code** — the calendar UI uses the existing `webview.open` action. User opens Google Calendar via `turmctl call webview.open --params '{"url":"https://calendar.google.com","mode":"tab"}'` or any trigger that targets it. Calendar plugin is a pure event emitter; what to do with events (open KB note, post Slack, fire webhook, etc.) is entirely user-trigger config — no coupling between calendar and KB plugins.
- [x] Plugin manifest at `examples/plugins/calendar/plugin.toml`. `onStartup` activation (the polling daemon must be alive whenever turm is — `onAction:calendar.*` would only spawn on explicit query, too late for "10 minutes before meeting"). Example trigger config at `examples/plugins/calendar/triggers.example.toml` updated in Phase 10.2 to use the new `condition` clause directly (skip-if-declined, skip-1:1-from-common — see 10.2 below).

**10.2 — Per-event customization via `condition` clause ✅**

- [x] **`turm-core::condition` module**: hand-rolled minimal expression DSL (no external crate). Grammar: `or_expr / and_expr / not_expr / cmp_expr / atom`, recursive-descent parser, ~470 LOC including 26 unit tests. Operators: `== != < <= > >= && || !` plus parens. References: `event.X.Y` (navigates JSON payload by key, missing path → `null`) and `context.X` (top-level `active_panel` / `active_cwd` only — matches the existing `{context.X}` interpolation surface). Literals: quoted strings (with `\n \t \r \\ \"` escapes), integers, floats, `true` / `false` / `null`. Bare identifiers without a `.` are rejected at parse time so a typo like `recurring_id` instead of `event.recurring_id` errors loudly. **Numeric equality is type-tolerant**: `serde_json::Value::eq` returns false for `Number(PosInt(1)) == Number(Float(1.0))` which would surprise users writing `event.count == 1`; we override `==` / `!=` to normalize numeric Values to `f64` before comparing. Ordering ops require both sides numeric — string-vs-string `<` returns an evaluation error.
- [x] **`Trigger.condition: Option<String>`** added with `#[serde(default)]` so existing TOML configs are forward-compatible. `TriggerEngine` storage moved from `Vec<Trigger>` to internal `Vec<CompiledTrigger>` (trigger + cached AST). `set_triggers` parses each condition once at config-load / hot-reload time; a parse failure drops THAT trigger with a `log::warn` while the rest of the set still loads — a single typo can't disable the whole config. Per-event dispatch evaluates the cached AST: an `Err` from the evaluator (type mismatch on ordering, etc.) is logged and treated as "trigger does not match" — never fires the action on a misconfigured condition. 5 new TriggerEngine integration tests cover skip-when-condition-false, eval-error-skips-safely, parse-error-drops-only-the-bad-trigger, condition-with-context-ref, and TOML round-trip serialization.
- [x] **Example update**: `examples/plugins/calendar/triggers.example.toml` rewritten to use `condition` directly. Skip-if-declined: `event.my_response_status != "declined"`. Skip-the-weekly-1:1-from-common: `event.recurring_id != "REPLACE_..."`. Both rules now fire only on the events they should — no more multi-rule workaround callout. The 1:1 override has its own skip-when-declined guard.
- [x] **Resolution of original Phase 10 user requirements**: All four shapes of per-event customization (common across events, per-recurring differentiation, disable-common-for-specific-event, attendance-status conditional execution) are now expressible through the combination of existing positive `[triggers.when]` matching + the new `condition` clause. No further trigger-engine primitives required for the Phase 10 design space.

**Known limitations of 10.1, tracked for follow-up:**

- [ ] **All-day event timezone**: Google's `date`-form fields (no clock time) are defined in the calendar's own timezone, but the plugin interprets them as midnight in the *process's* local timezone, not the calendar's. For the canonical single-user-on-own-laptop case the two coincide and reminders fire correctly. For users who run a calendar on `Asia/Seoul` while travelling on a laptop set to `America/Los_Angeles`, all-day reminders shift by the offset. Closing the gap cleanly requires `chrono-tz` (~150KB extra binary) plus an extra `calendars.get('primary')` call to discover the calendar tz, which is not worth carrying for the rare-in-practice TZ-mismatch case. Accepted per user decision; flagged here so an adversarial setup re-opens it.
- [ ] **GTK-blocking poll calls**: The `calendar.list_events` action call from a trigger runs synchronously on the supervisor thread (Phase 9 known limitation `invoke_remote` blocks). With a slow Google API response (>200ms), the GTK timer thread stalls. Inherited from Phase 9; lands when the supervisor adopts an async handler signature.
- [ ] **OAuth client credentials must be supplied by the user** (`TURM_CALENDAR_CLIENT_ID` / `TURM_CALENDAR_CLIENT_SECRET`). Embedding shared OAuth credentials in OSS would let any forked turm impersonate "turm" in consent screens. The setup cost (one-time Google Cloud project) is the price of the trust boundary. Documented in `examples/plugins/calendar/plugin.toml`.
- [ ] **Single-account v1**: `TURM_CALENDAR_ACCOUNT` exists as a keyring-entry namespacing primitive but the plugin only ever reads from `primary` calendar of a single account at a time. Multi-account support would mean spawning N plugin instances with distinct `account_label` values, which the supervisor doesn't yet model.

### Phase 11: Messenger ingestion

**11.1 — Slack Socket Mode plugin (read-only events) ✅**

- [x] First-party `turm-plugin-slack` (Rust workspace member, Unix-only via `compile_error!` gate — same rationale as KB / calendar plugins). Connects to Slack via Socket Mode WebSocket — no public HTTPS endpoint required, perfect for desktop / single-user. Two run modes: `auth` subcommand validates the env tokens via `auth.test` and persists them to the configured store; default RPC mode runs the supervisor protocol over stdio plus a background Socket Mode loop.
- [x] **Two-token auth via env + keyring**. Required env: `TURM_SLACK_BOT_TOKEN` (`xoxb-...`, Bot User OAuth Token for HTTP API) + `TURM_SLACK_APP_TOKEN` (`xapp-...`, App-Level Token with `connections:write` for Socket Mode). One-time setup: create a Slack App at api.slack.com/apps, enable Socket Mode, install to workspace, copy both tokens — no OAuth redirect-flow needed for personal use. Tokens validated at `auth` time and persisted to keyring (Linux Secret Service / macOS Keychain) with plaintext fallback at `$XDG_CONFIG_HOME/turm/slack-tokens-<workspace>.json` (mode 0600, atomic-replace via per-call `AtomicU64` sequence so concurrent saves don't collide). `TURM_SLACK_REQUIRE_SECURE_STORE=1` refuses plaintext fallback. `TURM_SLACK_WORKSPACE` env var validated against the same charset as calendar's account label (alphanumeric + `_-.@`) to prevent path traversal.
- [x] **Socket Mode loop with auto-reconnect**. POST `apps.connections.open` returns a single-use WSS URL (Slack handles its own load balancing); plugin connects via `tungstenite` (sync rustls), reads frames, ACKs every `events_api` frame BEFORE invoking the user-side handler so Slack doesn't retry on slow consumers. Frame routing: `hello` (resets backoff), `events_api` (parse + ACK + emit turm event), `disconnect` (Slack rotated us; reconnect immediately with fresh bootstrap), `slash_commands`/`interactive` (ACK only — out of scope for v1). Any I/O error or generic WebSocket close (`ConnectionClosed`, `AlreadyClosed`, `Message::Close`) triggers exponential-backoff reconnect (1s → 60s capped) — only Slack's `disconnect` frame is graceful, so a peer-side error can't drive a tight reconnect against the API. Supervisor `shutdown` currently exits the process abruptly via `std::process::exit(0)` rather than draining the loop; tracked as a known limitation below.
- [x] **Aggressive event filtering** so triggers see signal only. `app_mention` → `slack.mention`. `message` events emit `slack.dm` only when `channel_type == "im"` AND no `subtype` (skips edits, deletions, joins, file_share notifications) AND no `bot_id` (skips automated messages and self-loops). All other event types dropped. Payload includes user, channel, text, ts, thread_ts, team_id, event_id — enough for triggers to do `kb.append`, `webhook.fire`, etc. without further API calls.
- [x] Provides `slack.auth_status` (returns `{configured, authenticated, store_kind, workspace, team_id, user_id}` — same shape as calendar.auth_status). Emits two event kinds: `slack.mention`, `slack.dm`. Plugin manifest at `examples/plugins/slack/plugin.toml` with `onStartup` activation (Socket Mode needs a long-lived connection — lazy activation would never connect because no `slack.*` actions drive demand). 14 unit tests covering env parsing, account-label charset, two-token store roundtrip with 0600 perms verification, concurrent-save isolation, broken-store reporting, event filtering (mention / DM / channel-message-skip / subtype-skip / bot-skip / unknown-type-skip / missing-fields), thread_ts capture, payload serialization.

**Known limitations of 11.1, tracked for follow-up:**

- [ ] **No graceful WebSocket close on shutdown**: the supervisor's `shutdown` notification handler calls `std::process::exit(0)` immediately. The Socket Mode loop is blocked in `ws.read()` while connected, so it never gets to send a `Close` frame to Slack — the server sees a TCP RST instead. Slack handles abrupt disconnects gracefully (the `disconnect` rotation path is exactly this case daily), but it's not formally polite. Fix path: set a read timeout on the WebSocket's underlying TCP stream (or use a write-half close from another thread) so the loop can exit cooperatively. Acceptable for v1 because plugin shutdown happens on turm exit, where the OS cleans up the socket regardless.
- [ ] **env-only path skips cross-token consistency check**: the `auth` subcommand validates `team_id` parity between bot and app tokens via `auth.test`, but RPC mode using direct env tokens (`TURM_SLACK_BOT_TOKEN` / `TURM_SLACK_APP_TOKEN` set without ever running `auth`) bypasses that check — a user pasting tokens from different workspaces would see Socket Mode connect successfully but to a different workspace than `auth_status` could attribute. Mitigation today: run `turm-plugin-slack auth` once with the env set; the consistency check fires there. Fix path: optionally re-run `auth.test` on the env pair at RPC startup (adds a network call to the spawn path).

**11.2 — Raw archive + write actions ✅**

- [x] **`slack.raw` event** — every `events_api` frame now produces a `slack.raw` turm event in addition to the optional filtered `slack.mention` / `slack.dm`. Payload shape: `{event_type, channel, ts, team_id, event_id, event_json}` where `event_json` is the verbatim Slack inner event (blocks, files, attachments, edits, joins — everything). The filter that controls mention/DM emission is unchanged; raw fires regardless so archive triggers see Slack's full diversity. `from_events_api_payload` API changed from `Option<SlackEvent>` to `Vec<SlackEvent>` to express the "one frame, two events" shape; socket loop iterates and emits each.
- [x] **`slack.post_message` action** — registered via `provides`. Params: `{channel, text, thread_ts?}`. Calls Slack's `chat.postMessage` with the resolved bot token (env or store, via the same `current_credentials` path the Socket Mode loop uses — write actions can never disagree with the live read source). Returns `{ts, channel}` on success. Surfaces Slack's error codes verbatim under `io_error` (`missing_scope` / `not_in_channel` / `channel_not_found` / `is_archived` / `msg_too_long` / `rate_limited`) so triggers can branch without parsing message strings. Refuses upfront if `fatal_error` is set or no credentials are available.
- [x] **Example raw-archive trigger** at `examples/plugins/slack/triggers.example.toml` — `slack.raw` → `kb.ensure` to `.raw/slack/{event.team_id}/{event.event_id}.json`. **Uses `kb.ensure`, not `kb.append`-with-ensure**: ensure is create-once-only (returns `created=false` on duplicate, content unchanged), which is the actual dedup primitive Slack-redelivery scenarios need. `kb.append+ensure=true` would atomically create + append, so a redelivered event would write a second copy. Path uses `event_id` (not `channel`+`ts`) because non-message events like `team_join` have null channel/ts which would collapse into a single file via interpolation; `event_id` is populated for every `events_api` envelope. Also illustrates a `slack.dm` → `slack.post_message` auto-reply pattern (commented; users opt in).
- [x] Plugin manifest at `examples/plugins/slack/plugin.toml` updated: `provides += [slack.post_message]`, setup notes call out the required `chat:write` Bot Token Scope. 27 unit tests (5 new — raw fidelity preservation, raw-only emission for filtered-out frames, raw on unknown event types, missing event field returns empty vec).

**11.3 — Full OAuth + reactions/updates + composable URL helpers ⏳ (deferred)**

- [ ] OAuth redirect flow as an alternative to env-paste setup — needs a localhost listener; defer until env+keyring proves insufficient.
- [ ] `slack.add_reaction` / `slack.update_message` / `slack.delete_message` write actions — convenience surface beyond `chat.postMessage`.
- [ ] Trigger interpolation DSL string ops — needed to transform a Slack ts into the `https://<workspace>.slack.com/archives/<ch>/p<ts-without-dot>` deep link URL inside `params`. Currently inexpressible without a wrapper action.

**11.3 — Derived markdown ingestion ⏳**

- [ ] Depends on Phase 12 LLM plugin. Uses the `.raw/slack/` archive as input, summarizes to `~/docs/threads/<topic>.md` for searchability via `kb.search`.

### Phase 12: LLM plugin

**12.1 — Anthropic provider + token-usage tracking ✅**

- [x] First-party `turm-plugin-llm` (Rust workspace member, Unix-only via `compile_error!` gate). Single provider for v1 (Anthropic Messages API) — multi-provider abstraction (OpenAI / local models) deferred to 12.2+ because the cost of the abstraction outweighs the value before a second provider is committed. Two run modes: `auth` validates `ANTHROPIC_API_KEY` with a 1-token messages call and persists `{api_key, validated_at}`; default RPC mode handles actions over stdio. Activation `onAction:llm.*` (lazy — no inbound stream to keep alive).
- [x] **Single primitive `llm.complete`** with `{prompt, system?, model?, max_tokens?, temperature?, source?}`. Higher-level `summarize` / `draft_reply` collapse into trigger config patterns rather than separate actions — different system prompts on top of the same primitive. Returns `{text, model, stop_reason, usage: {input_tokens, output_tokens}}`. Refuses upfront on `fatal_error` set or no credentials available. Validates `temperature` in `[0.0, 2.0]` and `max_tokens > 0` so trigger typos surface as `invalid_params` rather than a wasted Anthropic call.
- [x] **Single-source credential resolution** (env wins, store fallback) via `resolve_api_key` — same shape as slack/calendar. Env-key validation: must start with `sk-ant-`. `auth` subcommand exercises a real messages call so revoked / wrong-prefix keys fail at setup, not at first user-facing action.
- [x] **Anthropic client** (`src/anthropic.rs`) — `POST /v1/messages` with `x-api-key` + `anthropic-version: 2023-06-01`. Concatenates `content[i].text` blocks into a single string for the common case (skips `tool_use` etc.). Error handling mirrors slack's prefix-match contract: 401 → `auth_error: ...`, 429 → `rate_limited (Retry-After: <s>)`, 4xx other → `messages HTTP <code>: <body>`, top-level `type: "error"` payloads → `<error_type>: <message>`. Top-level `type: "error"` is also handled in 200 responses defensively.
- [x] **Append-only JSONL usage log** at `$XDG_DATA_HOME/turm/llm-usage-<account>.jsonl`. Each `llm.complete` writes one line `{ts, model, input_tokens, output_tokens, source?}` via single-syscall `libc::write` on `O_APPEND` fd — same atomicity contract as KB plugin's `kb.append`. Short-write surfaces as error (preserves no-interleave guarantee). Failure to append does NOT fail the action — user already paid for the tokens; stderr surfaces the issue.
- [x] **`llm.usage` aggregation** — read JSONL, optionally filter by `since` / `until` (RFC3339) and / or `by_model`. Returns `{calls, input_tokens, output_tokens, by_model: {<model>: {calls, input_tokens, output_tokens}}, parse_errors, since, until}`. Malformed lines (truncated writes, unrelated drops) counted as `parse_errors` and skipped — aggregation never fails on a partial file. No SQLite for v1; JSONL scan is fine for personal volume (a few hundred calls / month) and the swap to SQLite is internal-only since the action protocol is unchanged.
- [x] **No USD cost computation in v1**. Pricing changes too often for the plugin to maintain stale tables; users compute cost in their own dashboard layer using `llm.usage` output × current rates. Documented rationale in roadmap; revisit if multiple users ask for it.
- [x] **`llm.auth_status`** — `{configured, authenticated, credentials_source, fatal_error, store_kind, account, default_model, validated_at}`. Same shape as slack.auth_status; `validated_at` only meaningful when source is "store" (env-supplied keys haven't been validated by this plugin instance — could be revoked / wrong workspace).
- [x] **Supervisor `action_timeout` bumped 30s → 120s** to accommodate LLM completions. Documented as a Phase 12.1 trade-off — affects all plugins (none currently take more than ~100ms but the bump just changes how long a stuck plugin holds before surfacing `action_timeout`). Per-action timeout override is the right long-term fix; tracked here.
- [x] Plugin manifest at `examples/plugins/llm/plugin.toml` with `onAction:llm.*` lazy activation. Example file `examples/plugins/llm/triggers.example.toml` explicitly documents the result-handling gap with trigger-fired `llm.complete` (response discarded — fire-and-forget; only usage record is captured) and steers users at `turmctl call llm.complete` for visible-output completions. Phase 12.3 deferred-work fixes the chained-trigger mechanism that would let the result land somewhere useful. 29 unit tests covering env parsing, account-label charset, store roundtrip + concurrent-save isolation, anthropic response parsing (text concat, tool_use skip, error payloads, missing usage), credential resolution preferring env over store, auth_status short-circuit on fatal_error, complete param validation (missing prompt / zero max_tokens / out-of-range temperature / strict-type system+model / missing key), usage filtering (model, time range, parse-error counting, malformed-ts rejection without filter, account_resolved gate).

**12.2 — Multi-provider + streaming + per-action timeout ⏳ (deferred)**

- [ ] OpenAI / local-model providers behind a `provider` discriminator. Token counting + cost surfaces stay uniform.
- [ ] Streaming completions via SSE — needs a different action-protocol shape (incremental events instead of single response). Most useful for terminal-output progressive rendering.
- [ ] Per-action timeout override at the `register_blocking` site so `llm.complete` can extend to e.g. 5min for long-context tasks without affecting the rest of the supervisor.

**12.3 — Derived markdown ingestion ⏳**

- [ ] Trigger-driven distillation of the slack `.raw/slack/...` archive into searchable markdown under `~/docs/threads/`. Composes `kb.search` (find related threads) + `kb.read` + `llm.complete` (synthesize) + `kb.ensure` (write derived). Needs the chained-trigger / composite-action mechanism that's been deferred since Phase 9.

### Phase 13: KB indexing upgrade (when grep is slow)

- [ ] SQLite FTS5 sidecar index, fs-watcher rebuild — KB plugin internal change only, protocol unchanged

### Recommended execution order (Phases 14–18)

The phase numbers below reflect topical organization. Inter-system connectivity analysis (which phase actually unblocks which user flow) shows that **most new plugins are useful as single-action triggers without Phase 14** — `calendar → todo`, `jira.ticket_assigned → todo`, `git.list_worktrees`, `claude.start` all work today's trigger engine just fine. Only the multi-step flows (todo.start → worktree → claude, slack → llm summary → kb) need composite chaining.

So Phase 14 lands AFTER its real consumers exist:

1. **Phase 15.1 — Todo plugin basics (UI panel, `todo.create` / `list`)**. Daily-use surface, single-action triggers cover `calendar → todo` and `jira.ticket_assigned → todo` (when 16 ships). Highest user-visible impact for first-shippable phase. ETA: ~3 days.
2. **Phase 17 — Git worktree plugin** (lightweight, no external API). Single-action `git.worktree_add` already useful via `turmctl call`. ETA: ~1-2 days.
3. **Phase 18 — `claude.start` + tmux session integration**. Tiny — wraps `tab.new` + `tmux new-session -A` + `claude`. Manual invocation works without 14.
4. **Phase 16 — Jira plugin** (Slack pattern reused). Single-action `jira.ticket_assigned → todo.create` flow lands.
5. **Phase 14.1 — chained `<action>.completed` events**. By this point we have 4 concrete consumers (Todo / Git / Claude / Jira) so the chained-trigger primitive is informed by real composition needs, not abstract sketches.
6. **Phase 15.2 — Todo `start` workflow chain**. `todo.start_requested → git.worktree_add.completed → claude.start` end-to-end demo of Flow G from the Vision section.
7. **Phase 14.2 — async correlation primitive** (Slack ask → wait for reply with Jira id). Enables Flow H.
8. **Backfill** — Phase 11.3 derived slack markdown, Phase 12.3 LLM ingestion, Phase 10's deferred meeting-note auto-open all unblock here.
9. **Phase 19 — CLI ergonomics + context aggregation** (`turmctl todo create …`, `turmctl context`, `turmctl recent`). Parallel track to the chain work — every plugin's actions are already exposed via `turmctl plugin run`, so this is pure UX layering. Lands once the plugin set has stabilized so subcommand surfaces don't churn.

Throughout the sequence each step ships visible value; Phase 14 design lands with concrete consumers ready to dogfood it.

### Phase 14: Composite / chained workflow primitive

The biggest architectural item — but **scheduled mid-stream, not first** (see "Recommended execution order" above). Currently the trigger engine is `event → 1 action → done`. Multi-step flows from the Vision section ("Todo start → worktree → tmux → Claude") collapse against this. Same root cause as Phase 11.3's deferred derived markdown ingestion, Phase 12.1's discarded trigger-fired `llm.complete` results, and Phase 10's deferred meeting-note auto-open. Resolving this unblocks all derived workflows.

**14.1 — design decision** ([service-plugins.md](./service-plugins.md) Open Q reopened) — **shipped**:

Three sketches were on the table:
- **(a) Chained triggers via synthetic events**: every action's `try_dispatch` callback publishes a synthetic `<action>.completed` (with the result payload) and `<action>.failed` event onto the bus. Downstream triggers match on those. Most extensible — the bus already exists. Most uniform — every step is just another trigger.
- **(b) Composite actions**: a built-in `workflow.<name>` action whose handler runs a multi-step procedure inline. Easiest for hand-rolled wrappers like `workflow.start_todo` but doesn't help the user-config case.
- **(c) Multi-step trigger TOML**: extend `[[triggers]]` with `actions = [...]` instead of single `action`. Most readable for users but less flexible (no async wait, no branching).

**Decided: option (a) — chained triggers via synthetic events** (Phase 14.2 implementation below). (b) deferred to 14.2's `workflow.<name>` follow-up if the chained-TOML form gets unwieldy in practice; (c) discarded because it loses to (a) on async-correlation cases (Slack send → wait for reply).

**14.2 — implementation** (slice 1 — action result fan-out — **shipped**):

- [x] **`ActionRegistry::with_completion_bus(bus)`** — opt-in constructor that wires an `EventBus` into the registry. Every `try_dispatch` then publishes `<name>.completed` (Ok, payload = action's `Value`) or `<name>.failed` (Err, payload = `{code, message}`) AFTER the handler returns and BEFORE the caller's `Responder` fires. Source field `turm.action` distinguishes auto-publication from a plugin's own emitted events.
- [x] **Sync vs blocking semantics**: sync handlers publish from the caller thread (inline) before the `Responder` runs; blocking handlers publish from the worker thread (since the registry already runs them off-thread). Either way, `<action>.completed` lands on the bus before downstream chained triggers run on it.
- [x] **`register_silent` opt-out** for high-frequency built-ins (`system.ping`, `context.snapshot`) so their completion events don't dwarf real workflow events on the bus. Same handler shape as `register`; only the dispatch-time fan-out differs.
- [x] **Manual emit removed from `turm-plugin-git`**: previously the git plugin self-published `git.worktree_add.completed` from inside the action handler. With registry-level fan-out that would double-fire (once from the plugin, once from the registry). The plugin now just returns `Ok(payload)` and trusts the registry to stamp the completion event.
- [x] **6 new unit tests** in `turm-core::action_registry` covering Ok→completed, Err→failed, blocking-from-worker-thread publication, silent suppression, no-bus pre-Phase-14.1 compatibility, and ordering (publish before `Responder` for the sync path).

**14.2 slice 1 — async correlation primitive** — **shipped**:

- [x] **`Trigger.await` clause** with `{ event_kind, payload_match, timeout_seconds, on_timeout }` shape. `event_kind` is a glob (same matcher as `WhenSpec.event_kind`); `payload_match` values are interpolated against the originating event at register-time so per-incoming-event matching is a pure JSON-value equality check; `on_timeout = "abort"` (default) drops the pending entry, `"fire_with_default"` synthesizes the awaited event with `await = null` so downstream chains can run with degraded data.
- [x] **Two-phase `preflight_awaits` → `pending_awaits` state machine**. Trigger dispatch lands in preflight; `<action>.completed` event promotes. **FIFO scope is per action name — NOT per trigger AND not per invocation**: two triggers using the same action share a queue, AND even a single trigger fired multiple times concurrently may mis-correlate completion events to preflights if completions arrive out of dispatch order. In practice most workflows have ≤ 1 in-flight invocation per action; the limitation only matters when the same action fires repeatedly in fast succession with order-sensitive follow-up payloads. Closing fully needs per-invocation correlation tokens on `<X>.completed`/`.failed` (slice-2 follow-up). Workaround for predictable cross-trigger separation today: distinct action wrappers per trigger. `<action>.failed` drops preflight. Sweep cleans BOTH phases on deadline expiry; legacy match-arm actions that never publish `.completed` time out and drop or fire-with-default depending on policy. The deadline carries unchanged across promotion (one timeout window covers both phases). Why two-phase: `LiveTriggerSink` returns `Ok({queued: true})` for blocking and plugin actions before they actually succeed, so arming pending on the sink's `Ok` would queue awaits even when the action later fails async. `<action>.completed` is the only signal that's reliable for both sync registry and async-blocking paths. No persistence: both phases are volatile and clear on turm restart (acceptable for typical minute-scale awaits; documented).
- [x] **`<trigger_name>.awaited` synthesized event** published via `with_publish_bus(bus)` opt-in. Payload carries the original event's payload at the top level (so `{event.<orig>}` keeps working downstream) plus the matched event's payload nested under `await:` (read via `{event.await.<field>}` through the interpolator's new dot-path support). Source label `turm.trigger.await` distinguishes the synthesized origin.
- [x] **Periodic sweep** via `engine.sweep_pending_awaits()` called from turm-linux's 50ms GTK timer. Drops expired entries; for `fire_with_default` policy, publishes the synthesized event with `await: null` so downstream triggers can branch on it.
- [x] **Interpolator dot-path extension**: `event.foo.bar.baz` walks nested JSON objects (returns `None` on non-object hop, which keeps the `{token}` literal in the output as a fail-loud signal — same posture as flat-token resolution).
- [x] **7 new unit tests** in `turm-core::trigger` cover: action success registers pending; action failure does NOT; matching event publishes synthesized with namespaced payload; `payload_match` interpolation against original event filters non-matches; sweep drops expired with abort policy; sweep fires defaulted event with `fire_with_default`; multiple pending entries — only the matched one fires.
- [x] **vision-flow-3.toml** carries a commented-out `slack.dm` ask-and-wait example for Todos without `linked_jira`, demonstrating the typical use shape.

**14.2 — deferred slices**:

- [ ] **`action_result` interpolation in `payload_match`**. Today the await's payload_match can reference `{event.<orig>}` only. Referencing the action's return value (e.g. `payload_match = { thread_ts = "{action_result.ts}" }` for Slack threads) needs synchronous capture of the sink's result, which `LiveTriggerSink` returns as `{queued: true}` for blocking + legacy actions. Closing this needs the engine to chain through `<action>.completed` to capture the real result — a state-machine extension that's bigger than slice 1's scope.
- [ ] **Persistent pending_awaits**. Restart loses any in-flight awaits. Acceptable for typical minute-scale flows; would need a small on-disk journal for hour-scale awaits (e.g. waiting for a slow Slack approval).
- [ ] `workflow.<name>` action namespace for hand-rolled multi-step Rust handlers when chained TOML gets cumbersome. Built into core or registered by a `turm-plugin-workflow` (TBD).

**14.3 — backfill: re-enable derived workflows that need composition**

- [ ] Phase 11.3 derived markdown ingestion (`slack.raw` → LLM summarize → `kb.ensure`) now expressible
- [ ] Phase 12.1 trigger-fired `llm.complete` results land in subsequent triggers
- [ ] LLM example trigger ("auto-summarize DM to KB") rewritten as a real chain, not a fire-and-forget

### Phase 15: Todo system

User explicit requirement. Todos are first-class workflow entry points (Todo `start` action drives the worktree+tmux+Claude flow in 17/18) AND a daily-use UI surface (turm already has `PanelVariant`).

**Packaging**: standalone `turm-plugin-todo` plugin (its own manifest, its own actions, its own UI panel). The plugin SHARES the markdown-with-frontmatter file format with the KB plugin's filesystem layout — todos under `~/docs/todos/...` are just KB documents with a known schema — but the actions, the file-watcher, and the UI all live in `turm-plugin-todo` for clean separation. KB plugin keeps its current surface unchanged. This is what makes `turmctl plugin open todo` (standard `plugin.open` activation surface) work.

**Phase 15 ships in two slices**, with Phase 14.1 (chained triggers) sandwiched between them. **Phase 15.1** is the basics + UI (single-action triggers, usable today with current engine). **Phase 15.2** is the composite "start" workflow chain (depends on Phase 14.1). The slice-1 subsections below are bullet-organized rather than numbered to keep the slice numbering unambiguous.

**Phase 15.1 — Todo basics + UI** (slice 1, current trigger engine) — **shipped**:

- [x] **`turm-plugin-todo` Rust workspace member** (Linux-only via `compile_error!` gate, same posture as `turm-plugin-kb` since `store.rs` reuses `renameat2(RENAME_NOREPLACE)` + `O_NOFOLLOW`). Activation `onStartup` so the file-watcher is alive whenever turm runs (`onAction:todo.*` would only catch programmatic edits, missing vim writes).
- [x] **`todo.create` / `todo.list` / `todo.set_status` / `todo.start` / `todo.delete` actions**. `todo.start` emits a `todo.start_requested` bus event carrying the full Todo payload — already useful for single-step triggers today, hooks into Phase 15.2's chain when 14.1 lands. `todo.set_status` does in-place rewrite of the `status:` frontmatter line (preserving comments/order/spacing the user typed in vim) with a render-from-scratch fallback for malformed files. Atomic create via temp-file + `renameat2(RENAME_NOREPLACE)`; atomic replace via temp-file + `rename`. ID validation rejects path separators, leading dots, control chars; workspace label uses the same charset as KB folder names.
- [x] **Polling file-watcher** scans `<root>/<workspace>/*.md` every `TURM_TODO_POLL_SECS` (default 2s, range 1..=60), tracks `(workspace, id) → (mtime_ns, status)`, and diffs against the previous snapshot to emit `todo.created` / `todo.changed` / `todo.completed` / `todo.deleted`. The first scan after startup is treated as the baseline (no `created` spam on every restart). `todo.completed` only fires on transitions `* → done` observed after first sight — a brand-new todo born `done` does NOT spuriously fire `completed`. Polling chosen over `notify`/inotify to keep dep graph small and match the calendar plugin's pattern; trivial swap if latency hurts the loop.
- [x] **Plugin Panel UI** at `examples/plugins/todo/panel.html` — 3-column kanban (Todo / Doing / Done) with HTML5 drag-and-drop calling `todo.set_status` (column position == status, optimistic move + reload-on-failure snap-back). The Doing column corresponds to `in_progress`. The hover-revealed `Start` button on Todo cards bumps the card to Doing AND fires `todo.start_requested` so the vision-flow-3 chain runs. Items with status `blocked` show in the Todo column with a red badge — there is intentionally no UI to set `blocked` because frontmatter `vim`-edit is the supported workflow; dragging out of Todo commits the column status (loses `blocked` by design). A "hide done" toggle (persisted in localStorage) collapses to a 2-column layout. Live-refreshes on `todo.*` events from the watcher. Activated via the existing `plugin.open` surface (`turmctl plugin open todo`); keybinding deferred (the `Ctrl+Shift+T` slot is already taken by new-tab and Phase 15.1 doesn't reserve a replacement — left as a config-time decision).
- [x] **Example triggers** at `examples/plugins/todo/triggers.example.toml` covering `calendar.event_imminent → todo.create` (prep tasks for upcoming meetings) plus commented sketches for `slack.mention → todo.create`, `todo.completed → kb.append` (daily wins log), and `jira.ticket_assigned → todo.create` (Phase 16 sketch).
- [x] **33 unit tests** covering frontmatter parse / render round-trips, surgical status update preservation, atomic create collision, traversal/hidden-id rejection, list filtering by status/tag/due_before, watcher diff event emission, fatal_error short-circuit, and unknown-action rejection.

Original spec for reference (kept in case a Phase 15 v2 wants to revisit decisions):

- **File format** — Markdown checkbox files at `~/docs/todos/<workspace>/<id>.md` with frontmatter:
  ```yaml
  ---
  id: T-123
  status: open | in_progress | blocked | done
  created: 2026-04-27T12:00:00Z
  due: 2026-05-01
  priority: high | normal | low
  workspace: turm
  linked_jira: PROJ-456
  linked_slack:
    - { team: T0, channel: D123, ts: 1700.000 }
  linked_kb:
    - meetings/abc.md
    - threads/q2-roadmap.md
  tags: [feature, api]
  ---
  body markdown with `- [ ]` subtasks
  ```
- **File-watcher events** — `turm-plugin-todo` parses frontmatter on changes under `~/docs/todos/` and emits `todo.created` / `todo.changed` / `todo.completed` / `todo.deleted`. File is source of truth — vim-edit + git-version compatible.
- **Search delegation** — `kb.search` works on todo files unchanged (frontmatter `tags:` filterable via search-in-text); `turm-plugin-todo` doesn't reimplement search.
- **Actions** — `todo.create {workspace, title, body?, priority?, due?, linked_jira?, linked_slack?, linked_kb?}` (returns `{id, path}`, internally `kb.ensure`); `todo.set_status {id, status}` (read-modify-write of frontmatter `status` field; atomicity via full-file rewrite + `renameat2(RENAME_EXCHANGE)` OR a Phase 9 extension `kb.replace` — decision at impl time); `todo.list {status?, workspace?, due_before?, tag?}`; `todo.start {id}` (emits `todo.start_requested` with full payload for slice-2 chained triggers).
- **UI panel** — Plugin Panel route (HTML/JS via existing `plugin_panel.rs`). `panel.html` lists todos via `turm.call("todo.list")`, renders markdown, exposes click-to-trigger-action. Native GTK fallback if WebView UX proves insufficient. Default activation through the existing `plugin.open` action (`turmctl plugin open todo`); keybinding configurable via `[keybindings]` since `Ctrl+Shift+T` is already "new tab" — `Ctrl+Shift+K` (checklist) or `Ctrl+Shift+G` (agenda) are candidates.
- **Initial single-action example triggers** (work with current engine, no Phase 14 needed):
  - `calendar.event_imminent` → `todo.create` for prep tasks (with link back to event id)
  - `slack.mention` matching specific patterns (e.g. text contains "todo:") → `todo.create`
  - `jira.ticket_assigned` (Phase 16) → `todo.create` linked to the ticket

**Phase 15.2 — composite "start" workflow chain** (slice 2, depends on Phase 14.1) — **shipped**:

The killer demo with the layered prompt seeded into claude. Clicking "start" on a Todo pops a turm tab with claude-code running inside a tmux session in a fresh git worktree — and the prompt is pre-pasted + submitted, assembled from `~/docs/claude/global.md` + the workspace preamble + the Todo's title/body/`prompt` + linked_kb markdown. Jira summary enrichment still pending Phase 16 + 14.2.

- [x] **`git.worktree_add { sanitize_jira: bool }`** — opt-in flag that lowercases the input branch name (preserving slashes for Jira hierarchies like `epic/PROJ-456` → `epic/proj-456`) before validation. Lets the trigger interpolate `{event.linked_jira}` straight from the Todo payload without the user pre-lowercasing in TOML. Default `false` keeps the contract for callers passing pre-prepared branch names.
- [x] **`examples/triggers/vision-flow-3.toml`** — the full chain as `[[triggers]]` rows ready to drop into `~/.config/turm/config.toml`. Three rules:
  1. `todo.start_requested` (with `linked_jira != null`) → `git.worktree_add { branch = "{event.linked_jira}", sanitize_jira = true }`
  2. `todo.start_requested` (with `linked_jira == null`) → `git.worktree_add { branch = "todo-{event.id}" }`
  3. `git.worktree_add.completed` → `claude.start { workspace_path = "{event.path}" }`
- [x] **Convention documented**: the Todo plugin's `workspace` label and the git plugin's workspace `name` are assumed to coincide. If they don't, the user hardcodes `workspace = "<git-name>"` in the trigger params instead of `{event.workspace}`.
- [x] **3 integration tests in `turm-core::trigger`** drive the chain end-to-end via the in-process `TriggerEngine` + `EventBus` + `ActionRegistry::with_completion_bus`: with-jira branch (sanitize_jira flag carried through interpolation), without-jira branch (`todo-<id>` fallback), and failure-halts-chain (`git.worktree_add.failed` does NOT fire `claude.start`).
- [x] **What works end-to-end today**: click Start in Todo panel → new turm tab opens with cwd=worktree → tmux session attached or created → claude REPL ready. User pastes the prompt themselves.

**Deferred to follow-up slices** (intentionally out of scope here to keep the chain shippable now):
- ~~`prompt` pre-fill: needs Phase 18.2 (tmux send-keys timing). claude.start currently rejects `prompt` with `not_implemented`.~~ **Shipped in Phase 18.2** — claude.start accepts `prompt`, delivers via tmux load-buffer + paste-buffer with capture-pane readiness polling. Layered assembly lives in `turm-plugin-todo`'s `assemble_prompt`; chain forwarding via `git.worktree_add`'s `prompt` passthrough.
- Jira summary enrichment via `jira.get_ticket {key=linked_jira}`: needs Phase 16 (Jira plugin) + Phase 14.2's still-pending `action_result` interpolation slice (slice 1 ships only `event` interpolation in payload_match; joining `jira.get_ticket`'s response back into the prompt needs `<action>.completed` correlation that's slice 2).
- `linked_kb` fan-in via `kb.read` per path: same shape as Jira enrichment.
- ~~Optional Slack-question branch when `linked_jira == null` (post → wait for reply with Jira id → use as branch): needs Phase 14.2's `await = { event_kind, payload_match, timeout }` primitive.~~ **Available as of Phase 14.2 slice 1** — example wired (commented) in `examples/triggers/vision-flow-3.toml`.

### Phase 16: Jira plugin

Same shape as Slack plugin — REST + auth + events + actions.

- [ ] **Auth**: API token (Atlassian Cloud) via env or keyring. Email + token combination per Atlassian's API spec. Same token-store pattern as `turm-plugin-slack`.
- [ ] **Polling** (no public webhooks for self-hosted desktop): `/rest/api/3/search` for assigned-to-me + watching tickets. Configurable poll interval (default 300s — Jira rate limits aggressively).
- [ ] **Events**: `jira.ticket_assigned`, `jira.status_changed`, `jira.comment_added`, `jira.mention` (when text mentions current user). Each carries `{key, summary, status, assignee, reporter, project, url}` plus `event_json` raw.
- [ ] **Actions**:
  - `jira.list_my_tickets {status?, project?, updated_since?}`
  - `jira.create_ticket {project, summary, description?, assignee?, parent?}`
  - `jira.transition {key, status}` (via Jira's transition workflow)
  - `jira.add_comment {key, body}`
  - `jira.get_ticket {key}` — returns full ticket json
- [ ] **Cross-plugin trigger example** (in `examples/plugins/jira/triggers.example.toml`): `jira.ticket_assigned` → `todo.create` with `linked_jira` field populated.

### Phase 17: Git workspace plugin (worktree-first)

Lightweight — no external API, just local git operations. **Worktrees, not plain branches**: keeps the original repo dir clean, supports concurrent parallel branches in different turm tabs (one tab per worktree), and `git worktree remove` cleanly tears them down when work is done. Branch-only workflows would force the user to stash/switch and lose the parallel-tabs property.

**Phase 17.1 — actions + workspace config** (slice 1) — **shipped**:

- [x] **`turm-plugin-git` Rust workspace member** (cross-platform: Linux + macOS, `git` is the only binary dependency). Activation `onAction:git.*` (lazy — file-watcher events come in slice 2 and will flip activation to `onStartup`).
- [x] **Workspace config** loaded from `~/.config/turm/workspaces.toml` (override via `TURM_GIT_WORKSPACES_FILE`). Per-entry validation: name follows the KB folder charset, path canonicalized + must contain `.git/`, duplicate names rejected, default_base required, default `worktree_root = <path>-worktrees`. Missing config file is OK (returns empty workspace list).
- [x] **Actions**: `git.list_workspaces`, `git.list_worktrees {workspace}`, `git.worktree_add {workspace, branch, base?}`, `git.worktree_remove {path, force?}`, `git.current_branch {workspace}`, `git.status {workspace?, path?}`. Every git invocation goes through `Command::arg(...)` argv vectors — no shell strings, no injection paths.
- [x] **Branch validation** mirrors `git check-ref-format` at validate-time so bad names fail fast with a tighter error than git would emit. Rules: non-empty, no leading `-`/`/`, no `..`/`@{`/`//`, no whitespace/`~^:?*[\\`, no segment starts with `.` or ends with `.lock`.
- [x] **`git.worktree_add.completed` event** emitted on every successful return — both fresh creation and the secondary-worktree idempotent path — with `{workspace, path, branch, base}` payload (plus the optional `prompt` passthrough). On the idempotent path, `base` echoes the request's base (or the workspace default when omitted), NOT the historical base the worktree was originally created from — `git worktree list --porcelain` doesn't record historical base, so consumers that genuinely need it should treat `event.base` as request-shaped on re-clicks. No in-tree consumer reads `event.base` today; the field exists for trigger interpolation symmetry. Originally the git plugin self-published this event from inside its action handler (Phase 17.1 ship); with Phase 14.1's registry-level fan-out now landed, the platform's `ActionRegistry::with_completion_bus` stamps the event automatically and the manual emit was removed (would have double-fired). Net result for users: identical — the event is on the bus, chained triggers compose.
- [x] **Trust-boundary defense**: `worktree_remove` and `path`-form `status` refuse paths that don't live under any configured workspace `path` or `worktree_root` (canonicalize-existing-prefix + `Path::starts_with` whole-component check) so a misconfigured trigger that interpolates the wrong field can't delete arbitrary directories or leak status from `/etc`. Computed worktree_add target also re-verified to stay under `worktree_root` as belt-and-braces.
- [x] **40 unit tests** (real-`git` repo fixtures via tempdir): branch-name validation positive/negative, current_branch, list_worktrees porcelain v2 parse, worktree_add → list → remove round-trip, branch-exists conflict, status dirty/untracked detection, action-level forbidden-path enforcement, workspace-or-path mutex, fatal_error short-circuit, secondary-worktree idempotent re-add + prompt passthrough preservation, primary-checkout exemption (request on primary's branch surfaces branch_exists), stale-registry fall-through (rm-rf'd worktree dir → falls through to create + git collision error).
- [x] **Idempotent `git.worktree_add` for secondary worktrees**: re-running the same action with a branch that already has a SECONDARY worktree (i.e. one created via `worktree add`, NOT the repo's primary checkout) returns the recorded path via `git worktree list --porcelain` scan without re-invoking `git worktree add`. That makes vision-flow-3's trigger chain re-runnable on Start clicks for the same Todo — the chain reaches claude.start, tmux re-attaches, prompt re-pastes. The primary checkout is intentionally exempted: a request like `git.worktree_add {branch: "main"}` against the repo's main checkout still surfaces `branch_exists` (the primary lives at `ws.path`, not under `worktree_root`, and falsely echoing it would fail the worktree_root guard anyway). The action layer mirrors the create path's full validation — `check_no_symlink_ancestors` on the original recorded path, `canonicalize_existing_or_self` + `path_starts_with` for `..`-bypass closure, `validate_base_ref` on the supplied base — so the idempotent return is not a weaker trust gate than the create. The `prompt` passthrough is echoed on both paths.

**Phase 17.2 — file-watcher events** (slice 2, deferred):

- [ ] Emit `git.worktree_created` / `git.worktree_removed` / `git.branch_created` / `git.branch_deleted` / `git.checkout` from `notify` (or polling) on `.git/HEAD`, `.git/refs/heads/`, `.git/worktrees/` per workspace. Per-workspace event payload. Flip activation to `onStartup` so the watcher is alive whenever turm runs.
- [ ] Useful for live status indicators in turm's status bar / a future git panel; not blocking for Vision Flow 3 since `worktree_add.completed` is already emitted directly by the action.

Original spec for reference (kept while slice 2 is pending):

- [ ] **Workspace concept**: configured via `~/.config/turm/workspaces.toml`:
  ```toml
  [[workspace]]
  name = "turm"
  path = "/home/marshall/dev/turm"
  default_base = "master"
  worktree_root = "/home/marshall/dev/turm-worktrees"  # optional; default = "<path>-worktrees"

  [[workspace]]
  name = "site"
  path = "/home/marshall/dev/site"
  default_base = "main"
  ```
- [ ] **Events** (file-watcher on `.git/HEAD`, `.git/refs/heads/`, `.git/worktrees/`): `git.worktree_created`, `git.worktree_removed`, `git.branch_created`, `git.branch_deleted`, `git.checkout`. Per-workspace event payload.
- [ ] **Actions** (return shapes shown reflect the as-shipped Phase 17.1 wire format; they're objects rather than bare arrays/strings so a `fatal_error` field can ride along on `list_workspaces` for degraded-mode discovery, and so single-result calls echo enough context for trigger interpolation):
  - `git.list_workspaces` → `{ workspaces: [{name, path, default_base, worktree_root, current_branch, worktree_count}], fatal_error }`
  - `git.list_worktrees {workspace}` → `{ workspace, worktrees: [{path, branch, head_sha, locked, prunable}] }`
  - `git.worktree_add {workspace, branch, base?}` → `{ workspace, path, branch, base }`. Path is `<worktree_root>/<branch>` (slashes preserved as path components). Phase 14.1's registry fan-out auto-publishes `git.worktree_add.completed` with the same payload so chained triggers compose.
  - `git.worktree_remove {path, force?}` → `{ workspace, path, removed: true }`. Refuses if the worktree has uncommitted changes unless `force=true`. Refuses paths outside any configured workspace path or worktree_root.
  - `git.current_branch {workspace}` → `{ workspace, branch }`
  - `git.status {workspace?, path?}` → `{ path, branch, upstream, ahead, behind, staged, unstaged, untracked, dirty }`. Exactly one of `workspace` / `path` must be supplied.
- [ ] **Branch name sanitization**: `linked_jira` like `PROJ-456` becomes `proj-456` (lowercase). Slashes from Jira hierarchies (`epic/PROJ-456`) become directory components in the worktree path so `git.worktree_add {branch="feat/PROJ-456"}` lands at `<worktree_root>/feat/PROJ-456/`.
- [ ] **Phase 14 composability test case**: `todo.start_requested` → `git.worktree_add` (chain via `<action>.completed` event). Branch name derived from todo metadata (`linked_jira` if present, else `todo-<id>`).

### Phase 18: Claude Code spawn (with tmux session)

Closes the loop: after a workflow stages a worktree + context, drop the user into Claude Code **inside a tmux session** so the work persists across turm restarts and is reattachable from any terminal.

**Phase 18.1 — `claude.start` action** (slice 1) — **shipped**:

- [x] **Built as a turm-internal socket action** (not a stdio plugin) — composes `tab.new` + `terminal.feed_input` directly against the GTK tab manager, which is the natural home since the action inherently needs window/tab access.
- [x] **`claude.start {workspace_path, session_name?, resume_session?}`**: validates `workspace_path` exists and is a directory (canonicalized). Two distinct paths for `session_name` to keep the contract predictable: when the caller OMITS the field, we DERIVE one from the path (last-2 components, lowercased, non-`[A-Za-z0-9_-]` replaced with `-`); when the caller supplies it EXPLICITLY, we VALIDATE strictly (refuse empty, leading `-`, anything outside `[A-Za-z0-9_-]`) and return `invalid_params` rather than silently rewriting — silent rewrites would mask user typos and break `re-running on the same name re-attaches the same tmux session`. Spawns a new tab whose terminal cwd is `workspace_path` (`TerminalPanel::new_with_cwd` threads `Option<&Path>` through to VTE's `spawn_async`), then feeds `tmux new-session -A -s <name> 'claude [...]'` into the terminal. `-A` re-attaches existing sessions instead of stacking duplicates. Returns `{panel_id, tab, tmux_session, workspace_path}` (matches the `tab.created` event payload shape — `panel_id` is the UUID consumed by `session.info`, `tab` is the numeric index).
- [x] **Shell-safe argument escaping**: `shell_single_quote` POSIX-quotes session_name and the inner claude command, including `'\''` escaping for embedded quotes. Caller-supplied `resume_session` ids cannot inject extra arguments.
- [x] ~~**`prompt` parameter rejected** with `not_implemented`~~ — superseded by Phase 18.2 below; the action now accepts `prompt` and seeds it post-spawn via tmux paste-buffer with two-layer claude-pane safety checks.
- [x] **5 unit tests** cover `validate_tmux_session_name` positive/negative, `sanitize_session_name`, `derive_session_name` (single/two-component, sanitization), `shell_single_quote` (incl. embedded-quote escape).

**Phase 18.2 — prompt seeding via `tmux send-keys`** (slice 2) — **shipped**:

- [x] After spawning the tmux session, deliver `prompt` to claude's running REPL via tmux's `load-buffer` + `paste-buffer -d` (multi-line + special-char safe through a paste buffer) followed by `send-keys Enter` to submit. Runs in a background thread so `claude.start` returns immediately to the caller; failures log to stderr but never propagate (the prompt is post-action best-effort).
- [x] **Readiness detection**: poll `tmux capture-pane` every 200ms for up to 8s, looking for claude-specific substrings (`Anthropic`, `Try "`, `claude --`, lower-cased `claude code`). Generic shell-compatible markers (`> `, `│`) are intentionally NOT accepted because `tmux new-session -A` attaches to a pre-existing session and that session might be a shell — pasting a prompt into a shell would execute it as a command. The paste step ALSO requires `pane_current_command` to report `claude` or `node`. If either signal fails (or both time out), the seeder logs to stderr and skips the paste — the user can paste manually.
- [x] **Mutual exclusion** between `prompt` and `resume_session`. Resume restores existing context; seeding new text on top would just confuse claude.
- [x] **Layered prompt assembly** lives upstream in the Todo plugin (Phase 15.x): `assemble_prompt(todo, docs_root)` reads `<docs_root>/claude/global.md`, `<docs_root>/claude/workspaces/<ws>.md`, the Todo's `prompt` (or title+body), and full markdown of every `linked_kb` path; concatenates into a single string surfaced as `event.assembled_prompt` on `todo.start_requested` (distinct from `event.prompt`, which is the raw per-Todo frontmatter field — both fields ride the same event so consumers can pick whichever they want). claude.start consumes that string as-is. Late-bound — files re-read at start time so common-context evolution between Todo creation and Start picks up automatically. `linked_kb` paths are containment-checked (lexical reject of `..`/absolute, plus symlink-ancestor walk) before reading.
- [x] **Cross-trigger forwarding via `git.worktree_add`'s `prompt` passthrough**: optional `prompt` param echoed verbatim into the action result (and thus the auto-published `git.worktree_add.completed` event), letting `claude.start` interpolate `{event.prompt}` from the chained event without Phase 14.2's async correlation. Localized hack with a clear migration path when 14.2 lands.
- [x] **Pane-process safety check**: claude.start refuses to paste the prompt unless BOTH `tmux capture-pane` shows claude-specific markers (`Anthropic`, `Try "`, `claude --`, `claude code`) AND `pane_current_command` reports `claude` or `node`. `tmux new-session -A` attaches to a pre-existing session ignoring the supplied `claude` command, so without these checks a re-clicked Start could paste the assembled prompt into a shell pane and execute it. Generic `> ` / box-drawing markers were dropped because they fire in shells too.
- [x] **Stub scaffolding via `scripts/install-plugins.sh`**: `~/docs/claude/global.md` is created if missing (idempotent — never overwrites). Workspace stubs at `~/docs/claude/workspaces/<ws>.md` are user-created on demand.
- [x] **5 prompt-assembly unit tests** in `turm-plugin-todo::prompt::tests` cover layer ordering, missing-file resilience, explicit-prompt-overrides-body, linked_jira inclusion, docs-root resolution.

**Phase 18.X — original spec** (kept while slice 2 is pending):

- [ ] **Action `claude.start {workspace_path, prompt?, session_name?, resume_session?}`**:
  1. Opens a new turm tab with `cwd = workspace_path`.
  2. In that tab, runs `tmux new-session -A -s <session_name>` — `-A` attaches if a session with that name already exists, creates otherwise. Default `session_name` derived from the worktree path (last two components, sanitized) so re-running on the same worktree re-attaches the same tmux session instead of stacking new ones.
  3. Inside the tmux session, runs `claude` (or `claude --resume <id>` if `resume_session` provided). Pre-filled `prompt` fed via stdin pipe (`echo <prompt> | claude`) or via `claude --print` — pick whichever the installed claude-code CLI supports cleanly at impl time.
  4. Returns `{panel_id, tab, tmux_session, workspace_path}` (matches `tab.created` event payload shape — `panel_id` UUID for `session.info`, `tab` numeric index for tab-bar UI) so the caller (Todo panel, trigger chain) can reference the spawned session later.
- [ ] **Persistence wins from tmux**: detach the tab → kill turm → next turm restart → `claude.start` with the same `session_name` reattaches the running claude. Long-running tasks (refactors, multi-step reasoning) survive turm crashes and laptop reboots.
- [ ] **Built on `tab.new` + `terminal.exec`** primitives. No custom IPC with claude-code — orchestration is "spawn it in the right place with the right context, let it run." If claude-code adds programmatic surfaces later (e.g. `claude --json-events`) that can land as a separate enhancement.
- [ ] **Phase 14 composability test case**: full end-to-end Vision Flow 3, all triggers in user's `[[triggers]]`:
  1. `todo.start_requested` → `git.worktree_add` → publishes `git.worktree_add.completed {path, branch}`
  2. `git.worktree_add.completed` → `claude.start {workspace_path, prompt}` where prompt is interpolated from the original `todo.start_requested` payload (via Phase 14.2 async correlation that joins the chain's earlier event with the latest one).

### Phase 19: CLI ergonomics + richer context surfaces

User-explicit gap. The plugin landscape (Todo / KB / Calendar / Slack / Jira / Git) ships with a Plugin Panel UI per surface, but the CLI side is still the generic `turmctl plugin run <plugin>.<action> --params '{...}'`. That works — it's how every `[[triggers]]` row already drives those actions — but JSON-string params and bare-action names make it the wrong tool for "I just want to add a todo from the prompt before starting work." On the reading side, `turmctl context` already exists (Phase 8 — surfaces `active_panel` + `active_cwd` from `context.snapshot`) but it stops at raw window/cwd state; "what am I actually working on?" needs that base joined with the open todos, calendar events, and git worktree state for the resolved workspace. Slice 19.1 fixes the writing side; 19.2 expands the existing `turmctl context` surface for the reading side.

**Phase 19.1 — per-plugin ergonomic subcommands** (slice 1):

- [ ] **`turmctl todo`** subcommands: `create <title> [--priority] [--workspace] [--due] [--linked-jira] [--tags]`, `list [--status] [--workspace] [--tag] [--due-before]`, `set <id> --status <open|in_progress|blocked|done>` (status tokens match the wire format the action accepts; the panel's "Doing" column is a label for `in_progress`), `start <id>` (publishes `todo.start_requested` for the vision-flow-3 chain), `delete <id>`. Convenience shorthands `done <id>` / `doing <id>` / `block <id>` desugar to `set <id> --status …` so muscle-memory works without paging the status enum. Defaults the workspace to whatever the current cwd maps to via `[[workspace]]` resolution; falls back to `default`. ID-shorthand: bare prefix matches uniquely (`turmctl todo done T-2026` matches `T-2026-0042` if unambiguous).
- [ ] **`turmctl kb`** subcommands: `new <id> [--title] [--from-stdin]`, `cat <id>` (renders the markdown), `search <query> [--limit]`, `append <id> <text>` (calls `kb.append` with `ensure=true`), `list <prefix>`. Composes the existing kb actions; no new plugin work.
- [ ] **`turmctl calendar`** subcommands: `today` and `next [--within 2h]` map to `calendar.list_events` with sane defaults; `event <id>` maps to `calendar.event_details` for a full payload including description and attendees.
- [ ] **`turmctl slack`** subcommands: `send <channel> <message> [--thread-ts]`, `auth status`, `auth login`. Wraps `slack.post_message` and the existing `auth` subcommand of `turm-plugin-slack`.
- [ ] **`turmctl jira`** subcommands: `mine`, `ticket <key>`, `transition <key> <status>`, `comment <key> <text>` (lands when Phase 16 ships).
- [ ] **`turmctl git`** subcommands: `worktrees [--workspace]`, `wt add <branch> [--workspace] [--sanitize-jira]`, `wt remove <path>`, `status`. Wraps `git.list_worktrees` / `git.worktree_add` etc.
- [ ] **Output mode flags**: each subcommand accepts `--json` (raw payload from the action, identical to `plugin run`), `--yaml`, and a default human-readable table/list. The default mode is what makes this slice valuable; `--json` keeps the surface scriptable for shell pipelines.
- [ ] **Auto-completion**: shell completion script (`turmctl completions zsh|bash|fish`) generated from clap so subcommand discovery doesn't require reading source.
- [ ] **Implementation**: each subcommand is a thin clap wrapper that builds the right action params and calls into the existing socket dispatch (same path `plugin run` uses today). No new IPC, no new actions. Lives in `turm-cli/src/commands.rs` with one module per plugin under `turm-cli/src/plugin_cmds/`.

**Phase 19.2 — context aggregation surfaces** (slice 2, depends on 19.1 + Phase 16):

- [ ] **`turmctl todo show <id>`**: full Todo payload + linked-entity expansion. If `linked_jira` set, fans out to `jira.get_ticket` for summary/status (single call, swallow errors). If `linked_kb` non-empty, runs `kb.read` for each path and renders the first few lines as previews. If recent watcher events for this id (`todo.changed` / `todo.completed`) plus the action-event `todo.start_requested` exist in the bus history (when that lands — currently events are ephemeral, see follow-up below), shows them as a timeline. Output is structured for terminal reading, not for piping; `--json` returns the aggregate as a single object for scripting.
- [ ] **`turmctl context` expansion**: today's `turmctl context` returns the bare `context.snapshot` payload (`active_panel`, `active_cwd`). 19.2 widens it (behind a `--full` flag, default-on for the human renderer; `--json` keeps the raw `context.snapshot` shape for backward-compat with anything already piping it) to fan out into `todo.list` (open + in-progress for the resolved workspace), `calendar.list_events` (next 2h), `git.status` (active worktree + branch + ahead/behind), and `slack.list_unseen_mentions` (if linked — TBD, may slip to 19.3 since it needs a new slack action). Single command for "where am I?" before sitting down.
- [ ] **`turmctl recent [--since 2h]`**: scrollback of recent bus events from a ring buffer. Requires a small core change: `EventBus` retains the last N events (configurable, default 500) so the CLI can ask the running turm "what just happened?" without subscribing live. Useful for "what did Slack/Jira/Calendar surface in the last hour while I was AFK?" Lands as a turm-internal socket action (`event.history`); not a plugin concern.
- [ ] **Cross-plugin enrichment via composition, not new actions**: `turmctl todo show` doesn't gain a special "enrich" code path — it's just N parallel action calls in the CLI, same as a chained trigger would be. Keeps the plugin protocol surface flat and the enrichment logic in one place (CLI) rather than spread across plugin services.

**Phase 19.X — open follow-ups** (not in slices 1/2):

- [ ] **Workspace resolution from cwd**: needs a turm-internal "given cwd, what's the workspace label?" helper. Today the git plugin has a `resolve_workspace(path)` notion in `git.status`; lift it into core or socket so `turmctl todo create` can use it without spawning git first. Lands when 19.1 picks up the workspace-default behavior.
- [ ] **Bus-history retention** (`event.history`): Phase 19.2's `turmctl recent` needs `EventBus` to keep a bounded ring buffer. Trivial implementation; tracked here so the turn isn't surprised when the action shows up.
- [ ] **CLI as a panel surface**: a longer arc — let the CLI render the same kanban Todo board (text-mode), full-screen, when invoked as `turmctl todo board`. Useful in SSH sessions where the GUI panel isn't reachable. Out of scope for 19.1/19.2; flagged here so the subcommand layout stays consistent if we do it later.

### Phase 20: Discord plugin

User-explicit gap. Same shape as Slack (Phase 11) — a long-lived WebSocket plugin that emits messenger events into the bus. Discord's Gateway protocol is more involved than Slack's Socket Mode (explicit heartbeat, IDENTIFY/RESUME OP codes, intents declaration), but the plugin's external surface mirrors Slack: `<plugin> auth` one-time CLI, keyring-backed token store, `onStartup` activation, `discord.message` / `discord.mention` / `discord.dm` events plus `discord.send_message` action.

**Phase 20.1 — auth + manifest** (slice 1) — **shipped**:

- [x] **`turm-plugin-discord` Rust workspace member** (Linux + macOS). Same dep set as Slack: `ureq` (HTTPS), `tungstenite` (WebSocket, scaffolded for slice 2), `keyring`. Workspace registered in root `Cargo.toml`.
- [x] **`turm-plugin-discord auth` subcommand**. Reads `TURM_DISCORD_BOT_TOKEN` env, calls Discord's `GET /users/@me` with `Authorization: Bot <token>`, parses the response, persists `TokenSet { bot_token, user_id, username }` via the same keyring-with-plaintext-fallback pattern Slack uses (`KeyringStore` / `PlaintextStore` / `BrokenStore` triplet, namespaced by `TURM_DISCORD_WORKSPACE`). `TURM_DISCORD_REQUIRE_SECURE_STORE=1` refuses plaintext fallback identically to Slack.
- [x] **`discord.auth_status` action**. Returns `{configured, authenticated, store_kind, workspace, user_id, username}`. Mirrors Slack's `slack.auth_status` so a future `turmctl context --full` can surface both workspaces uniformly.
- [x] **Plugin manifest** at `examples/plugins/discord/plugin.toml`. `onStartup` activation (Gateway WebSocket lives whenever turm runs once slice 2 ships). `provides = ["discord.auth_status"]` only; slice 2 adds `discord.send_message`. Required scopes / OAuth flow / MESSAGE CONTENT intent setup documented inline in the manifest.
- [x] **2 unit tests** in `config.rs` cover workspace label charset acceptance and rejection.

**Phase 20.2 — Gateway WebSocket + message events** (slice 2, deferred):

- [ ] `gateway.rs` Gateway v10 client: HELLO → IDENTIFY (with intents bitfield: `GUILD_MESSAGES + MESSAGE_CONTENT + DIRECT_MESSAGES` for the typical "watch DMs and mentions" flow) → heartbeat thread (interval from HELLO) → DISPATCH MESSAGE_CREATE → RESUME on disconnect (track session_id + last seq from each DISPATCH; on reconnect, RESUME instead of IDENTIFY).
- [ ] `events.rs` parses MESSAGE_CREATE → emits one of `discord.message` (any), `discord.mention` (when bot user_id is in `mentions[]`), `discord.dm` (when channel is type 1 / DM). Each event payload carries `{channel_id, channel_type, author_id, author_username, content, message_id, ts, raw}` plus a `discord.raw` archive event for full-fidelity ingestion (mirrors `slack.raw`).
- [ ] `discord.send_message` action: `POST /channels/{channel_id}/messages` with `{content}`. Returns `{message_id, ts}` so chained triggers can reference the posted message.
- [ ] `discord.add_reaction` / `discord.list_channels` deferred to slice 3 — convenience actions, not blocking.
- [ ] **Cross-plugin trigger example** in `examples/plugins/discord/triggers.example.toml`: `discord.dm + payload_match { author_id = "<my-id>" } → todo.create` for "DM the bot a Todo title" workflow.

**Phase 20.X — open follow-ups**:

- [ ] OAuth redirect flow as alternative to bot-token paste — needs a localhost listener; defer until env+keyring proves insufficient (same posture as Slack).
- [ ] **Voice / slash command surface**: not in scope — text-channel ingestion + send is the messenger-style use case turm cares about.

## Pending Cleanup

- [x] ~~Remove turm-core/pty.rs and state.rs (VTE handles PTY on Linux, SwiftTerm on macOS)~~
- [x] ~~Unify D-Bus and Socket API — D-Bus removed, socket is the sole IPC~~

## Reference Projects

- `~/dev/cmux/` — Socket protocol, CLI structure, window/workspace model
- `~/kitty-random-bg.sh` — Background rotation logic (ported to turm-random-bg.sh)
- Zellij — Panel/plugin architecture reference
- Wezterm — Lua scripting, multiplexer model
