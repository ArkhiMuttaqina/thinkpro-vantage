# Icon Generation

Application and tray icons are generated from two raster source files:

| Source | Purpose |
|--------|---------|
| `src/assets/app_logo.png` | Application and bundle icons |
| `src/assets/tray-small.png` | System tray icons |

## Prerequisites

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get install imagemagick
```

## Usage

```bash
./generate-icons.sh
```

This generates the Linux icon set into `src-tauri/icons/`:

| Icon | Size | Purpose |
|------|------|---------|
| icon.png | 512x512 | Main application icon |
| 32x32.png | 32x32 | Small icon |
| 128x128.png | 128x128 | Medium icon |
| 128x128@2x.png | 256x256 | Retina medium icon |
| tray-icon.png | 32x32 | System tray |
| tray-icon@2x.png | 64x64 | Retina system tray |
| tray-icon-linux.png | 32x32 | Linux tray |

## Updating the Logo

1. Replace `src/assets/app_logo.png` and/or `src/assets/tray-small.png`
2. Run `./generate-icons.sh`
3. All icons regenerate automatically

The About dialog and README branding use `src/assets/logo-wordmark.png` directly
(no regeneration step).
