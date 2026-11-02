# ThinkUtils GNOME Shell Indicator Extension

A native GNOME Shell top panel indicator designed for Lenovo ThinkPad laptops on Linux.

## Features

- **Live Top Panel Display**: Shows real-time CPU temperature (°C) and fan speed (RPM) directly in the Linux system top bar.
- **Color Alerts**: Temperature turns amber above 70°C and red above 85°C.
- **Dropdown Quick Menu**:
  - Detailed CPU temperature reading.
  - Fan speed and current fan mode (`auto`, `level 4`, `level 7`, `level full-speed`, etc.).
  - Battery charge level and AC adapter status.
  - One-click fan speed profile switching via `thinkutils-fan-control`.
  - Quick launcher to open the ThinkUtils desktop app.

## Supported GNOME Versions

- GNOME 45, 46, 47, 48 (ESM extension format).

## Installation

Run the included install script:

```bash
./install.sh
```

Or install manually:

```bash
mkdir -p ~/.local/share/gnome-shell/extensions/thinkutils@tysontranthai.site
cp -r thinkutils@tysontranthai.site/* ~/.local/share/gnome-shell/extensions/thinkutils@tysontranthai.site/
gnome-extensions enable thinkutils@tysontranthai.site
```

### Note for Wayland Users

On Wayland, GNOME Shell only detects newly installed extension directories upon starting a session. **Please log out and log back in** (or reboot) for the extension to appear in your top bar.

## Uninstallation

```bash
./install.sh --uninstall
```
