#!/bin/bash
set -e

echo "Building turm..."
cargo build --release -p turm-linux

echo "Installing binary..."
sudo install -Dm755 target/release/turm /usr/local/bin/turm

echo "Installing desktop entry..."
sudo install -Dm644 turm-linux/turm.desktop /usr/share/applications/turm.desktop

echo "Done. turm is now available as a system terminal."
echo "You can set it as default with: gsettings set org.gnome.desktop.default-applications.terminal exec turm"
