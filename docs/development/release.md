# Release Procedure

Applies while ThinkPro Vantage is pre-1.0 (0.x). After 1.0.0 the table shifts
one row: breaking changes bump **major** instead of **minor**.

## 1. Decide the version bump

| What changed | Level (0.x) |
|--------------|-------------|
| Breaking — removed OS support, changed/removed a `#[tauri::command]`, config schema change, MCP wire-protocol change (major dep bump like rmcp 2→3) | **minor** |
| New feature — new page, command, or backend module | **minor** |
| Bug fix, security/dependency bump, refactor, docs | **patch** |

When a session mixes these, take the highest level.

## 2. Bump every declaration

```bash
./scripts/bump-version.sh 0.3.0
./scripts/bump-version.sh --check
```

`bump-version.sh` edits every version declaration (package.json,
package-lock.json ×2, Cargo.toml, tauri.conf.json, AUR PKGBUILD, COPR spec,
Debian changelog) and refreshes Cargo.lock. `--check` is what CI runs to catch
a half-bumped tree before it ships.

## 3. Commit, tag, push

```bash
git add -A
git commit -m "chore: bump to v0.3.0"
git tag v0.3.0
git push && git push --tags
```

The `v*` tag fires `.github/workflows/release.yml`: `ci` (tests) → `build`
(deb/rpm/AppImage × amd64/arm64) → `release` (GitHub Release + APT repo update
on `gh-pages`).

## 4. Verify

- Release assets: deb, rpm, AppImage × both arches, plus GitHub's auto-attached
  source tar.gz.
- APT repo: `apt update` against the gh-pages repo (unsigned unless signing
  keys are set — see `apt-signing.md`).

## Secrets

| secret | used by | required |
|--------|---------|----------|
| `GITHUB_TOKEN` | action-gh-release | auto |
| `APT_GPG_PRIVATE_KEY` / `APT_GPG_PASSPHRASE` | APT signing | optional |
| `AUR_SSH_PRIVATE_KEY` | publish-aur.yml | optional (manual) |
| `COPR_API_TOKEN` | publish-copr.yml | optional (manual) |
| `LAUNCHPAD_GPG_*` | publish-ppa.yml | optional (manual) |
