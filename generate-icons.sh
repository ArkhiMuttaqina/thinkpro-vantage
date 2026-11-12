#!/bin/bash

# Icon Generation Script (Linux-only)
# Converts src/assets/app_logo.png (app/bundle icons) and
# src/assets/tray-small.png (tray icons) into the Linux icon set.

set -e

ICON_DIR="src-tauri/icons"
SOURCE_APP="src/assets/app_logo.png"
SOURCE_TRAY="src/assets/tray-small.png"

if ! command -v magick &> /dev/null && ! command -v convert &> /dev/null; then
    echo "Error: ImageMagick (magick/convert) is not installed."
    exit 1
fi

CONVERT="magick"
if ! command -v magick &> /dev/null; then
    CONVERT="convert"
fi

mkdir -p "$ICON_DIR"

resize() {
    local src=$1 size=$2 out=$3
    "$CONVERT" "$src" -background none -resize "${size}x${size}" "$out"
    echo "Generated: $out (${size}x${size})"
}

echo "Generating app icons from $SOURCE_APP..."
resize "$SOURCE_APP" 32  "$ICON_DIR/32x32.png"
resize "$SOURCE_APP" 128 "$ICON_DIR/128x128.png"
resize "$SOURCE_APP" 256 "$ICON_DIR/128x128@2x.png"
resize "$SOURCE_APP" 512 "$ICON_DIR/icon.png"

echo ""
echo "Generating tray icons from $SOURCE_TRAY..."
resize "$SOURCE_TRAY" 32 "$ICON_DIR/tray-icon.png"
resize "$SOURCE_TRAY" 64 "$ICON_DIR/tray-icon@2x.png"
"$CONVERT" "$SOURCE_TRAY" -background none -resize 32x32! -type TrueColorAlpha "PNG32:$ICON_DIR/tray-icon-linux.png"
echo "Generated: $ICON_DIR/tray-icon-linux.png (32x32 RGBA)"

echo ""
echo "Icon generation complete."
