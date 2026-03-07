# Roadmap

## Vision

custerm = terminal-centric programmable workspace.
Terminal at the core, but extensible to browser panels, AI agents, and custom views.
Everything controllable via API so AI agents can operate the workspace.

## Implementation Phases

### Phase 1: MVP Terminal ✅
- [x] Cargo workspace with custerm-core, custerm-linux, custerm-cli
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
- [x] Shell script for random rotation daemon (`custerm-random-bg.sh`)
- [x] Config hot-reload (file watcher)

### Phase 3: Tabs + Panel System ✅
Terminal tabs first, then generalize to support different panel types.

- [x] **Tab model**: `Panel` trait with `TerminalPanel` as first impl
- [x] **TabBar**: new / close / switch / reorder (drag)
- [x] **Split panes**: horizontal / vertical split within a tab
- [x] **Pane resize**: drag dividers
- [x] **Focus tracking**: active pane focus via `EventControllerFocus`
- [x] **Keyboard shortcuts**: Ctrl+Shift+T/W/Tab, Ctrl+Shift+E/O (split), Ctrl+Shift+[1-9]
- [x] **Configurable tab position**: top, bottom, left, right (`[tabs] position` in config)
- [x] **In-terminal search**: Ctrl+F search bar with VTE regex (next/prev/case toggle)
- [ ] **Panel type registry**: extensible system for registering new panel types

### Phase 4: Control API
Single programmable interface for both human CLI and AI agents.

- [x] CLI tool (custermctl) with clap subcommands
- [x] cmux V2 JSON protocol types
- [x] Unix socket client
- [x] **Socket server** in custerm-linux (Unix socket, per-PID path)
- [x] **Command dispatch**: system.ping, background.set/clear/set_tint/next/toggle, tab.new/close/list, split.horizontal/vertical
- [x] **Env var injection**: CUSTERM_SOCKET per terminal session
- [ ] **Event stream**: subscribe to terminal output, focus changes, panel lifecycle
- [ ] **Query API**: read terminal screen content, list panels/tabs, get state

### Phase 5: WebView Panel
Embed browser as a panel type alongside terminals.

- [ ] **WebKitGTK panel**: browser view as a Panel impl
- [ ] **URL bar / navigation** within panel
- [ ] **DevTools toggle**
- [ ] **JS ↔ custerm bridge**: page scripts can call custerm API
- [ ] **Side-by-side workflow**: terminal + docs/PR/CI in one window

### Phase 6: AI Agent Integration
Make custerm a first-class environment for AI coding agents.

- [ ] **Agent protocol**: structured input/output channel (beyond raw PTY text)
- [ ] **Screen reading API**: semantic terminal content (not just raw bytes)
- [ ] **Command execution API**: AI sends commands, gets structured results
- [ ] **Notification channel**: OSC 9/99/777 parsing for agent status
- [ ] **Approval workflow**: AI proposes action → user confirms in custerm UI
- [ ] **Context sharing**: share terminal history, file paths, git status with agent

### Phase 7: Polish + Ecosystem
- [ ] Theme system (parse theme files, multiple palettes)
- [ ] Clipboard integration (OSC 52)
- [ ] URL detection + click-to-open
- [ ] Session persistence / restore
- [ ] Plugin system (Lua or WASM for custom panels/commands)
- [ ] macOS native app (Swift/AppKit)

## Pending Cleanup
- [ ] Consider whether custerm-core/pty.rs and state.rs are needed for Linux (VTE handles PTY)
- [ ] Unify D-Bus and Socket API (D-Bus for system integration, Socket for rich control)

## Reference Projects
- `~/dev/cmux/` — Socket protocol, CLI structure, window/workspace model
- `~/kitty-random-bg.sh` — Background rotation logic (ported to custerm-random-bg.sh)
- Zellij — Panel/plugin architecture reference
- Wezterm — Lua scripting, multiplexer model
