# turm

<img width="3440" height="1440" alt="image" src="https://github.com/user-attachments/assets/a1392646-1255-40ed-9722-ea8523a5c342" />

A custom terminal emulator built with a shared Rust core and platform-native UIs. GPU-rendered background images, Catppuccin Mocha theme, and a control API designed for both human and AI agent use.

![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- **GPU-rendered backgrounds** — wallpaper images composited behind the terminal with configurable tint and opacity
- **Catppuccin Mocha theme** — hardcoded color palette
- **Tabs** — create, switch, and split terminal tabs (horizontal/vertical)
- **Dynamic font scaling** — `Ctrl+=`/`Ctrl+-`/`Ctrl+0`
- **D-Bus control** — change backgrounds, tint, and more at runtime
- **CLI tool (`turmctl`)** — control the terminal from the command line
- **TOML configuration** — simple config at `~/.config/turm/config.toml`

## Screenshots

_Coming soon_

## Requirements

### Arch Linux

```bash
sudo pacman -S gtk4 vte4
```

### Other Linux

Install GTK4 and libvte-2.91-gtk4 from your distribution's package manager.

### macOS

Xcode Command Line Tools (for SwiftPM) and Rust (for `turmctl`).

```bash
xcode-select --install
# https://rustup.rs for Rust
```

The macOS app targets macOS 14+. SwiftTerm is fetched as an SPM dep at build time.

## Build & Run

```bash
# Build all crates
cargo build

# Run the terminal
cargo run -p turm-linux

# Generate a default config file
cargo run -p turm-linux -- --init-config

# Control the running terminal via CLI
cargo run -p turm-cli -- <command>
```

## Install

### Linux — From GitHub Releases (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/marshallku/turm/master/install.sh | bash
```

Options:

- `--version v0.2.0` — install a specific version
- `--system` — install to `/usr/local/bin` (requires sudo)

### Linux — From source

```bash
./scripts/install-dev.sh           # build + install everything (sudo, /usr/local/bin)
./scripts/install-dev.sh --user    # ~/.local/bin (no sudo)
./scripts/install-dev.sh --restart # also pkill -x turm afterwards
```

This builds a release binary, installs the desktop entry, and lays down first-party plugins.

### macOS — From source

```bash
./scripts/install-macos.sh             # ~/Applications + ~/.cargo/bin (no sudo)
./scripts/install-macos.sh --system    # /Applications + ~/.cargo/bin (sudo for /Applications)
./scripts/install-macos.sh --launch    # open Turm.app after installing
```

This builds the macOS app via SwiftPM (release config), installs `Turm.app`, and installs `turmctl` via `cargo install --path turm-cli` (the workspace root is a virtual manifest, so the more obvious `cargo install turm-cli` and `cargo install --path .` both fail — the script wraps the correct invocation).

For dev iteration without installing, use `turm-macos/run.sh` (debug bundle under `.build/debug/Turm.app`, opened in place).

### Update

```bash
turmctl update check    # check for new versions
turmctl update apply    # download and install latest (Linux only — macOS users re-run install-macos.sh)
```

## Configuration

Config file: `~/.config/turm/config.toml`

```toml
[terminal]
shell = "/bin/zsh"
font_family = "JetBrainsMono Nerd Font Mono"
font_size = 14

[background]
directory = "/path/to/wallpapers/"
tint = 0.85
opacity = 0.95

[tabs]
position = "top"  # top, bottom, left, right

[socket]
path = "/tmp/turm.sock"

[theme]
name = "catppuccin-mocha"
```

All fields have defaults — the config file is entirely optional.

## Project Structure

```
turm/
├── turm-core/    # Shared Rust library (config, background, protocol, state)
├── turm-linux/   # GTK4 + VTE4 native terminal app
├── turm-cli/     # CLI control tool (turmctl)
├── turm-macos/   # Swift/AppKit app (stub)
└── docs/            # Internal documentation
```

## License

MIT
