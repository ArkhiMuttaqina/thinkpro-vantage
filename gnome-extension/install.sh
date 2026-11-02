#!/bin/bash
set -e

UUID="thinkutils@tysontranthai.site"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_SRC_DIR="$SCRIPT_DIR/$UUID"
EXT_DEST_DIR="$HOME/.local/share/gnome-shell/extensions/$UUID"

if [ "$1" = "--uninstall" ]; then
    echo "=== Uninstalling ThinkUtils GNOME Extension ==="
    if command -v gnome-extensions >/dev/null 2>&1; then
        gnome-extensions disable "$UUID" 2>/dev/null || true
    fi
    rm -rf "$EXT_DEST_DIR"
    echo "✓ Extension uninstalled from $EXT_DEST_DIR"
    echo "Please log out and log back in to finalize removal."
    exit 0
fi

echo "=== Installing ThinkUtils GNOME Shell Extension ==="
echo "Extension UUID: $UUID"

if [ ! -d "$EXT_SRC_DIR" ]; then
    echo "Error: Source directory $EXT_SRC_DIR not found." >&2
    exit 1
fi

# Ensure target directory exists
mkdir -p "$EXT_DEST_DIR"

# Copy extension files
cp -r "$EXT_SRC_DIR"/* "$EXT_DEST_DIR"/
echo "✓ Extension files copied to $EXT_DEST_DIR"

# Enable extension via gnome-extensions CLI if available
if command -v gnome-extensions >/dev/null 2>&1; then
    gnome-extensions enable "$UUID" 2>/dev/null || true
fi

# Ensure enabled in gsettings
CURRENT_ENABLED=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "@as []")
if [[ "$CURRENT_ENABLED" != *"$UUID"* ]]; then
    if [ "$CURRENT_ENABLED" = "@as []" ] || [ "$CURRENT_ENABLED" = "[]" ]; then
        gsettings set org.gnome.shell enabled-extensions "['$UUID']"
    else
        UPDATED=$(echo "$CURRENT_ENABLED" | sed "s/]/, '$UUID']/")
        gsettings set org.gnome.shell enabled-extensions "$UPDATED"
    fi
fi

echo "✓ Extension registered and enabled in GNOME Shell settings."
echo ""
echo "NOTE for Wayland sessions:"
echo "GNOME Shell requires a session restart (logout & login) to load newly installed extensions."
echo "Once logged back in, live CPU temperature and fan RPM will appear directly in the top panel."
