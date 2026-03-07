# Architecture

## Overview

custerm is a cross-platform custom terminal emulator built with a shared Rust core and platform-native UIs. Originally planned with Tauri v2 + React, but pivoted to native UIs due to Tauri IPC input latency (see [decisions.md](./decisions.md)).

## Crate Layout

```
custerm/
├── Cargo.toml              # Workspace root (resolver = "2", edition = "2024")
├── custerm-core/            # Shared Rust library
│   └── src/
│       ├── lib.rs           # Module declarations
│       ├── config.rs        # TOML config loading/defaults
│       ├── background.rs    # Background image cache & rotation
│       ├── protocol.rs      # cmux V2 JSON protocol types
│       ├── state.rs         # AppState, Workspace model
│       ├── pty.rs           # PTY session (portable-pty)
│       └── error.rs         # Error types (thiserror)
├── custerm-linux/           # GTK4 + VTE4 native terminal
│   ├── src/
│   │   ├── main.rs          # Entry point, CLI flags (--init-config, --config-path)
│   │   ├── app.rs           # GtkApplication setup, dark theme
│   │   ├── window.rs        # ApplicationWindow, D-Bus polling, bg init
│   │   ├── terminal.rs      # VTE terminal + background overlay compositing
│   │   ├── tabs.rs          # Tab manager (Notebook, tab bar, keyboard shortcuts)
│   │   ├── split.rs         # Split pane tree (SplitNode, TabContent)
│   │   ├── search.rs        # In-terminal search bar (Ctrl+Shift+F, VTE regex search)
│   │   ├── panel.rs         # Panel trait + PanelVariant enum
│   │   ├── webview.rs       # WebView panel (WebKitGTK 6.0)
│   │   ├── socket.rs        # Unix socket server + command dispatcher
│   │   └── dbus.rs          # D-Bus service (com.marshall.custerm)
│   ├── custerm.desktop      # Desktop entry for system integration
│   └── install.sh           # Build + install script
├── custerm-cli/             # CLI control tool (binary: custermctl)
│   └── src/
│       ├── main.rs          # Entry point, output formatting
│       ├── commands.rs      # clap subcommands (session, background, tab, split, event, webview)
│       └── client.rs        # Unix socket client
└── custerm-macos/           # Swift/AppKit app (stub)
    ├── Package.swift        # Swift Package Manager config (Swift 6, macOS 14+)
    └── Sources/Custerm/
        └── CustermApp.swift # Basic NSWindow, terminal view TBD
```

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Core library | Rust (shared across platforms) |
| Linux terminal | GTK4 + VTE4 (VTE handles PTY internally, zero IPC overhead) |
| macOS terminal | Swift/AppKit + SwiftTerm or Ghostty embedding (TBD) |
| CLI tool | clap (Rust) |
| Config | TOML (`~/.config/custerm/config.toml`) |
| IPC (Linux) | D-Bus session bus (`com.marshall.custerm`) |
| IPC (socket) | Unix domain socket, cmux V2 newline-delimited JSON |
| Background mgmt | File cache at `~/.cache/custerm/wallpapers.txt` |
| Theme | Catppuccin Mocha (hardcoded palette) |

## Key Dependencies

### custerm-core
- `portable-pty 0.8` - Cross-platform PTY
- `serde 1` + `serde_json 1` + `toml 0.8` - Serialization
- `uuid 1` - Session IDs
- `dirs 6` - XDG directories
- `thiserror 2` - Error types
- `rand 0.9` - Random background selection (`rand::seq::IndexedRandom`)

### custerm-linux
- `gtk4 0.9` (features: `gnome_46`) - UI framework
- `vte4 0.8` - Terminal widget (libvte-2.91-gtk4)
- `webkit6 0.4` - WebView panel (WebKitGTK 6.0)
- `env_logger 0.11` - Logging

### custerm-cli
- `clap 4` (features: `derive`) - Argument parsing
- `uuid 1` - Request IDs

## Socket Server (IPC)

custerm runs a Unix domain socket server for programmatic control alongside D-Bus.

**Path**: `/tmp/custerm-{PID}.sock` (per-process, discovered via `CUSTERM_SOCKET` env var)

**Protocol**: Newline-delimited JSON (`Request` → `Response`, defined in `custerm-core/protocol.rs`)

**Architecture**:
```
custermctl ──Unix socket──► socket server (per-client thread)
                                │
                          mpsc::channel
                                │
                          glib::timeout_add_local (50ms poll on GTK thread)
                                │
                          dispatch() ──► TabManager / TerminalPanel
                                │
                          oneshot response ──► socket thread ──► client
```

**Supported commands**: `system.ping`, `background.set`, `background.clear`, `background.set_tint`, `background.next`, `background.toggle`, `tab.new`, `tab.close`, `tab.list`, `tab.info`, `tab.rename`, `tabs.toggle_bar`, `split.horizontal`, `split.vertical`, `session.list`, `session.info`, `event.subscribe`, `terminal.read`, `terminal.state`, `terminal.exec`, `terminal.feed`, `terminal.history`, `terminal.context`, `agent.approve`, `webview.open`, `webview.navigate`, `webview.back`, `webview.forward`, `webview.reload`, `webview.execute_js`, `webview.get_content`, `webview.screenshot`, `webview.query`, `webview.query_all`, `webview.get_styles`, `webview.click`, `webview.fill`, `webview.scroll`, `webview.page_info`, `webview.devtools`

**Cleanup**: Socket file removed on window destroy.

## Event Stream

Clients can subscribe to real-time events via `event.subscribe`. The socket stays open and streams newline-delimited JSON events.

**Protocol**: Send `{"id":"...","method":"event.subscribe","params":{}}`, receive `{"id":"...","ok":true,"result":{"status":"subscribed"}}`, then receive event lines indefinitely.

**Event format**: `{"type":"<event_type>","data":{...}}`

**Event types**:
| Event | Data | Trigger |
|-------|------|---------|
| `panel.focused` | `{panel_id}` | Panel gains focus |
| `panel.title_changed` | `{panel_id, title}` | Terminal window title changes |
| `panel.exited` | `{panel_id, tab}` | Shell process exits |
| `tab.created` | `{panel_id, tab}` | New tab opened |
| `tab.closed` | `{panel_id, tab}` | Tab closed |
| `terminal.output` | `{panel_id, text}` | Terminal receives output (high frequency) |
| `terminal.cwd_changed` | `{panel_id, cwd}` | Terminal CWD changes (OSC 7) |
| `terminal.shell_precmd` | `{panel_id}` | Shell prompt ready (precmd) |
| `terminal.shell_preexec` | `{panel_id}` | Command about to execute (preexec) |
| `terminal.notification` | `{panel_id, summary, body}` | OSC 9/777 notification received |
| `webview.loaded` | `{panel_id}` | WebView finishes loading |
| `webview.title_changed` | `{panel_id, title}` | WebView title changes |
| `webview.navigated` | `{panel_id, url}` | WebView URI changes |
| `tab.renamed` | `{panel_id, title}` | Tab renamed |

**Usage**: `custermctl event subscribe` — prints events as JSON lines to stdout.

## Query API

**`session.list`**: Returns all panels across all tabs with `[{id, type, title, tab, focused, url?}]`. WebView panels include `url`.

**`session.info`** (`{id}`): Returns detailed panel info. Terminal: `{id, type, title, tab, focused, cols, rows, cursor: [row, col]}`. WebView: `{id, type, title, tab, focused, url}`.

**`tab.info`**: Returns extended tab info: `{count, current, tabs: [{index, panel_count, title}]}`.

## Terminal Agent API

Commands for programmatic terminal interaction (AI agent integration).

| Command | Params | Response |
|---------|--------|----------|
| `terminal.read` | `id?`, `start_row?`, `start_col?`, `end_row?`, `end_col?` | `{text, cursor: [row, col], rows, cols}` |
| `terminal.state` | `id?` | `{cols, rows, cursor: [row, col], cwd, title}` |
| `terminal.exec` | `id?`, `command` | Sends command + newline to terminal PTY |
| `terminal.feed` | `id?`, `text` | Sends raw text to terminal PTY (no newline) |
| `terminal.history` | `id?`, `lines?` (default 100) | `{text, lines_requested, rows, cols}` — scrollback buffer |
| `terminal.context` | `id?`, `history_lines?` (default 50) | `{state, screen, history}` — combined context |

All commands default to the active terminal panel when `id` is omitted.

**CLI usage**:
```bash
custermctl terminal state
custermctl terminal read --start-row 0 --end-row 5
custermctl terminal exec "ls -la"
custermctl terminal feed $'\x03'  # Send Ctrl+C
custermctl terminal history --lines 200
custermctl terminal context --history-lines 100
```

## Approval Workflow

AI agents can request user approval before taking actions.

| Command | Params | Response |
|---------|--------|----------|
| `agent.approve` | `message`, `title?`, `actions?` | `{approved, action, index}` |

Shows a modal GTK dialog and blocks until the user responds. The `actions` param is an array of button labels (default: `["Approve", "Deny"]`). The first action (index 0) is treated as "approved".

**CLI usage**:
```bash
custermctl agent approve "Delete 15 files from /tmp?"
custermctl agent approve "Deploy to production?" --title "Deploy" --actions "Deploy,Cancel"
```

## Panel System

custerm supports multiple panel types via the `PanelVariant` enum:

- **Terminal** (`TerminalPanel`): VTE4 terminal with shell, background images, search
- **WebView** (`WebViewPanel`): WebKitGTK 6.0 browser panel with JS execution, URL toolbar (back/forward/reload/URL entry/DevTools toggle)

The `Panel` trait provides a common interface (`widget()`, `title()`, `panel_type()`, `grab_focus()`, `id()`). `PanelVariant` delegates to the inner type and provides `as_terminal()` / `as_webview()` accessors.

### Tab Bar Controls

The tab bar has two modes: **collapsed** (icon-only, default) and **expanded** (icon + label + close button). Toggle with `Ctrl+Shift+B` or the toggle button in the tab bar. Tabs can be renamed by double-clicking the tab label or via socket API. Custom titles suppress auto-title updates from terminal/webview.

**Auto-expand**: When going from 1 to 2 tabs, the tab bar auto-expands. Once the user manually toggles, that preference is preserved. The tab bar is never fully hidden — collapsed mode shows panel type icons and a toggle button.

| Command | Params | Behavior |
|---------|--------|----------|
| `tabs.toggle_bar` | — | Toggle tab bar visibility, returns `{visible}` |
| `tab.rename` | `id`, `title` | Rename a tab by panel ID |

### WebView API

| Command | Params | Behavior |
|---------|--------|----------|
| `webview.open` | `url`, `mode?` (tab/split_h/split_v) | Create webview panel, return panel_id |
| `webview.navigate` | `id`, `url` | Navigate existing webview |
| `webview.back` | `id` | Go back in history |
| `webview.forward` | `id` | Go forward in history |
| `webview.reload` | `id` | Reload page |
| `webview.execute_js` | `id`, `code` | Run JS, return result (async) |
| `webview.get_content` | `id`, `format?` (text/html) | Get page content via JS (async) |
| `webview.screenshot` | `id`, `path?` | Take screenshot (base64 PNG or save to file) |
| `webview.query` | `id`, `selector` | Query single DOM element (tag, text, rect, attrs) |
| `webview.query_all` | `id`, `selector`, `limit?` | Query all matching elements |
| `webview.get_styles` | `id`, `selector`, `properties` | Get computed CSS styles for element |
| `webview.click` | `id`, `selector` | Click a DOM element |
| `webview.fill` | `id`, `selector`, `value` | Type text into an input element |
| `webview.scroll` | `id`, `selector?`, `x?`, `y?` | Scroll to position or element |
| `webview.page_info` | `id` | Page metadata (title, dimensions, element counts) |
| `webview.devtools` | `id`, `action?` (show/close/attach/detach) | Control WebKit DevTools inspector |

`webview.execute_js`, `webview.get_content`, `webview.screenshot`, and all DOM query/interaction commands use async dispatch — the reply sender is captured by the WebKit callback and sent when execution completes. DOM commands use pre-built JS snippets from `webview::js` module.

## System Prerequisites

### Arch Linux
```bash
sudo pacman -S gtk4 vte4 webkitgtk-6.0
```

### macOS
- Xcode with Swift 6
