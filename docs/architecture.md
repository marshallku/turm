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

## System Prerequisites

### Arch Linux
```bash
sudo pacman -S gtk4 vte4
```

### macOS
- Xcode with Swift 6
