#!/bin/bash
# End-user install script: downloads turm + turmctl from the
# latest GitHub Release tag and lays them out at ~/.local/bin (or
# /usr/local/bin with --system).
#
# For LOCAL DEVELOPMENT iteration on the working tree, use
# scripts/install-dev.sh instead — that one builds from source
# AND keeps system + user binaries from drifting against each
# other (a stale system binary silently shadowing a working-tree
# fix is a real failure mode this script can't avoid).
#
# Plugin binaries (turm-plugin-*) are NOT in the release tarball
# yet; if you want plugins, install them separately via
# scripts/install-plugins.sh after running install-dev.sh, OR
# build from source.
set -euo pipefail

REPO="marshallku/turm"
INSTALL_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
TARGET_VERSION=""
SYSTEM_INSTALL=false

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Install turm from GitHub Releases."
    echo ""
    echo "Options:"
    echo "  --version VERSION    Install a specific version (e.g., v0.1.0)"
    echo "  --system             Install to /usr/local/bin (requires sudo)"
    echo "  -h, --help           Show this help message"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            TARGET_VERSION="$2"
            shift 2
            ;;
        --system)
            SYSTEM_INSTALL=true
            INSTALL_DIR="/usr/local/bin"
            DESKTOP_DIR="/usr/share/applications"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

ARCH="$(uname -m)"
if [[ "${ARCH}" != "x86_64" ]]; then
    echo "Error: unsupported architecture '${ARCH}'. Only x86_64 is supported."
    exit 1
fi

check_deps() {
    local missing=()
    pkg-config --exists gtk4 2>/dev/null || missing+=("gtk4")
    pkg-config --exists vte-2.91-gtk4 2>/dev/null || missing+=("vte4 (libvte-2.91-gtk4)")
    pkg-config --exists webkitgtk-6.0 2>/dev/null || missing+=("webkitgtk-6.0")
    pkg-config --exists gstreamer-1.0 2>/dev/null || missing+=("gst-plugins-good gst-plugins-bad")
    if [[ ${#missing[@]} -gt 0 ]]; then
        echo "Warning: missing system dependencies: ${missing[*]}"
        echo "turm requires these libraries to run. Install them via your package manager."
    fi
}

if [[ -n "${TARGET_VERSION}" ]]; then
    VERSION="${TARGET_VERSION}"
    API_URL="https://api.github.com/repos/${REPO}/releases/tags/${VERSION}"
else
    API_URL="https://api.github.com/repos/${REPO}/releases/latest"
fi

echo "Fetching release info..."
RELEASE_JSON="$(curl -fsSL "${API_URL}")"
VERSION="$(echo "${RELEASE_JSON}" | grep -m1 '"tag_name"' | cut -d'"' -f4)"

if [[ -z "${VERSION}" ]]; then
    echo "Error: could not determine release version."
    exit 1
fi

echo "Installing turm ${VERSION}..."

ASSET_NAME="turm-${VERSION}-x86_64-linux.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET_NAME}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

echo "Downloading ${ASSET_NAME}..."
curl -fsSL -o "${TMPDIR}/turm.tar.gz" "${DOWNLOAD_URL}"

echo "Extracting..."
tar -xzf "${TMPDIR}/turm.tar.gz" -C "${TMPDIR}"

if ${SYSTEM_INSTALL}; then
    echo "Installing to ${INSTALL_DIR} (requires sudo)..."
    sudo install -Dm755 "${TMPDIR}/turm" "${INSTALL_DIR}/turm"
    sudo install -Dm755 "${TMPDIR}/turmctl" "${INSTALL_DIR}/turmctl"
    sudo install -Dm644 "${TMPDIR}/turm.desktop" "${DESKTOP_DIR}/turm.desktop"
else
    mkdir -p "${INSTALL_DIR}" "${DESKTOP_DIR}"
    install -m755 "${TMPDIR}/turm" "${INSTALL_DIR}/turm"
    install -m755 "${TMPDIR}/turmctl" "${INSTALL_DIR}/turmctl"
    install -m644 "${TMPDIR}/turm.desktop" "${DESKTOP_DIR}/turm.desktop"
fi

check_deps

if ! echo "${PATH}" | tr ':' '\n' | grep -qx "${INSTALL_DIR}"; then
    echo ""
    echo "Warning: ${INSTALL_DIR} is not in your PATH."
    echo "Add it to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\${PATH}\""
fi

echo ""
echo "turm ${VERSION} installed successfully!"
echo "  turm    -> ${INSTALL_DIR}/turm"
echo "  turmctl -> ${INSTALL_DIR}/turmctl"
