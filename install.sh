#!/usr/bin/env bash
# Builds minnamate and installs it for the current user:
# ~/.local/bin/minnamate, its icon, and a .desktop launcher.
set -euo pipefail

cd "$(dirname "$0")"

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"

cargo build --release -p minnamate-editor

install -Dm755 target/release/minnamate "$BIN_DIR/minnamate"
install -Dm644 assets/logo.png "$ICON_DIR/minnamate.png"

mkdir -p "$APP_DIR"
cat > "$APP_DIR/minnamate.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=minnamate
Comment=Open-source 2D animation editor
Exec=$BIN_DIR/minnamate %f
Icon=minnamate
Terminal=false
Categories=Graphics;2DGraphics;VectorGraphics;
MimeType=application/vnd.adobe.fla;
StartupWMClass=minnamate
EOF

update-desktop-database "$APP_DIR" 2>/dev/null || true
gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Installed $BIN_DIR/minnamate"
