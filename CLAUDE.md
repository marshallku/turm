# turm

Cross-platform custom terminal emulator with shared Rust core and platform-native UIs.

## Documentation

**Always read `docs/INDEX.md` first** when starting a session. Read only the specific doc files relevant to your current task.

**Always update docs** when making changes:

- New features or modules → update `docs/architecture.md` and relevant doc
- Bug fixes or gotchas → add to `docs/troubleshooting.md`
- Design decisions → add to `docs/decisions.md`
- Completed/new tasks → update `docs/roadmap.md`

## Project Structure

- `turm-core/` — Shared Rust library (config, background, plugin, protocol, theme, error)
- `turm-linux/` — GTK4 + VTE4 native terminal app (binary: `turm`)
- `turm-cli/` — CLI control tool (binary: `turmctl`)
- `turm-macos/` — Swift/AppKit app (stub)
- `docs/` — Project documentation (architecture, decisions, troubleshooting, roadmap)

## Build & Run

```bash
# Build all
cargo build

# Run terminal
cargo run -p turm-linux

# Run CLI
cargo run -p turm-cli -- <command>
```

## Key Conventions

- Rust edition 2024, Cargo workspace with `resolver = "2"`
- GTK4 with `gnome_46` feature flag
- VTE handles PTY on Linux (no custom PTY management)
- D-Bus (`com.marshall.turm`) for Linux IPC
- Config: `~/.config/turm/config.toml` (TOML)
- Cache: `~/.cache/turm/wallpapers.txt`
- Theme: Catppuccin Mocha (hardcoded)
- Dark theme forced via GTK settings

## Critical Implementation Details

- **Background images**: Must call `terminal.set_clear_background(false)` for VTE transparency
- **GTK thread safety**: D-Bus → mpsc channel → glib::timeout_add_local polling
- **Binary names**: `turm` (app) and `turmctl` (CLI) — do not rename to collide
