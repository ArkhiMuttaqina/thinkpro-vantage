#!/bin/bash
set -e

echo "=== ThinkPro Vantage Upgrade ==="
echo ""

# Debian-family only: this script builds a .deb and installs it with dpkg. On an
# Arch-family machine the package comes from the PKGBUILD instead, so say that
# rather than failing with "dpkg: command not found" after a full release build.
if ! command -v dpkg >/dev/null 2>&1; then
    echo "dpkg is not installed: this is not a Debian-family system."
    echo ""
    echo "Build and install the Arch package instead -- and regenerate the source"
    echo "tarball first, or it will not carry your current working tree:"
    echo ""
    echo "  cd ~/build/tpv-pkg && makepkg -f && sudo pacman -U thinkpro-vantage-*.pkg.tar.zst"
    exit 1
fi

# Build the .deb only. A bare `tauri build` also bundles an AppImage, and that
# step shells out to linuxdeploy, whose failure aborts this script with `set -e`
# before the .deb it already produced is ever installed. Nothing here uses the
# AppImage, and skipping it also cuts the build time noticeably.
echo "Building..."
npm run tauri build -- --bundles deb

# Find the .deb
DEB=$(ls -t src-tauri/target/release/bundle/deb/*.deb 2>/dev/null | head -1)

if [ -z "$DEB" ]; then
    echo "Error: No .deb package found"
    exit 1
fi

VERSION=$(basename "$DEB" | grep -oP '\d+\.\d+\.\d+')
echo "Installing ThinkPro Vantage v${VERSION}..."
echo ""

sudo dpkg -i "$DEB"

echo ""
echo "✓ ThinkPro Vantage v${VERSION} installed successfully!"
echo "  Restart the app to use the new version."
