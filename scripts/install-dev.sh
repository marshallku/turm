#!/usr/bin/env bash
# Build + install the local working-tree nestty binaries + plugins
# in one shot. Companion to `install.sh` (which downloads from
# GitHub Releases for end users); this is the dev-iteration path
# for working on nestty itself.
#
# Why this exists: `install.sh --system` puts nestty at
# `/usr/local/bin/nestty`. After that, `cargo build --release` only
# refreshes `target/release/nestty` — `/usr/local/bin/nestty` stays at
# whatever version was last installed via Releases. That's how a
# stale system binary silently shadowed a freshly-built fix and
# wasted real debugging time. Run THIS script after every
# meaningful change so the GUI nestty and CLI nestctl on PATH stay
# in sync with the working tree.
#
# Usage:
#   ./scripts/install-dev.sh                # build + install everything
#   ./scripts/install-dev.sh --user         # install to ~/.local/bin (no sudo)
#   ./scripts/install-dev.sh --no-build     # skip cargo build (use existing target/release)
#   ./scripts/install-dev.sh --no-plugins   # skip the plugin install step
#   ./scripts/install-dev.sh --restart      # also `pkill -x nestty` afterwards
#
# By default this is a SYSTEM install (`/usr/local/bin`, sudo
# required) because that matches how `install.sh --system` lays
# things out — using `--user` instead is fine but won't override a
# pre-existing `/usr/local/bin/nestty` which takes PATH precedence.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="$REPO_ROOT/target/release"
DO_BUILD=true
DO_PLUGINS=true
DO_RESTART=false
USER_INSTALL=false

while [ "$#" -gt 0 ]; do
    case "$1" in
        --user)        USER_INSTALL=true ; shift ;;
        --no-build)    DO_BUILD=false ; shift ;;
        --no-plugins)  DO_PLUGINS=false ; shift ;;
        --restart)     DO_RESTART=true ; shift ;;
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

if $USER_INSTALL; then
    INSTALL_DIR="$HOME/.local/bin"
    DESKTOP_DIR="$HOME/.local/share/applications"
    ICON_BASE="$HOME/.local/share/icons/hicolor"
    SUDO=""
else
    INSTALL_DIR="/usr/local/bin"
    DESKTOP_DIR="/usr/share/applications"
    ICON_BASE="/usr/share/icons/hicolor"
    SUDO="sudo"
fi

if $DO_BUILD; then
    echo "==> cargo build --release --workspace"
    cargo build --release --workspace --manifest-path "$REPO_ROOT/Cargo.toml"
fi

for bin in nestty nestctl; do
    src="$TARGET/$bin"
    if [ ! -x "$src" ]; then
        echo "error: $src not built — run with default flags or 'cargo build --release'" >&2
        exit 1
    fi
done

echo "==> installing nestty + nestctl into $INSTALL_DIR"
if [ -n "$SUDO" ]; then
    # `install -m755` on existing files just rewrites; safe to repeat.
    $SUDO install -Dm755 "$TARGET/nestty" "$INSTALL_DIR/nestty"
    $SUDO install -Dm755 "$TARGET/nestctl" "$INSTALL_DIR/nestctl"
else
    mkdir -p "$INSTALL_DIR"
    install -Dm755 "$TARGET/nestty" "$INSTALL_DIR/nestty"
    install -Dm755 "$TARGET/nestctl" "$INSTALL_DIR/nestctl"
fi

echo "==> installing desktop entry + hicolor icons into ${DESKTOP_DIR%/applications} / $ICON_BASE"
# Desktop file basename matches the GTK app_id (com.marshall.nestty)
# so Wayland compositors can map the running window to this launcher
# entry. Without that mapping, the WM falls back to a generic icon
# and the StartupNotify cookie does not flow through.
$SUDO install -Dm644 \
    "$REPO_ROOT/nestty-linux/com.marshall.nestty.desktop" \
    "$DESKTOP_DIR/com.marshall.nestty.desktop"
# Cleanup: a pre-rename "nestty.desktop" lingering at the same dest
# would show up as a second, broken launcher entry.
$SUDO rm -f "$DESKTOP_DIR/nestty.desktop"
for size in 16 22 24 32 48 64 128 256 512; do
    $SUDO install -Dm644 \
        "$REPO_ROOT/nestty-linux/icons/hicolor/${size}x${size}/apps/nestty.png" \
        "$ICON_BASE/${size}x${size}/apps/nestty.png"
done
# Refresh the icon cache so launchers pick up Icon=nestty without a logout.
# gtk-update-icon-cache is in libgtk-4 / gtk4 packages — present on any
# system that already builds nestty, so the missing-binary branch is rare.
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    $SUDO gtk-update-icon-cache -q -t "$ICON_BASE" || true
fi

# Sanity: user might have BOTH ~/.local/bin/nestty and
# /usr/local/bin/nestty. PATH typically prefers /usr/local/bin, so
# if they're out of sync the user gets the wrong one. We can't
# auto-fix without making policy decisions about which copy to
# trust, but we can flag the drift so the next "why isn't my fix
# applied?" debug session is shorter. Concrete remedies the user
# can pick from are listed in the warning.
if [ -x "$HOME/.local/bin/nestty" ] && [ -x "/usr/local/bin/nestty" ]; then
    if ! cmp -s "$HOME/.local/bin/nestty" "/usr/local/bin/nestty"; then
        echo
        echo "warn: ~/.local/bin/nestty and /usr/local/bin/nestty differ." >&2
        echo "warn: PATH lookup typically picks /usr/local/bin first;" >&2
        echo "warn: a desktop-entry-launched nestty will use the system copy." >&2
        echo "warn: to resolve, pick one of:" >&2
        echo "warn:   - re-run WITHOUT --user (overwrites the system copy with the same build)" >&2
        echo "warn:   - sudo rm /usr/local/bin/nestty (let the user-local copy win)" >&2
        echo "warn:   - sudo rm $HOME/.local/bin/nestty (drop the user-local copy entirely)" >&2
        echo
    fi
fi

if $DO_PLUGINS; then
    echo "==> installing first-party plugin manifests + binary symlinks"
    bash "$REPO_ROOT/scripts/install-plugins.sh"
fi

if $DO_RESTART; then
    echo "==> pkill -x nestty (you'll need to relaunch via desktop entry / shell)"
    pkill -x nestty 2>/dev/null || true
else
    echo
    echo "Restart nestty to pick up the new binary:"
    echo "  pkill -x nestty"
    echo "  # then relaunch via your usual path (desktop entry / shell)"
fi
