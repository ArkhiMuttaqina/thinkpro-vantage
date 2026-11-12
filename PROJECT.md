# ThinkPro Vantage

Linux-only ThinkPad companion utility. Rust backend + vanilla JavaScript
frontend on Tauri v2.

## Purpose

Fan curves, dual-battery charge thresholds, CPU governor / boost, PAM security
verification, and a localhost MCP server for AI assistants.

## Stack

- Frontend: Tauri v2, vanilla ES modules (`src/js`), scoped CSS (`src/styles`),
  HTML templates (`src/templates`).
- Backend: Rust (`src-tauri/src`), one module per concern (fan_control, battery,
  performance, power, security, mcp, ...).
- Distribution: deb/rpm/AppImage via GitHub Actions, plus AUR/COPR/PPA
  publishers.

## Key commands

Build: see `CLAUDE.md`. Release: see `docs/development/release.md`.

## Release pipeline

`.github/workflows/release.yml` triggers on a `v*` tag push and publishes the
GitHub Release + APT repo automatically. Bump with
`./scripts/bump-version.sh <X.Y.Z>`.
