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
- [x] **Resolved (Phase 9.4):** `ActionRegistry` now distinguishes sync from blocking handlers. New API: `register_blocking` (same handler signature as `register`, just flags the entry); `try_dispatch(self: &Arc<Self>, name, params, on_done) -> bool` that runs sync handlers inline (callback fires synchronously on caller thread) and spawns a worker thread for blocking handlers (callback fires from worker). Existing `invoke` / `try_invoke` retained for tests + explicit-block contexts; new `is_blocking()` for diagnostic branches. `service_supervisor` registers every plugin action via `register_blocking` because `invoke_remote` parks waiting for the stdio reply. **Caller migration:** `socket::dispatch` uses `try_dispatch` uniformly — its reply path is already channel-based so sync vs blocking is invisible to the CLI client. `LiveTriggerSink::dispatch_action` deliberately branches on `is_blocking()` to preserve the pre-Phase-9.4 trigger contract: sync handlers keep their synchronous error path so `TriggerEngine`'s `log::warn` and `fired` count remain accurate; blocking handlers go through `try_dispatch` and return `Ok({queued})` because the engine can't await a worker without re-introducing the GTK stall. 11 new unit tests across both crates: caller-thread inline for sync, worker-thread observation for blocking with `<40ms` return assertion, error propagation through both, `is_blocking` flag accuracy, register-vs-register_blocking overwrite, plus four LiveTriggerSink tests covering sync-Ok / sync-Err / blocking-fast-return / unknown-action-fallthrough.
- [ ] **Known limitation, Phase 9.4 ships with O(n) threads under blocking burst:** `try_dispatch` spawns a dedicated OS thread per blocking call, and the supervisor's `dispatch_invocation` already spawns a waiter thread per invocation. Under a burst of N concurrent slow plugin actions that's `2N` sleeping threads. Acceptable for v1 traffic (a few triggers/min + occasional `turmctl call`); becomes a real cost when the LLM plugin lands and triggers fan out to many concurrent completions. Fix path: shared thread pool (rayon, or hand-rolled bounded-channel worker pool) for the registry, and reuse for the supervisor. Tracked here so it isn't forgotten.
- [x] `ServiceSupervisor::shutdown_all()` — wired from `window.connect_destroy`. Sends the documented `shutdown` notification to every Running service, drops the writer-channel sender so child stdin closes on EOF, and SIGKILLs any pid still recorded after a 200ms grace window. Idempotent.
- [x] **Resolved (Phase 9.5):** orphan-on-crash hardening. (a) Linux `prctl(PR_SET_PDEATHSIG, SIGTERM)` set in the spawn `pre_exec` hook so the kernel sends SIGTERM to the plugin whenever its parent (turm) dies for any reason — including SIGKILL, segfault, or a panic before the GTK destroy callbacks fire. The fork↔prctl race is closed by capturing `getppid()` BEFORE arming the signal and re-checking after — if the parent already died (getppid changed to 1) we `libc::_exit(1)` rather than running an orphaned plugin whose death notice will never arrive. Best-effort: `prctl` failures are swallowed (older kernel / locked-down sandbox) so the worst case is the pre-fix orphan behavior, never a failed spawn. macOS / BSD path unchanged via `cfg(target_os = "linux")`. (b) `glib::unix_signal_add_local` SIGTERM/SIGINT handlers in `app.rs` close all GTK windows on signal, which fires the existing `connect_destroy → ServiceSupervisor::shutdown_all` chain. Together: PR_SET_PDEATHSIG (with race-recheck) covers the unrecoverable cases (SIGKILL/segfault), the signal handler covers the cooperative cases (Ctrl-C / `kill <pid>` SIGTERM).
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

## Pending Cleanup

- [x] ~~Remove turm-core/pty.rs and state.rs (VTE handles PTY on Linux, SwiftTerm on macOS)~~
- [x] ~~Unify D-Bus and Socket API — D-Bus removed, socket is the sole IPC~~

## Reference Projects

- `~/dev/cmux/` — Socket protocol, CLI structure, window/workspace model
- `~/kitty-random-bg.sh` — Background rotation logic (ported to turm-random-bg.sh)
- Zellij — Panel/plugin architecture reference
- Wezterm — Lua scripting, multiplexer model
