#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SCRIPT_DIR"

echo "=== Installing ThinkUtils to User Applications ==="

# Build release if not present
if [ ! -f "src-tauri/target/release/thinkutils" ]; then
    echo "Release binary not found. Building production release..."
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    fi
    export PKG_CONFIG_PATH="/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
    npm run build
fi

# Ensure user directories exist
mkdir -p "$HOME/.local/bin" \
         "$HOME/.local/share/applications" \
         "$HOME/.local/share/icons/hicolor/32x32/apps" \
         "$HOME/.local/share/icons/hicolor/128x128/apps" \
         "$HOME/.local/share/icons/hicolor/512x512/apps" \
         "$HOME/.local/share/icons/hicolor/scalable/apps"

# Terminate any running instance before replacing binary
pkill -x thinkutils 2>/dev/null || true

# 1. Install application binary
install -m 755 "src-tauri/target/release/thinkutils" "$HOME/.local/bin/thinkutils"
echo "✓ Installed executable: $HOME/.local/bin/thinkutils"

# 2. Install desktop launcher
rm -f "$HOME/.local/share/applications/com.vietanhnv.thinkutils.desktop"
cp "thinkutils.desktop" "$HOME/.local/share/applications/thinkutils.desktop"
sed -i "s|^Exec=.*|Exec=$HOME/.local/bin/thinkutils|" "$HOME/.local/share/applications/thinkutils.desktop"
sed -i "s|^Icon=.*|Icon=$HOME/.local/share/icons/thinkutils.png|" "$HOME/.local/share/applications/thinkutils.desktop"
echo "✓ Installed desktop entry: $HOME/.local/share/applications/thinkutils.desktop"

# 3. Install icons
cp "src-tauri/icons/icon.png" "$HOME/.local/share/icons/thinkutils.png"
cp "src-tauri/icons/32x32.png" "$HOME/.local/share/icons/hicolor/32x32/apps/thinkutils.png"
cp "src-tauri/icons/128x128.png" "$HOME/.local/share/icons/hicolor/128x128/apps/thinkutils.png"
cp "src-tauri/icons/icon.png" "$HOME/.local/share/icons/hicolor/512x512/apps/thinkutils.png"
cp "src/assets/logo.svg" "$HOME/.local/share/icons/hicolor/scalable/apps/thinkutils.svg"
echo "✓ Installed desktop application icons"

# 4. Refresh desktop and icon database caches
update-desktop-database "$HOME/.local/share/applications/" 2>/dev/null || true
gtk-update-icon-cache "$HOME/.local/share/icons/hicolor/" 2>/dev/null || true

# 5. Install GNOME Shell extension
if [ -f "gnome-extension/install.sh" ]; then
    ./gnome-extension/install.sh || true
fi

echo ""
echo "🎉 ThinkUtils installation completed!"
echo "You can now launch ThinkUtils by:"
echo "  1. Searching for 'ThinkUtils' in your application menu (Super / Windows key)"
echo "  2. Running 'thinkutils' in any terminal"
echo "  3. Clicking 'Open ThinkUtils' from the GNOME top bar indicator"
