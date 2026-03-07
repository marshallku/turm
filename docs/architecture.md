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
│   │   ├── search.rs        # In-terminal search bar (Ctrl+F, VTE regex search)
│   │   ├── panel.rs         # Panel trait
│   │   ├── socket.rs        # Unix socket server + command dispatcher
│   │   └── dbus.rs          # D-Bus service (com.marshall.custerm)
│   ├── custerm.desktop      # Desktop entry for system integration
│   └── install.sh           # Build + install script
├── custerm-cli/             # CLI control tool (binary: custermctl)
│   └── src/
│       ├── main.rs          # Entry point, output formatting
│       ├── commands.rs      # clap subcommands (window, workspace, session, background)
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

**Supported commands**: `system.ping`, `background.set`, `background.clear`, `background.set_tint`, `background.next`, `background.toggle`, `tab.new`, `tab.close`, `tab.list`, `tab.info`, `split.horizontal`, `split.vertical`, `session.list`, `session.info`, `event.subscribe`

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

**Usage**: `custermctl event subscribe` — prints events as JSON lines to stdout.

## Query API

**`session.list`**: Returns all panels across all tabs with `[{id, title, tab, focused}]`.

**`session.info`** (`{id}`): Returns detailed panel info: `{id, title, tab, focused, cols, rows, cursor: [row, col]}`.

**`tab.info`**: Returns extended tab info: `{count, current, tabs: [{index, panel_count, title}]}`.

## System Prerequisites

### Arch Linux
```bash
sudo pacman -S gtk4 vte4
```

### macOS
- Xcode with Swift 6
