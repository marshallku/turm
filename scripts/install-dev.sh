#!/usr/bin/env bash
# Build + install the local working-tree turm binaries + plugins
# in one shot. Companion to `install.sh` (which downloads from
# GitHub Releases for end users); this is the dev-iteration path
# for working on turm itself.
#
# Why this exists: `install.sh --system` puts turm at
# `/usr/local/bin/turm`. After that, `cargo build --release` only
# refreshes `target/release/turm` — `/usr/local/bin/turm` stays at
# whatever version was last installed via Releases. That's how a
# stale system binary silently shadowed a freshly-built fix and
# wasted real debugging time. Run THIS script after every
# meaningful change so the GUI turm and CLI turmctl on PATH stay
# in sync with the working tree.
#
# Usage:
#   ./scripts/install-dev.sh                # build + install everything
#   ./scripts/install-dev.sh --user         # install to ~/.local/bin (no sudo)
#   ./scripts/install-dev.sh --no-build     # skip cargo build (use existing target/release)
#   ./scripts/install-dev.sh --no-plugins   # skip the plugin install step
#   ./scripts/install-dev.sh --restart      # also `pkill -x turm` afterwards
#
# By default this is a SYSTEM install (`/usr/local/bin`, sudo
# required) because that matches how `install.sh --system` lays
# things out — using `--user` instead is fine but won't override a
# pre-existing `/usr/local/bin/turm` which takes PATH precedence.
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
    SUDO=""
else
    INSTALL_DIR="/usr/local/bin"
    SUDO="sudo"
fi

if $DO_BUILD; then
    echo "==> cargo build --release --workspace"
    cargo build --release --workspace --manifest-path "$REPO_ROOT/Cargo.toml"
fi

for bin in turm turmctl; do
    src="$TARGET/$bin"
    if [ ! -x "$src" ]; then
        echo "error: $src not built — run with default flags or 'cargo build --release'" >&2
        exit 1
    fi
done

echo "==> installing turm + turmctl into $INSTALL_DIR"
if [ -n "$SUDO" ]; then
    # `install -m755` on existing files just rewrites; safe to repeat.
    $SUDO install -Dm755 "$TARGET/turm" "$INSTALL_DIR/turm"
    $SUDO install -Dm755 "$TARGET/turmctl" "$INSTALL_DIR/turmctl"
else
    mkdir -p "$INSTALL_DIR"
    install -Dm755 "$TARGET/turm" "$INSTALL_DIR/turm"
    install -Dm755 "$TARGET/turmctl" "$INSTALL_DIR/turmctl"
fi

# Sanity: user might have BOTH ~/.local/bin/turm and
# /usr/local/bin/turm. PATH typically prefers /usr/local/bin, so
# if they're out of sync the user gets the wrong one. Warn loudly
# when we detect a drift between the two so the next "why isn't
# my fix applied?" debug session is shorter.
if [ -x "$HOME/.local/bin/turm" ] && [ -x "/usr/local/bin/turm" ]; then
    if ! cmp -s "$HOME/.local/bin/turm" "/usr/local/bin/turm"; then
        echo
        echo "warn: ~/.local/bin/turm and /usr/local/bin/turm differ." >&2
        echo "warn: PATH lookup typically picks /usr/local/bin first; the user-local copy is shadowed." >&2
        echo "warn: re-run with --user OR delete one to avoid silent version drift." >&2
        echo
    fi
fi

if $DO_PLUGINS; then
    echo "==> installing first-party plugin manifests + binary symlinks"
    bash "$REPO_ROOT/scripts/install-plugins.sh"
fi

if $DO_RESTART; then
    echo "==> pkill -x turm (you'll need to relaunch via desktop entry / shell)"
    pkill -x turm 2>/dev/null || true
else
    echo
    echo "Restart turm to pick up the new binary:"
    echo "  pkill -x turm"
    echo "  # then relaunch via your usual path (desktop entry / shell)"
fi
