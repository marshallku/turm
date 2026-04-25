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
- [ ] **Trigger engine** — TOML rules, hot-reload, `{event.*}` / `{context.*}` interpolation
- [ ] **Google Calendar provider** (OAuth + polling + `calendar.event_imminent` events + context contribution)
- [ ] **First vertical PoC**: meeting-prep trigger opens meeting link tab + Notion doc in WebView panel split
- [ ] Slack / Discord event gateway (native WebSocket adapter, Event Bus publisher)
- [ ] Notion document provider (WebView panel with saved-documents quick switcher)
- [ ] Command palette (Ctrl+Shift+P) over Action Registry
- [ ] Knowledge base layer (local embeddings + semantic search over context + notes)

## Pending Cleanup

- [x] ~~Remove turm-core/pty.rs and state.rs (VTE handles PTY on Linux, SwiftTerm on macOS)~~
- [x] ~~Unify D-Bus and Socket API — D-Bus removed, socket is the sole IPC~~

## Reference Projects

- `~/dev/cmux/` — Socket protocol, CLI structure, window/workspace model
- `~/kitty-random-bg.sh` — Background rotation logic (ported to turm-random-bg.sh)
- Zellij — Panel/plugin architecture reference
- Wezterm — Lua scripting, multiplexer model
