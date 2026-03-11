#!/usr/bin/env bash
# Build and run turm-macos as a proper .app bundle
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

swift build

APP_DIR=".build/debug/Turm.app"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"

mkdir -p "$MACOS" "$RESOURCES"
cp .build/debug/Turm "$MACOS/Turm"

cat > "$CONTENTS/Info.plist" << 'EOF'
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

# Kill any running instance first so the rebuilt binary is used
pkill -x Turm 2>/dev/null || true
sleep 0.3

open -n "$APP_DIR"
