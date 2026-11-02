<div align="center">

# ⚡ ThinkUtils

### The Ultimate ThinkPad Companion Utility for Linux & Windows

**Hardware Fan Curves • Dual-Battery Thresholds • CPU Governors • Native PAM Biometrics • AI Model Context Protocol (MCP)**

<p align="center">
  <a href="https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded/actions/workflows/ci.yml">
    <img src="https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded/actions/workflows/ci.yml/badge.svg" alt="CI Status" />
  </a>
  <a href="https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded/releases">
    <img src="https://img.shields.io/github/v/release/TysonTranThai/ThinkUtils-Maintained-Upgraded?color=blue&label=release" alt="Latest Release" />
  </a>
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows-informational?logo=linux&logoColor=white" alt="Platform" />
  <img src="https://img.shields.io/badge/Built%20with-Tauri%20v2%20%2B%20Rust-orange?logo=tauri&logoColor=white" alt="Tauri + Rust" />
  <a href="./LICENSE">
    <img src="https://img.shields.io/badge/License-LGPL%20v3-green" alt="License" />
  </a>
  <a href="https://thinkutils.tysontranthai.site">
    <img src="https://img.shields.io/badge/Docs-thinkutils.tysontranthai.site-blue" alt="Documentation" />
  </a>
</p>

<p align="center">
  <a href="#-key-features">Key Features</a> •
  <a href="#-ui-showcase">UI Showcase</a> •
  <a href="#-installation">Installation</a> •
  <a href="#-thinkpad-hardware-setup">Hardware Setup</a> •
  <a href="#-pam-security--health-cli">PAM CLI</a> •
  <a href="#-ai-integration-mcp-server">MCP Server</a> •
  <a href="#-building-from-source">Development</a>
</p>

---

<img src="./screenshots/home.png" alt="ThinkUtils Home Dashboard" width="900" style="border-radius: 8px; box-shadow: 0 4px 20px rgba(0,0,0,0.3);" />

</div>

<br/>

## 📖 Overview

**ThinkUtils** is a high-performance, native desktop utility tailored specifically for IBM & Lenovo ThinkPad hardware. Powered by a lightweight **Rust** backend and **Tauri v2**, ThinkUtils gives you granular control over thermal fan curves, charge thresholds, CPU governors, and system telemetry—all packaged in a sleek dark interface inspired by ThinkPad's iconic design language.

Whether you need whisper-quiet operation in a quiet library, maximum thermal dissipation during heavy compilation, or prolonged battery lifespan on the go, ThinkUtils gives you full command of your hardware without clunky scripts or terminal juggling.

---

## ✨ Key Features

| Feature | Description |
| :--- | :--- |
| 🌀 **Intelligent Fan Control** | Direct hardware EC interface via `thinkpad_acpi`. Custom interactive temperature-to-RPM curves, auto/manual steps (0–7), full blast, and automated safety watchdog re-arming. |
| 🔋 **Battery Health & Thresholds** | Prolong battery life using the 40–80% charging cycle rule. Set custom start/stop charge limits with multi-battery support, degradation metrics, and cycle counts. |
| ⚡ **Performance & CPU Profiles** | Live scaling policy switching (`performance`, `powersave`, `schedutil`, `ondemand`), Intel Turbo Boost toggle, Energy Performance Preference (EPP), and per-core clock monitoring. |
| 📊 **Real-Time System Telemetry** | Comprehensive live dashboards for per-core CPU loads, memory & swap allocation, storage mount points, network interface throughput, and top active processes. |
| 🛡️ **Biometrics & Security Suite** | Native fingerprint enrollment, ClamAV antivirus scanning, and built-in PAM health verification & recovery tooling (`--check-pam`, `--repair-pam`, `--rollback-pam`). |
| 🤖 **AI Model Context Protocol (MCP)** | Built-in SSE MCP server allowing AI assistants (Claude Code, Claude Desktop, Cursor) to inspect telemetry, adjust fan curves, and tune performance parameters safely. |
| 🪟 **Cross-Platform Readiness** | Universal Linux packages (`.deb`, `.rpm`, `.AppImage`) and standalone native Windows 10/11 executables and installers. |
| ☁️ **Settings Sync & Backup** | Export and import your finely tuned configurations to JSON, or back them up seamlessly to Google Drive. |

---

## 📸 UI Showcase

<div align="center">

| 🌀 Fan Control & Curve Editor | 🔋 Dual-Battery Management |
| :---: | :---: |
| <img src="./screenshots/fan_control.png" width="440" alt="Fan Control" /> | <img src="./screenshots/battery.png" width="440" alt="Battery Management" /> |
| **⚡ CPU Performance Tuning** | **📊 System Resources & Processes** |
| <img src="./screenshots/performance.png" width="440" alt="Performance Tuning" /> | <img src="./screenshots/system_monitor.png" width="440" alt="System Monitor" /> |
| **🛡️ Security & Biometrics** | **🤖 AI Integration (MCP Server)** |
| <img src="./screenshots/security.png" width="440" alt="Security View" /> | <img src="./screenshots/ai_integration.png" width="440" alt="AI MCP Server" /> |

<details>
<summary><b>🔍 Click to view more screenshots (System Info, Settings Sync, Extended Dashboard)</b></summary>
<br/>

| 💻 Hardware & Kernel Info | ⚙️ Settings & Cloud Backup |
| :---: | :---: |
| <img src="./screenshots/system_info.png" width="440" alt="System Info" /> | <img src="./screenshots/settings_sync.png" width="440" alt="Settings Sync" /> |

<br/>

**Extended Home Dashboard Overview**
<br/>
<img src="./screenshots/home_extended.png" width="800" alt="Extended Dashboard" />

</details>

</div>

---

## 🚀 Installation

Prebuilt release binaries for Linux (`.deb`, `.rpm`, `.AppImage`) and Windows (`.exe`) are available on the [**GitHub Releases Page**](https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded/releases).

### 🐧 Ubuntu / Debian / Pop!_OS / Linux Mint

**Option 1: Using APT Repository**
```bash
echo "deb [trusted=yes] https://gh.vietanh.dev/ThinkUtils/apt ./" | sudo tee /etc/apt/sources.list.d/thinkutils.list
sudo apt update
sudo apt install thinkutils
```

**Option 2: Direct .deb Package**
```bash
# Download the latest thinkutils_*.deb from Releases
sudo dpkg -i thinkutils_*.deb
sudo apt install -f  # resolve any missing dependencies if needed
```

### 🔴 Fedora / RHEL / Rocky Linux
```bash
# Download the latest thinkutils-*.rpm from Releases
sudo dnf install ./thinkutils-*.rpm
```

### 📦 Universal Linux (AppImage)
```bash
chmod +x thinkutils_*.AppImage
./thinkutils_*.AppImage
```

### 🪟 Windows 10 / 11
Download `thinkutils.exe` or the Windows installer from [**GitHub Releases**](https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded/releases) and run directly.

---

## ⚙️ ThinkPad Hardware Setup

To allow ThinkUtils to directly read and control your cooling fan hardware on Linux:

1. Create or edit the `thinkpad_acpi` kernel module configuration:
   ```bash
   sudo nano /etc/modprobe.d/thinkpad_acpi.conf
   ```

2. Add this configuration directive:
   ```ini
   options thinkpad_acpi fan_control=1
   ```

3. Reload the module (or reboot your laptop):
   ```bash
   sudo modprobe -r thinkpad_acpi
   sudo modprobe thinkpad_acpi
   ```

4. Verify that the fan interface is active:
   ```bash
   cat /proc/acpi/ibm/fan
   ```

---

## 🛡️ PAM Security & Health CLI

ThinkUtils includes an enterprise-grade PAM configuration analyzer and repair engine designed to prevent and recover from authentication lockout issues (such as malformed PAM extended control syntax):

```bash
# Audit PAM authentication configuration health
thinkutils --check-pam

# Safely sanitize, repair, and validate system PAM configurations
thinkutils --repair-pam

# Roll back PAM configuration to the latest verified automatic backup
thinkutils --rollback-pam

# Display CLI help
thinkutils --help
```

---

## 🤖 AI Integration (MCP Server)

ThinkUtils embeds a native **Model Context Protocol (MCP)** server on `http://127.0.0.1:3000/sse`. This enables AI assistants (such as Claude Code, Claude Desktop, Cursor, or OpenCode) to inspect thermal sensors and adjust system parameters on your behalf.

### Claude Desktop Configuration
Add this to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "thinkutils": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-sse", "http://127.0.0.1:3000/sse"]
    }
  }
}
```

---

## 🛠️ Building from Source

### Prerequisites
- **Rust** 1.70+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- **Node.js** 22+ & `npm`
- **Linux Packages**: `build-essential libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf`

### Build Instructions
```bash
# 1. Clone repository
git clone https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded.git
cd ThinkUtils-Maintained-Upgraded

# 2. Install dependencies
npm ci

# 3. Launch in development mode
npm run tauri dev

# 4. Build production packages
npm run tauri build
```

The resulting packages will be in `src-tauri/target/release/bundle/`.

### Code Quality & Linting
```bash
npm run validate     # Runs ESLint, Stylelint, HTMLHint, and Prettier check
cargo test           # Runs all 147+ Rust integration & safety test suites
```

---

## 📐 Architecture

```
ThinkUtils/
├── src/                          # Modern vanilla JS/CSS/HTML frontend
│   ├── index.html                # App shell
│   ├── styles/                   # Scoped modular stylesheets
│   ├── js/                       # Modular ES6 controllers
│   │   ├── app.js                # Lifecycle & navigation
│   │   ├── fanCurve.js           # Canvas interactive curve engine
│   │   └── views/                # Views (fan, battery, monitor, security, mcp)
│   └── templates/                # Modular HTML view templates
├── src-tauri/                    # Rust native backend
│   ├── src/
│   │   ├── fan_control.rs        # Manual fan speed & EC control
│   │   ├── fan_curve.rs          # Background thermal curve supervisor & watchdog
│   │   ├── battery.rs            # Charge thresholds & sysfs interfaces
│   │   ├── performance.rs        # CPU governors, EPP, and Turbo Boost
│   │   ├── monitor.rs            # Real-time resource telemetry
│   │   ├── pam.rs                # PAM configuration validator, repair & rollback
│   │   ├── biometrics.rs         # Fingerprint authentication management
│   │   ├── security.rs           # ClamAV scanner integration
│   │   └── mcp.rs                # Built-in Model Context Protocol server
│   └── Cargo.toml
├── .github/workflows/            # Continuous Integration & multi-platform releases
└── docs/                         # Detailed VitePress documentation
```

---

## 🤝 Contributing

Contributions are warmly welcomed! Please feel free to submit a Pull Request or file an issue:
1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Ensure tests pass (`npm run validate` and `cargo test`)
4. Commit your changes (`git commit -m 'feat: add amazing feature'`)
5. Push to the branch (`git push origin feature/amazing-feature`)
6. Open a Pull Request

---

## 📄 License & Disclaimer

ThinkUtils is dual-licensed under the **GNU Lesser General Public License v3.0 (LGPL v3)** and commercial terms. See [LICENSE](./LICENSE) for details.

> [!WARNING]
> ThinkUtils controls hardware cooling and power parameters. While it includes safety watchdogs and conservative fallbacks, always monitor your machine's thermals when using manual fan overrides. The authors are not liable for hardware damage caused by improper usage.

<div align="center">
<sub>Designed with ❤️ for the global ThinkPad & Linux community.</sub>
</div>
