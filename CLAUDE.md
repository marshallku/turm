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

## Local development install

`install.sh` is for end users on Linux (downloads from GitHub Releases). For dev iteration on the working tree, use:

```bash
# Linux
./scripts/install-dev.sh           # cargo build --release + sudo install /usr/local/bin/{turm,turmctl} + plugins
./scripts/install-dev.sh --user    # ~/.local/bin instead of /usr/local/bin (no sudo)
./scripts/install-dev.sh --restart # also pkill -x turm afterwards

# macOS
./scripts/install-macos.sh             # swift build -c release + ~/Applications/Turm.app + ~/.cargo/bin/turmctl (no sudo)
./scripts/install-macos.sh --system    # /Applications/Turm.app instead (sudo for /Applications)
./scripts/install-macos.sh --launch    # open the installed .app afterwards
```

Why these exist:
- **Linux**: `install.sh --system` puts turm at `/usr/local/bin/turm`. After that, `cargo build --release` only refreshes `target/release/turm` — the system binary stays at whatever Release version was last installed, so a fix in the working tree is silently shadowed when turm is launched via a desktop entry. The script also warns when `~/.local/bin/turm` and `/usr/local/bin/turm` are both present and differ.
- **macOS**: `cargo install turm-cli` fails (not on crates.io) and `cargo install --path .` fails from the repo root (workspace virtual manifest). The `turm` GUI app is SwiftPM, not cargo. Before this script, `turm-macos/run.sh` was the only path and it only built an ephemeral debug bundle under `.build/debug/`. The script wraps `swift build -c release` + bundle layout + `cargo install --path turm-cli` so the user gets a real `/Applications`-style install.

## Install first-party plugins

`install-dev.sh` runs `install-plugins.sh` automatically. To install plugins on their own (e.g. you only changed a plugin manifest):

```bash
./scripts/install-plugins.sh           # all plugins with a manifest
./scripts/install-plugins.sh todo git  # just these two
```

Plugins live in `examples/plugins/<name>/`; turm's runtime discovers them from `~/.config/turm/plugins/<name>/` at startup. The script copies the manifest + assets and symlinks the built binary into the plugin dir. `<plugin_dir>/<exec>` takes precedence over `PATH`, which matters because turm is often launched from a desktop entry whose env doesn't include `~/.local/bin`. After installing, **restart turm** — `discover_plugins()` only runs at startup. Symptom of an outdated install: `service X is not running and X.action cannot trigger its activation (OnStartup)` from the supervisor.

## Git Hooks

After cloning, enable the repo-tracked hooks once:

```bash
git config core.hooksPath .githooks
```

- `pre-commit` — runs `rustfmt --edition 2024` on the working-tree copy of every staged `.rs` file and re-stages each one. Aborts on syntax errors. Caveat: this does not honor partial staging — if you used `git add -p` on a `.rs` file, the formatted full file (including your unstaged edits) will be pulled into the commit. Stage the whole file or skip the hook (`git commit --no-verify`) for partial-stage workflows.
- `pre-push` — runs `cargo clippy --workspace --all-targets -- -D warnings`; blocks push on warnings. Stricter than CI's clippy step (CI omits `--all-targets`), but does **not** run CI's `fmt-check`/`test`/`build` steps — those can still fail in CI.

## Key Conventions

- Rust edition 2024, Cargo workspace with `resolver = "2"`
- GTK4 with `gnome_46` feature flag
- VTE handles PTY on Linux (no custom PTY management)
- Unix socket (`/tmp/turm-{PID}.sock`) for IPC
- Config: `~/.config/turm/config.toml` (TOML)
- Cache: `~/.cache/turm/wallpapers.txt`
- Theme: Catppuccin Mocha (hardcoded)
- Dark theme forced via GTK settings

## Critical Implementation Details

- **Background images**: Must call `terminal.set_clear_background(false)` for VTE transparency
- **GTK thread safety**: D-Bus → mpsc channel → glib::timeout_add_local polling
- **Binary names**: `turm` (app) and `turmctl` (CLI) — do not rename to collide
