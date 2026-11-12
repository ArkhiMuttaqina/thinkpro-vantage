#!/bin/bash

# Icon Generation Script
# Converts src/assets/app_logo.png (app/bundle icons) and
# src/assets/tray-small.png (tray icons) into every Tauri icon format.

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

resize "$SOURCE_APP" 30  "$ICON_DIR/Square30x30Logo.png"
resize "$SOURCE_APP" 44  "$ICON_DIR/Square44x44Logo.png"
resize "$SOURCE_APP" 50  "$ICON_DIR/Square50x50Logo.png"
resize "$SOURCE_APP" 71  "$ICON_DIR/Square71x71Logo.png"
resize "$SOURCE_APP" 89  "$ICON_DIR/Square89x89Logo.png"
resize "$SOURCE_APP" 107 "$ICON_DIR/Square107x107Logo.png"
resize "$SOURCE_APP" 142 "$ICON_DIR/Square142x142Logo.png"
resize "$SOURCE_APP" 150 "$ICON_DIR/Square150x150Logo.png"
resize "$SOURCE_APP" 284 "$ICON_DIR/Square284x284Logo.png"
resize "$SOURCE_APP" 310 "$ICON_DIR/Square310x310Logo.png"
resize "$SOURCE_APP" 50  "$ICON_DIR/StoreLogo.png"

"$CONVERT" "$SOURCE_APP" -background none -define icon:auto-resize=256,128,64,48,32,16 "$ICON_DIR/icon.ico"
echo "Generated: $ICON_DIR/icon.ico"

echo ""
echo "Generating tray icons from $SOURCE_TRAY..."
resize "$SOURCE_TRAY" 32 "$ICON_DIR/tray-icon.png"
resize "$SOURCE_TRAY" 64 "$ICON_DIR/tray-icon@2x.png"
"$CONVERT" "$SOURCE_TRAY" -background none -resize 32x32! -type TrueColorAlpha "PNG32:$ICON_DIR/tray-icon-linux.png"
echo "Generated: $ICON_DIR/tray-icon-linux.png (32x32 RGBA)"

"$CONVERT" "$SOURCE_TRAY" -background none -define icon:auto-resize=32,16 "$ICON_DIR/tray-icon.ico"
echo "Generated: $ICON_DIR/tray-icon.ico"

echo ""
if command -v png2icns &> /dev/null; then
    resize "$SOURCE_APP" 1024 "/tmp/icon-1024.png"
    png2icns "$ICON_DIR/icon.icns" /tmp/icon-1024.png
    rm /tmp/icon-1024.png
    echo "Generated: $ICON_DIR/icon.icns"
elif command -v iconutil &> /dev/null; then
    ICONSET_DIR="/tmp/icon.iconset"
    mkdir -p "$ICONSET_DIR"
    resize "$SOURCE_APP" 16   "$ICONSET_DIR/icon_16x16.png"
    resize "$SOURCE_APP" 32   "$ICONSET_DIR/icon_16x16@2x.png"
    resize "$SOURCE_APP" 32   "$ICONSET_DIR/icon_32x32.png"
    resize "$SOURCE_APP" 64   "$ICONSET_DIR/icon_32x32@2x.png"
    resize "$SOURCE_APP" 128  "$ICONSET_DIR/icon_128x128.png"
    resize "$SOURCE_APP" 256  "$ICONSET_DIR/icon_128x128@2x.png"
    resize "$SOURCE_APP" 256  "$ICONSET_DIR/icon_256x256.png"
    resize "$SOURCE_APP" 512  "$ICONSET_DIR/icon_256x256@2x.png"
    resize "$SOURCE_APP" 512  "$ICONSET_DIR/icon_512x512.png"
    resize "$SOURCE_APP" 1024 "$ICONSET_DIR/icon_512x512@2x.png"
    iconutil -c icns "$ICONSET_DIR" -o "$ICON_DIR/icon.icns"
    rm -rf "$ICONSET_DIR"
    echo "Generated: $ICON_DIR/icon.icns"
else
    # Fallback: pack a valid ICNS container from PNG chunks (macOS icon).
    python3 - "$SOURCE_APP" "$ICON_DIR/icon.icns" << 'PYEOF'
import struct, subprocess, os, tempfile, sys
src, out = sys.argv[1], sys.argv[2]
tmp = tempfile.mkdtemp(prefix="icns-")
sizes = {b"icp4": 16, b"icp5": 32, b"icp6": 64, b"ic07": 128, b"ic08": 256, b"ic09": 512, b"ic10": 1024}
body = b""
for otype, size in sizes.items():
    p = os.path.join(tmp, str(size) + ".png")
    subprocess.run(["magick", src, "-background", "none", "-resize", str(size) + "x" + str(size), p], check=True)
    data = open(p, "rb").read()
    body += otype + struct.pack(">I", 8 + len(data)) + data
open(out, "wb").write(b"icns" + struct.pack(">I", 8 + len(body)) + body)
print("Generated: " + out + " (" + str(len(body)) + " bytes)")
PYEOF
fi

echo ""
echo "Icon generation complete."
