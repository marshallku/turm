#!/bin/bash
set -e

echo "Building nestty..."
cargo build --release -p nestty-linux

echo "Installing binary..."
sudo install -Dm755 target/release/nestty /usr/local/bin/nestty

echo "Installing desktop entry..."
# Basename matches the GTK app_id (com.marshall.nestty) so Wayland
# compositors map the running window to this launcher entry.
sudo install -Dm644 nestty-linux/com.marshall.nestty.desktop \
    /usr/share/applications/com.marshall.nestty.desktop
# Remove a pre-rename "nestty.desktop" from older installs so the
# launcher does not show two duplicate entries.
sudo rm -f /usr/share/applications/nestty.desktop

echo "Installing hicolor icons..."
for size in 16 22 24 32 48 64 128 256 512; do
    sudo install -Dm644 \
        "nestty-linux/icons/hicolor/${size}x${size}/apps/nestty.png" \
        "/usr/share/icons/hicolor/${size}x${size}/apps/nestty.png"
done
# Refresh the icon cache so the desktop entry's Icon=nestty resolves
# without a logout. Best-effort — silently skip if the tool is missing.
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    sudo gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
fi

echo "Done. nestty is now available as a system terminal."
echo "You can set it as default with: gsettings set org.gnome.desktop.default-applications.terminal exec nestty"
