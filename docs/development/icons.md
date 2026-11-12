# Icon Generation

Application and tray icons are generated from two raster source files:

| Source | Purpose |
|--------|---------|
| `src/assets/app_logo.png` | App and bundle icons (window, desktop, Windows/macOS) |
| `src/assets/tray-small.png` | System tray icons |

## Prerequisites

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get install imagemagick
```

**macOS:**
```bash
brew install imagemagick
```

The macOS `.icns` step uses `png2icns` or `iconutil` if available; otherwise it
falls back to a built-in Python packer (no extra dependency).

## Usage

```bash
./generate-icons.sh
```

This generates all icons into `src-tauri/icons/`:

| Icon | Size | Purpose |
|------|------|---------|
| icon.png | 512x512 | Main application icon |
| 32x32.png | 32x32 | Small icon |
| 128x128.png | 128x128 | Medium icon |
| 128x128@2x.png | 256x256 | Retina medium icon |
| icon.ico | multi | Windows icon |
| icon.icns | multi | macOS icon |
| tray-icon.png | 32x32 | System tray |
| tray-icon@2x.png | 64x64 | Retina system tray |
| tray-icon-linux.png | 32x32 | Linux tray |
| tray-icon.ico | multi | Windows tray |
| Square*Logo.png | various | Windows Store logos |

## Updating the Logo

1. Replace `src/assets/app_logo.png` and/or `src/assets/tray-small.png`
2. Run `./generate-icons.sh`
3. All icons regenerate automatically

The About dialog and README branding use `src/assets/logo-wordmark.png` directly
(no regeneration step).
