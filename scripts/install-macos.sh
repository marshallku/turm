#!/usr/bin/env bash
# scripts/install-macos.sh — Build + install turm-macos as a real .app
# and install turmctl via `cargo install --path turm-cli`.
#
# Companion to scripts/install-dev.sh (which is Linux-only — it does
# `cargo build --workspace`, and the workspace contains turm-linux which
# does not build on macOS without GTK4).
#
# Why this script exists:
#   - The macOS GUI app builds via SwiftPM in turm-macos/, not cargo.
#     Up to now, turm-macos/run.sh was the only path, and it builds an
#     ephemeral debug bundle under .build/debug/ and `open -n`s it. There
#     was no way to install turm as a real /Applications app.
#   - `cargo install turm-cli` (crates.io) fails — the package is not
#     published. `cargo install --path .` from the repo root also fails
#     because the root manifest is a workspace, not a package. The
#     correct invocation is `cargo install --path turm-cli`, which this
#     script wraps so the user does not need to memorize it.
#
# Usage:
#   ./scripts/install-macos.sh              # ~/Applications + ~/.cargo/bin (no sudo)
#   ./scripts/install-macos.sh --system     # /Applications + ~/.cargo/bin (sudo for /Applications)
#   ./scripts/install-macos.sh --no-build   # skip swift build (use existing .build/release/Turm)
#   ./scripts/install-macos.sh --no-turmctl # skip cargo install of turmctl
#   ./scripts/install-macos.sh --no-plugins # skip building/installing plugin binaries
#   ./scripts/install-macos.sh --launch     # open the installed app afterwards
#
# Notes:
#   - turmctl always goes to ~/.cargo/bin (cargo install's default). If you
#     want it in /usr/local/bin, run `sudo install -m755 \\
#     ~/.cargo/bin/turmctl /usr/local/bin/turmctl` after this script.
#   - This script kills any running Turm instance so the binary can be
#     replaced. macOS holds an exclusive lock on a running .app's exec.
#   - First launch may show Gatekeeper warning if the .app is unsigned;
#     right-click → Open once, or `xattr -d com.apple.quarantine` (only
#     applies to downloaded apps; locally-built bundles do not carry the
#     quarantine xattr).

set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "this script is macOS-only; on Linux use scripts/install-dev.sh" >&2
    exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Turm.app"
DO_BUILD=true
SYSTEM_INSTALL=false
DO_TURMCTL=true
DO_PLUGINS=true
DO_LAUNCH=false

# macOS-buildable plugins. KB / Todo / Bookmark are excluded because
# they depend on Linux-only filesystem primitives (renameat2, O_NOFOLLOW)
# and won't compile — see codex round-2 finding in docs/macos-parity-plan.md.
# Slack / Calendar / LLM / Discord are Unix-gated and theoretically build
# on macOS but their Keychain/auth UX hasn't been verified yet (PR 5+).
# PR 4 added git (no platform-specific deps, just argv shell-out to /usr/bin/git).
MACOS_PLUGINS=(echo git)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --system)      SYSTEM_INSTALL=true ; shift ;;
        --no-build)    DO_BUILD=false ; shift ;;
        --no-turmctl)  DO_TURMCTL=false ; shift ;;
        --no-plugins)  DO_PLUGINS=false ; shift ;;
        --launch)      DO_LAUNCH=true ; shift ;;
        -h|--help)
            sed -n '2,/^set -euo/p' "$0" | grep -E '^# ' | sed 's/^# \?//'
            exit 0
            ;;
        *)
            echo "unknown flag: $1" >&2
            exit 2
            ;;
    esac
done

if $SYSTEM_INSTALL; then
    APP_DEST="/Applications"
    SUDO_APP="sudo"
else
    APP_DEST="$HOME/Applications"
    SUDO_APP=""
fi

# 1. Build the macOS app via SwiftPM (release config).
#    The Turm executable links libturm_ffi.a from the Rust staticlib crate;
#    SwiftPM cannot run cargo as a prebuild step from Package.swift, so we
#    invoke cargo here first. swift build's linker phase then picks up the
#    archive at $REPO_ROOT/target/release/libturm_ffi.a via the
#    -L../target/release flag baked into Package.swift.
if $DO_BUILD; then
    echo "==> cargo build --release -p turm-ffi (Rust staticlib for Swift FFI)"
    (cd "$REPO_ROOT" && cargo build --release -p turm-ffi)

    echo "==> swift build -c release (turm-macos)"
    (cd "$REPO_ROOT/turm-macos" && swift build -c release)
fi

BUILT_BIN="$REPO_ROOT/turm-macos/.build/release/Turm"
if [[ ! -x "$BUILT_BIN" ]]; then
    echo "error: $BUILT_BIN not found — drop --no-build, or run swift build -c release in turm-macos/" >&2
    exit 1
fi

# 2. Stop any running instance so we can replace the bundle's executable.
pkill -x Turm 2>/dev/null || true
sleep 0.3

# 3. Stage the bundle in a tmp dir so the install is atomic — the user
#    never sees a half-written .app at $APP_DEST.
STAGING_DIR="$(mktemp -d)"
trap 'rm -rf "$STAGING_DIR"' EXIT
STAGING="$STAGING_DIR/$APP_NAME"
CONTENTS="$STAGING/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
mkdir -p "$MACOS" "$RESOURCES"
cp "$BUILT_BIN" "$MACOS/Turm"

# Info.plist — kept in sync with turm-macos/run.sh by hand. Two copies is
# acceptable (Rule of Three); a third would mean extracting to a template.
cat > "$CONTENTS/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>Turm</string>
    <key>CFBundleIdentifier</key>
    <string>com.marshall.turm</string>
    <key>CFBundleName</key>
    <string>turm</string>
    <key>CFBundleDisplayName</key>
    <string>turm</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
EOF

# 4. Install — replace any prior bundle in one rename so a partially-failed
#    install never leaves $APP_DEST in a broken state.
echo "==> installing $APP_NAME to $APP_DEST"
mkdir -p "$APP_DEST" 2>/dev/null || $SUDO_APP mkdir -p "$APP_DEST"
$SUDO_APP rm -rf "$APP_DEST/$APP_NAME"
$SUDO_APP mv "$STAGING" "$APP_DEST/$APP_NAME"

# 5. Install turmctl via cargo install (writes to ~/.cargo/bin). This
#    is the canonical CLI install path on macOS — `cargo install turm-cli`
#    fails (not on crates.io) and `cargo install --path .` fails (workspace
#    root is a virtual manifest), so we wrap the correct invocation here.
if $DO_TURMCTL; then
    echo "==> cargo install --path turm-cli (turmctl → ~/.cargo/bin)"
    cargo install --path "$REPO_ROOT/turm-cli"
fi

# 6. Build + install macOS-buildable plugins. PluginSupervisor (PR 3) reads
#    ~/Library/Application Support/turm/plugins/<name>/ at startup; we
#    cargo-build the binary and copy the manifest. Manifest's
#    `services.exec` is resolved against the plugin dir first, so we drop
#    the binary alongside plugin.toml so the supervisor finds it without
#    a $PATH dance.
PLUGIN_DEST="$HOME/Library/Application Support/turm/plugins"
if $DO_PLUGINS; then
    mkdir -p "$PLUGIN_DEST"
    for name in "${MACOS_PLUGINS[@]}"; do
        crate="turm-plugin-$name"
        src_manifest="$REPO_ROOT/examples/plugins/$name/plugin.toml"
        if [[ ! -f "$src_manifest" ]]; then
            echo "skip plugin $name: $src_manifest missing"
            continue
        fi
        echo "==> cargo build --release -p $crate"
        (cd "$REPO_ROOT" && cargo build --release -p "$crate")

        bin_src="$REPO_ROOT/target/release/$crate"
        if [[ ! -x "$bin_src" ]]; then
            echo "warn  plugin $name: binary $bin_src not built — skipping" >&2
            continue
        fi

        plugin_dir="$PLUGIN_DEST/$name"
        mkdir -p "$plugin_dir"
        # Copy every loose file next to plugin.toml (manifest + panel.html
        # if any) so panel-bearing plugins land complete.
        find "$REPO_ROOT/examples/plugins/$name" -maxdepth 1 -type f \
            -exec cp -f {} "$plugin_dir/" \;
        # Copy (don't symlink) the binary so a `git clean` of target/ doesn't
        # silently break the install. Cheap — these binaries are small.
        cp -f "$bin_src" "$plugin_dir/$crate"
        chmod 755 "$plugin_dir/$crate"
        echo "ok    plugin $name → $plugin_dir/"
    done
fi

if $DO_LAUNCH; then
    open "$APP_DEST/$APP_NAME"
fi

cat <<EOF

Installed:
  $APP_DEST/$APP_NAME
EOF
if $DO_TURMCTL; then
    echo "  $HOME/.cargo/bin/turmctl"
fi
if $DO_PLUGINS; then
    echo "  $PLUGIN_DEST/{$(IFS=,; echo "${MACOS_PLUGINS[*]}")}"
fi
cat <<'EOF'

Next:
  - Launch turm via Spotlight, Launchpad, or `open -a turm`.
  - Generate a default config: `turmctl --init-config`-equivalent does
    not exist on macOS yet; create ~/.config/turm/config.toml manually
    or copy from examples/config.toml.
  - Verify a plugin is alive: `turmctl call echo.ping --params '{"hi":"there"}'`
EOF
