#!/bin/bash
set -e

# Load Rust environment
if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi

# Ensure pkg-config finds Debian system development libraries
export PKG_CONFIG_PATH="/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"

cd "$(dirname "$0")"

echo "=== Launching ThinkUtils (Development / Testing) ==="
npm run tauri dev
