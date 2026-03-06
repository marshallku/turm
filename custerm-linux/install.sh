#!/bin/bash
set -e

echo "Building custerm..."
cargo build --release -p custerm-linux

echo "Installing binary..."
sudo install -Dm755 target/release/custerm /usr/local/bin/custerm

echo "Installing desktop entry..."
sudo install -Dm644 custerm-linux/custerm.desktop /usr/share/applications/custerm.desktop

echo "Done. custerm is now available as a system terminal."
echo "You can set it as default with: gsettings set org.gnome.desktop.default-applications.terminal exec custerm"
