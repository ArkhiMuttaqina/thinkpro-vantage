# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
# Development
npm run tauri dev              # Run with hot reload + Rust debug build

# Build
npm run tauri build            # Production build (.deb, .rpm, .AppImage)
cargo check                    # Quick Rust compile check (from src-tauri/)
cargo test                     # Run Rust unit tests (from src-tauri/)

# Linting & Formatting
npm run lint                   # Run all linters (ESLint, Stylelint, HTMLHint)
npm run lint:fix               # Auto-fix linting issues
npm run format                 # Format with Prettier
npm run validate               # Lint + format check (run before committing)
```

Pre-commit hooks (Husky + lint-staged) run linters automatically on staged files.

## Version Bumping

Version must be updated in all 4 files before release:
- `package.json` — `"version": "X.Y.Z"`
- `package-lock.json` — `"version": "X.Y.Z"` (2 occurrences at top)
- `src-tauri/Cargo.toml` — `version = "X.Y.Z"`
- `src-tauri/tauri.conf.json` — `"version": "X.Y.Z"`

After committing, tag with `git tag vX.Y.Z` and push the tag — the GitHub Actions workflow builds and publishes release artifacts automatically.

After version bump, also run `cargo check` from `src-tauri/` to update `Cargo.lock`, and include it in the commit.

## Local Build & Install

```bash
./scripts/upgrade.sh          # Build + install .deb in one step
```

## Architecture

**Tauri v2 app** with Rust backend and vanilla JavaScript frontend (no framework).

### Backend (`src-tauri/src/`)

All Tauri commands return `ApiResponse<T> { success, data, error }` for consistent error handling. System operations requiring root use **pkexec** (PolicyKit) — the app itself runs unprivileged. `pkexec <helper> <verb>` is preferred wherever the helper has a verb for the job: the polkit rule matches the helper's own path, so those writes are passwordless, and the privileged surface stays an enumerable set instead of arbitrary bash.

Key modules:
- **fan_control.rs** — Manual fan speed via `/proc/acpi/ibm/fan`, permission checks, polkit rule installation
- **fan_curve.rs** — Temperature-based auto fan control with a background task (runs every 2s). Uses `FanCurveState` (Arc<Mutex>) shared state. Falls back to pkexec only when polkit rule is installed (to avoid dialog spam from background task)
- **permissions.rs** — Broader permission setup for sysfs files (CPU governor, battery thresholds). Separate from fan permissions
- **battery.rs** — Reads `/sys/class/power_supply/BAT0|BAT1/`
- **performance.rs** — CPU governor, turbo boost, power profiles via sysfs. Also the GPU clock ceiling: it asks LACT to detach the card, writes `pp_dpm_sclk` through the privileged helper (`gpu-clock-cap` / `gpu-clock-auto`, so the write is passwordless), saves one ceiling per power source in settings, and `gpu_clock_watcher` (spawned in `lib.rs`) re-applies the ceiling for whichever source is in force — AC, on battery or power saver, told apart by `AC/online` and the profile the app tracks — while the app runs, since LACT's detach does not survive an `lactd` restart.
- **monitor.rs** — System stats (CPU, memory, disk, network, processes)
- **security.rs** — ClamAV integration
- **settings.rs** — Persistent storage via tauri-plugin-store (JSON) with JSON export and import
- **autostart.rs** — Writes/removes the XDG login entry (`~/.config/autostart/thinkpro-vantage.desktop`) that makes the app come back at login and re-apply the fan curve and GPU ceiling. The Settings toggle drives it, and it is reconciled against the saved `auto_start` setting on every start.

### Frontend (`src/js/`)

- **app.js** — Initialization entry point, sets up all views and permissions
- **state.js** — Simple centralized state object (current mode, intervals, locks)
- **dom.js** — Cached DOM element references (avoids repeated getElementById)
- **navigation.js** — View routing via sidebar `data-feature` attributes
- **settingsManager.js** — Load/save/apply settings coordination
- **fanCurve.js** — Canvas-based interactive curve editor with draggable points
- **views/** — One JS file per feature (fan.js, battery.js, performance.js, etc.)

### Communication Pattern

Frontend calls backend via `window.__TAURI__.core.invoke('command', {args})`. Backend pushes async updates via `app.emit_to("main", "event-name", payload)` — used by the fan curve background task to send temperature/level updates and permission errors.

### Permission Model

One unified setup (`permissions.rs::setup_permissions()`) handles everything in a single pkexec call:
- Installs a dedicated fan helper at `/usr/local/bin/thinkpro-ctl` (validates commands, persists across reboots)
- Installs a tight polkit rule at `/etc/polkit-1/rules.d/50-thinkpro-vantage.rules` (only allows the helper, not arbitrary bash)
- Sets sysfs file permissions for CPU/battery control (resets on reboot but non-critical)

Permission checks use `Path::exists()` on the **helper binary** only — the polkit rules directory is root-only so the rule file can't be checked by non-root users.

Performance settings (CPU governor, turbo boost) are NOT auto-applied on startup to avoid pkexec password prompts. Fan settings are only restored if the helper is already installed.

## Rules

- Never add Claude as co-author in git commits.
