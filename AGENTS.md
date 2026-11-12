# AGENTS.md

Operating instructions for AI agents working on ThinkPro Vantage.

## What this is

Linux-only Tauri v2 desktop app (Rust backend + vanilla JS/CSS/HTML frontend,
no framework) for ThinkPad hardware control: fan curves, battery thresholds,
CPU governor, PAM security, and a localhost MCP server.

## Build & test

```bash
npm run tauri dev    # hot reload
npm run tauri build  # deb/rpm/AppImage
cargo check          # quick Rust check, from src-tauri/
cargo test           # Rust unit + integration tests
cargo audit          # dependency scan, must exit 0
npm run validate     # ESLint + Stylelint + HTMLHint + Prettier
```

## Release

Full procedure: `docs/development/release.md`. Short form — decide the level,
then:

```bash
./scripts/bump-version.sh 0.3.0 && ./scripts/bump-version.sh --check
git add -A && git commit -m "chore: bump to v0.3.0"
git tag v0.3.0 && git push && git push --tags
```

Bump level (pre-1.0): breaking → minor, new feature → minor,
fix/deps/docs → patch. Never hand-edit a version number.

## Invariants

- Linux-only: do not reintroduce Windows/macOS bundling (`tauri.conf.json`
  targets, `generate-icons.sh`, CI jobs, docs, issue templates).
- Delete dead weight on cutover; migrate every caller; no shims.
- `bump-version.sh` is the single source of truth for version declarations.
- The MCP server binds loopback only (127.0.0.1/localhost) and allowlists
  Host + Origin. Never widen it.
- Fan control goes through the `thinkpro-ctl` helper + polkit rule (pkexec),
  never arbitrary shell.
