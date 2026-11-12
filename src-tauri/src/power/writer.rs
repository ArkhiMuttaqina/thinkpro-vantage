//! Writing TLP settings without taking the machine hostage.
//!
//! Three rules shape this module.
//!
//! 1. The app never edits the administrator's `/etc/tlp.conf`. It owns exactly one
//!    file, `/etc/tlp.d/99-thinkpro-vantage.conf`, and every change is written
//!    there -- so removing that one file undoes everything the app ever set.
//! 2. Nothing reaches the file that the installed TLP does not recognise. Keys are
//!    checked against the schema TLP itself ships, values against the type that key
//!    declares, and the character set is closed: a value cannot carry a newline, a
//!    quote, a backtick or a `#`, so a value can never become a second setting.
//! 3. A value the app writes can still be overridden: TLP reads `/etc/tlp.conf`
//!    after `/etc/tlp.d`, so a key the user set there wins. That is reported rather
//!    than hidden, which is also why `verify` re-reads the merged configuration
//!    instead of trusting the write.
//! 4. A write merges, it never replaces. The file is rebuilt from the keys it
//!    already holds with the changes laid over them, so a caller that mentions one
//!    setting cannot delete the others. An empty value is the one way to ask for a
//!    key to go: leaving a key out keeps it, sending it cleared removes it.
//!    Merging does not make the `.bak` redundant, either: a write still replaces
//!    every value it names, so rolling back stays the only way back to those.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::tlp::{self, SchemaEntry, ValueKind};

/// The one file this app owns.
pub const DROPIN_FILE: &str = "etc/tlp.d/99-thinkpro-vantage.conf";

/// Upper bound on a single write: a settings file, not a dumping ground.
const MAX_CHANGES: usize = 200;

/// One setting to write -- or, with an empty value, one key to drop from the
/// file so that TLP's own default applies again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub key: String,
    pub value: String,
}

impl Change {
    /// An empty value means "take this key out", not "set it to nothing".
    ///
    /// The two have to stay apart in both directions: a key the caller leaves out
    /// keeps the value the file already holds, while a key sent with an empty
    /// value is deleted. Nothing is written for a removal, which is why
    /// `validate` does not check the empty value against the key's type.
    pub fn is_removal(&self) -> bool {
        self.value.is_empty()
    }
}

/// Where the drop-in lives. `/` on a real machine; a temporary directory in tests.
#[derive(Debug, Clone)]
pub struct DropinPaths {
    root: PathBuf,
}

impl Default for DropinPaths {
    fn default() -> Self {
        Self::system()
    }
}

impl DropinPaths {
    pub fn system() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    pub fn under(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn dropin(&self) -> PathBuf {
        self.root.join(DROPIN_FILE)
    }

    /// Kept next to the drop-in and harmless to TLP: it does not match `*.conf`.
    pub fn backup(&self) -> PathBuf {
        self.root.join(format!("{}.bak", DROPIN_FILE))
    }

    /// Staged content, written by the unprivileged app before it is installed.
    pub fn staged(&self) -> PathBuf {
        self.root.join("tmp/thinkpro-vantage-tlp.conf")
    }

    pub fn display_dropin(&self) -> String {
        format!("/{}", DROPIN_FILE)
    }
}

/// Why a requested change will not be written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub key: String,
    pub value: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validated {
    pub accepted: Vec<Change>,
    pub rejected: Vec<Rejection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    pub available: bool,
    pub reason: Option<String>,
    pub path: String,
    pub content: String,
    pub accepted: Vec<Change>,
    pub rejected: Vec<Rejection>,
    /// Keys the drop-in sets that `/etc/tlp.conf` also sets, so the app's value
    /// will lose. Reported before writing, not after.
    pub overridden: Vec<Override>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub key: String,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub path: String,
    pub backup: Option<String>,
    pub applied: Vec<Change>,
    pub rejected: Vec<Rejection>,
    /// Keys that are still owned by another file after the write.
    pub overridden: Vec<Override>,
    pub tlp_restarted: bool,
    pub note: String,
}

/// Characters a value may use, per kind. Everything else is refused, which is what
/// stops a value from carrying a second setting along with it.
fn value_allowed(kind: &ValueKind, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("the value is empty".to_string());
    }
    if value.len() > 256 {
        return Err("the value is longer than 256 characters".to_string());
    }
    if value.chars().any(|c| c.is_control()) {
        return Err("the value contains a control character".to_string());
    }

    match kind {
        // TLP accepts both spellings but does not translate between them: parts
        // of it compare a switch to the literal "Y" (see
        // func.d/05-tlp-func-pm, `[ "$WOL_DISABLE" = "Y" ]`), so writing 1 for
        // one of those leaves the setting silently unchanged. Both pairs are
        // allowed through, and the editor writes whichever one the setting's own
        // TLP default uses.
        ValueKind::Bool => match value {
            "0" | "1" | "Y" | "N" => Ok(()),
            _ => Err("this setting is a switch: 0, 1, Y or N".to_string()),
        },
        ValueKind::Int => {
            let digits = value.strip_prefix('-').unwrap_or(value);
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return Err("this setting is a whole number".to_string());
            }
            Ok(())
        }
        ValueKind::Enum { values, .. } => {
            if values.iter().any(|candidate| candidate == value) {
                Ok(())
            } else {
                Err(format!("must be one of: {}", values.join(", ")))
            }
        }
        ValueKind::Text => {
            // Letters, digits and the punctuation TLP uses in lists and paths.
            if !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " _,.+:%-/()".contains(c))
            {
                return Err(
                    "only letters, digits, spaces and _ , . + : % - / ( ) are allowed".to_string(),
                );
            }
            // A list is space separated; a long one is a mistake, not a setting.
            if value.split_whitespace().count() > 16 {
                return Err("no more than 16 entries in one list".to_string());
            }
            Ok(())
        }
    }
}

/// Check every requested change against the schema of the installed TLP.
pub fn validate(
    entries: &[SchemaEntry],
    renamed: &BTreeMap<String, String>,
    changes: &[Change],
) -> Validated {
    let mut accepted: Vec<Change> = Vec::new();
    let mut rejected: Vec<Rejection> = Vec::new();

    if changes.len() > MAX_CHANGES {
        return Validated {
            accepted,
            rejected: vec![Rejection {
                key: String::new(),
                value: String::new(),
                reason: format!("at most {} settings can be written at once", MAX_CHANGES),
            }],
        };
    }

    for change in changes {
        let mut reject = |reason: String| {
            rejected.push(Rejection {
                key: change.key.clone(),
                value: change.value.clone(),
                reason,
            });
        };

        if change.key.trim() != change.key || change.key.is_empty() {
            reject("the key is empty or padded with whitespace".to_string());
            continue;
        }
        if accepted.iter().any(|already| already.key == change.key) {
            reject("the same key was sent twice".to_string());
            continue;
        }
        if let Some(next) = renamed.get(&change.key) {
            reject(format!("TLP renamed this setting to {}", next));
            continue;
        }
        let Some(entry) = entries.iter().find(|entry| entry.key == change.key) else {
            reject("this TLP version has no such parameter".to_string());
            continue;
        };
        if let Some(note) = &entry.deprecated {
            reject(format!("TLP says this setting is deprecated: {}", note));
            continue;
        }
        if change.is_removal() {
            // A removal: the key leaves the file and TLP's own default applies
            // again. There is no value to check against the key's type.
            accepted.push(change.clone());
            continue;
        }
        match value_allowed(&entry.kind, &change.value) {
            Ok(()) => accepted.push(change.clone()),
            Err(reason) => reject(reason),
        }
    }

    Validated { accepted, rejected }
}

/// Render the drop-in: a comment saying what wrote it, then one setting per line.
///
/// Values are quoted only when they contain a space, and the character set allowed
/// by `validate` means a value can never break out of its own line.
pub fn render(changes: &[Change], stamp: &str) -> String {
    let mut sorted: Vec<&Change> = changes.iter().collect();
    sorted.sort_by(|left, right| left.key.cmp(&right.key));

    let mut out = String::new();
    out.push_str("# Written by ThinkPro Vantage. Do not edit by hand.\n");
    out.push_str("# This file is the only one the application owns; delete it to undo\n");
    out.push_str("# everything it ever set. Settings in /etc/tlp.conf win over these,\n");
    out.push_str("# because TLP reads that file after /etc/tlp.d.\n");
    out.push_str(&format!("# last change: {}\n", stamp));
    out.push('\n');
    for change in sorted {
        if change.value.contains(' ') {
            out.push_str(&format!("{}=\"{}\"\n", change.key, change.value));
        } else {
            out.push_str(&format!("{}={}\n", change.key, change.value));
        }
    }
    out
}

/// The settings a drop-in currently holds, in key order.
///
/// Only `key=value` counts as a setting; comments and blank lines are skipped
/// because the header is written fresh on every save. Anything else in the file --
/// a stray comment, a line with no `=` -- is ignored rather than guessed at, and
/// is not carried over.
fn parse_dropin(content: &str) -> Vec<Change> {
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        // TLP cuts a line at the first `#`, and no value `validate` accepts can
        // hold one, so a trailing comment is neither key nor value.
        let rest = rest.split('#').next().unwrap_or("").trim();
        let value = rest
            .strip_prefix('"')
            .and_then(|quoted| quoted.strip_suffix('"'))
            .unwrap_or(rest);
        pairs.insert(key.to_string(), value.to_string());
    }
    pairs
        .into_iter()
        .map(|(key, value)| Change { key, value })
        .collect()
}

/// What the next write leaves in the file.
///
/// The keys the file already holds come first, so a caller that mentions one
/// setting cannot delete the others; each change is then laid over them, a change
/// carrying an empty value removing its key. That is the whole difference between
/// "I did not mention it" and "I want it gone".
fn merge(existing: &[Change], changes: &[Change]) -> Vec<Change> {
    let mut pairs: BTreeMap<String, String> = existing
        .iter()
        .map(|pair| (pair.key.clone(), pair.value.clone()))
        .collect();
    for change in changes {
        if change.is_removal() {
            pairs.remove(&change.key);
        } else {
            pairs.insert(change.key.clone(), change.value.clone());
        }
    }
    pairs
        .into_iter()
        .map(|(key, value)| Change { key, value })
        .collect()
}

/// The content the next write stages: the file as it stands now, with `changes`
/// laid over it, under a fresh header.
///
/// The preview and the write both come through here, so what the operator is shown
/// is the file that will be installed -- not merely the changes they asked for.
fn next_content(paths: &DropinPaths, changes: &[Change], stamp: &str) -> Result<String, String> {
    let dropin = paths.dropin();
    let existing = match std::fs::read_to_string(&dropin) {
        Ok(content) => parse_dropin(&content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(format!(
                "could not read {} before writing it: {}",
                dropin.display(),
                error
            ))
        }
    };
    Ok(render(&merge(&existing, changes), stamp))
}

/// Which of the requested keys another file also sets.
pub fn overridden_keys(
    changes: &[Change],
    provenance: &BTreeMap<String, tlp::Provenance>,
    dropin: &str,
) -> Vec<Override> {
    changes
        .iter()
        .filter_map(|change| {
            let source = provenance.get(&change.key)?;
            if !wins_over_dropin(&source.path, dropin) {
                return None;
            }
            Some(Override {
                key: change.key.clone(),
                path: source.path.clone(),
                line: source.line,
            })
        })
        .collect()
}

/// Whether `path` is read late enough to take a value back from the drop-in.
///
/// TLP reads `/usr/share/tlp/defaults.conf`, then `/etc/tlp.d/*.conf` in lexical
/// order, then `/etc/tlp.conf` last. The drop-in is
/// `99-thinkpro-vantage.conf`, so it already outranks the shipped defaults and
/// every lower-sorting drop-in; those cannot override it and reporting them as
/// though they did would put a warning on almost every row. Only the two sources
/// genuinely read after it can: `/etc/tlp.conf` itself, and a drop-in whose name
/// sorts above `99-`.
fn wins_over_dropin(path: &str, dropin: &str) -> bool {
    if path == dropin {
        return false;
    }
    if path == tlp::TEMPLATE_CONF {
        return true;
    }
    match (path.rsplit_once('/'), dropin.rsplit_once('/')) {
        (Some((dir, name)), Some((dropin_dir, dropin_name))) if dir == dropin_dir => {
            name > dropin_name
        }
        _ => false,
    }
}

/// The script that installs a staged file as the drop-in.
///
/// Paths are fixed by the caller and quoted; no user value is interpolated into
/// this text, which is why the value checks above are the whole injection story.
pub fn install_script(paths: &DropinPaths, staged: &Path) -> String {
    format!(
        "set -e\nDROPIN='{}'\nBACKUP='{}'\nSOURCE='{}'\nmkdir -p \"$(dirname \"$DROPIN\")\"\nif [ -f \"$DROPIN\" ]; then cp -a \"$DROPIN\" \"$BACKUP\"; fi\ninstall -m0644 \"$SOURCE\" \"$DROPIN\"\nrm -f \"$SOURCE\"\nif systemctl is-active --quiet tlp; then tlp start >/dev/null 2>&1 || true; fi\n",
        paths.dropin().display(),
        paths.backup().display(),
        staged.display()
    )
}

/// The script that puts the previous state back.
pub fn rollback_script(paths: &DropinPaths) -> String {
    format!(
        "set -e\nDROPIN='{}'\nBACKUP='{}'\nif [ -f \"$BACKUP\" ]; then mv -f \"$BACKUP\" \"$DROPIN\"; else rm -f \"$DROPIN\"; fi\nif systemctl is-active --quiet tlp; then tlp start >/dev/null 2>&1 || true; fi\n",
        paths.dropin().display(),
        paths.backup().display()
    )
}

/// Stage the whole file the next write will install: what the drop-in holds now,
/// with this write's changes laid over it.
fn stage_next(paths: &DropinPaths, changes: &[Change], stamp: &str) -> Result<PathBuf, String> {
    stage(paths, &next_content(paths, changes, stamp)?)
}

/// Write the staged file, unprivileged, with the tightest permissions that work.
pub fn stage(paths: &DropinPaths, content: &str) -> Result<PathBuf, String> {
    let staged = paths.staged();
    if let Some(parent) = staged.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {}", parent.display(), error))?;
    }
    std::fs::write(&staged, content)
        .map_err(|error| format!("could not write {}: {}", staged.display(), error))?;
    Ok(staged)
}

/// When the file was last written, as the header's `last change` line reports it.
///
/// In-process and format-checked: the previous implementation shelled out to
/// `date -%Y-%m-%dT%H:%M:%S%z`, which GNU date refuses (it wants `+`), so every
/// write this app ever made said `last change: unknown`. A line that is supposed to
/// say when the file changed has to be able to.
fn timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%z")
        .to_string()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

use crate::battery::ApiResponse;

fn unavailable_preview(reason: String, paths: &DropinPaths) -> Preview {
    Preview {
        available: false,
        reason: Some(reason),
        path: paths.display_dropin(),
        content: String::new(),
        accepted: Vec::new(),
        rejected: Vec::new(),
        overridden: Vec::new(),
    }
}

/// The preview itself, against one drop-in. Split out so a test can point it at a
/// scratch directory rather than at the machine's `/etc/tlp.d`.
fn build_preview(paths: &DropinPaths, changes: &[Change]) -> Preview {
    let schema = tlp::tlp_schema();
    let Some(schema) = schema.data else {
        return unavailable_preview(
            "TLP's configuration could not be read, so no setting can be checked against it."
                .to_string(),
            paths,
        );
    };
    if !schema.available {
        return unavailable_preview(
            schema
                .reason
                .unwrap_or_else(|| "TLP is not installed.".to_string()),
            paths,
        );
    }

    let validated = validate(&schema.entries, &schema.renamed, changes);
    let dropin = paths.display_dropin();
    let overridden = match tlp::tlp_config().data {
        Some(config) => overridden_keys(&validated.accepted, &config.provenance, &dropin),
        None => Vec::new(),
    };
    // The file that would be installed, not the change list: a preview that
    // showed only the changes would hide the keys that are about to stay.
    let content = match next_content(paths, &validated.accepted, &timestamp()) {
        Ok(content) => content,
        Err(reason) => return unavailable_preview(reason, paths),
    };

    Preview {
        available: true,
        reason: None,
        path: dropin,
        content,
        accepted: validated.accepted,
        rejected: validated.rejected,
        overridden,
    }
}

/// What would be written, and what would lose to `/etc/tlp.conf`.
#[tauri::command]
pub async fn power_preview_changes(changes: Vec<Change>) -> ApiResponse<Preview> {
    let paths = DropinPaths::system();
    let result = tokio::task::spawn_blocking(move || build_preview(&paths, &changes)).await;

    match result {
        Ok(preview) => ApiResponse {
            success: true,
            data: Some(preview),
            error: None,
        },
        Err(error) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("The preview failed to run: {}", error)),
        },
    }
}

/// Write the drop-in, restart TLP, then re-read the merged configuration.
#[tauri::command]
pub async fn power_apply_changes(changes: Vec<Change>) -> ApiResponse<Applied> {
    let paths = DropinPaths::system();
    let result = tokio::task::spawn_blocking(move || apply(&paths, &changes)).await;
    match result {
        Ok(Ok(applied)) => ApiResponse {
            success: true,
            data: Some(applied),
            error: None,
        },
        Ok(Err(message)) => ApiResponse {
            success: false,
            data: None,
            error: Some(message),
        },
        Err(error) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("The write failed to run: {}", error)),
        },
    }
}

/// Put the previous drop-in back, or remove it when this was the first write.
#[tauri::command]
pub async fn power_rollback_dropin() -> ApiResponse<String> {
    let paths = DropinPaths::system();
    let result = tokio::task::spawn_blocking(move || {
        let had_backup = paths.backup().exists();
        run_privileged(&rollback_script(&paths))?;
        Ok::<String, String>(if had_backup {
            "The previous settings file was restored and TLP restarted.".to_string()
        } else {
            "The settings file was removed and TLP restarted; nothing this application set remains."
                .to_string()
        })
    })
    .await;

    match result {
        Ok(Ok(message)) => ApiResponse {
            success: true,
            data: Some(message),
            error: None,
        },
        Ok(Err(message)) => ApiResponse {
            success: false,
            data: None,
            error: Some(message),
        },
        Err(error) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("The rollback failed to run: {}", error)),
        },
    }
}

fn apply(paths: &DropinPaths, changes: &[Change]) -> Result<Applied, String> {
    let schema = tlp::tlp_schema();
    let schema = schema
        .data
        .ok_or_else(|| "TLP's configuration could not be read.".to_string())?;
    if !schema.available {
        return Err(schema
            .reason
            .unwrap_or_else(|| "TLP is not installed.".to_string()));
    }

    let validated = validate(&schema.entries, &schema.renamed, changes);
    if validated.accepted.is_empty() {
        return Err(match validated.rejected.first() {
            Some(rejection) => format!("{}: {}", rejection.key, rejection.reason),
            None => "No settings were sent.".to_string(),
        });
    }

    let had_backup = paths.backup().exists();
    let had_dropin = paths.dropin().exists();
    // Merged, not replaced: a key this write does not name keeps the value the
    // file already holds, and only an empty value takes a key out.
    let staged = stage_next(paths, &validated.accepted, &timestamp())?;
    run_privileged(&install_script(paths, &staged))?;

    // Ask TLP what it now thinks, rather than assuming the write took effect.
    let dropin = paths.display_dropin();
    let (overridden, restarted) = match tlp::tlp_config().data {
        Some(config) => (
            overridden_keys(&validated.accepted, &config.provenance, &dropin),
            true,
        ),
        None => (Vec::new(), false),
    };

    let note = if !overridden.is_empty() {
        "Some settings are also present in a file TLP reads later, so those values keep winning."
            .to_string()
    } else if !had_dropin {
        "Written. Delete the file, or use Roll back, to undo it.".to_string()
    } else {
        "Written over the previous file, which was kept as a backup.".to_string()
    };

    Ok(Applied {
        path: dropin,
        backup: if had_backup || had_dropin {
            Some(format!("{}.bak", paths.display_dropin()))
        } else {
            None
        },
        applied: validated.accepted,
        rejected: validated.rejected,
        overridden,
        tlp_restarted: restarted,
        note,
    })
}

fn run_privileged(script: &str) -> Result<(), String> {
    let output = std::process::Command::new("pkexec")
        .args(["bash", "-c", script])
        .output()
        .map_err(|error| format!("could not run pkexec: {}", error))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("the privileged step exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::tlp::{Group, SchemaEntry, Source};

    fn entry(key: &str, kind: ValueKind) -> SchemaEntry {
        SchemaEntry {
            key: key.to_string(),
            row: key.to_string(),
            source: Some(Source::Ac),
            group: Group::Cpu,
            group_label: "CPU".to_string(),
            kind,
            default_value: None,
            doc: None,
            deprecated: None,
            applies: true,
            unavailable: None,
        }
    }

    fn schema() -> Vec<SchemaEntry> {
        vec![
            entry("CPU_SCALING_GOVERNOR_ON_AC", ValueKind::Text),
            entry("CPU_BOOST_ON_AC", ValueKind::Bool),
            // Disabled on this class of machine, yet still a real key: the
            // applicability verdict is advisory and `validate` must not care,
            // or a config synced from another machine would stop loading.
            SchemaEntry {
                applies: false,
                unavailable: Some("no intel_pstate on this machine".to_string()),
                ..entry("CPU_MIN_PERF_ON_AC", ValueKind::Int)
            },
            entry(
                "PLATFORM_PROFILE_ON_AC",
                ValueKind::Enum {
                    values: vec!["performance".to_string(), "balanced".to_string()],
                    default: Some("balanced".to_string()),
                },
            ),
            SchemaEntry {
                deprecated: Some("removed in 1.6".to_string()),
                ..entry("USB_AUTOSUSPEND", ValueKind::Bool)
            },
        ]
    }

    fn change(key: &str, value: &str) -> Change {
        Change {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    /// The drop-in as the operator left it after the loss of 2026-11-03: six keys
    /// and the header that says who owns the file.
    const OPERATOR_DROPIN: &str = "# Written by ThinkPro Vantage. Do not edit by hand.\n# last change: restored\n\nCPU_BOOST_ON_BAT=1\nTLP_PROFILE_AC=BAL\nTLP_PROFILE_BAT=SAV\nTLP_PROFILE_DEFAULT=BAL\nMEM_SLEEP_ON_AC=s2idle\nMEM_SLEEP_ON_BAT=s2idle\n";

    /// A private root for one test, empty on entry, with the drop-in's parent made.
    /// The root comes back so the test can clean up after itself.
    fn scratch(name: &str) -> (PathBuf, DropinPaths) {
        let root = std::env::temp_dir().join(format!("tpv-writer-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = DropinPaths::under(&root);
        std::fs::create_dir_all(paths.dropin().parent().expect("parent")).expect("mkdir");
        (root, paths)
    }

    #[test]
    fn a_known_setting_with_a_valid_value_is_accepted() {
        let schema = schema();
        let validated = validate(
            &schema,
            &BTreeMap::new(),
            &[
                change("CPU_SCALING_GOVERNOR_ON_AC", "powersave"),
                change("CPU_BOOST_ON_AC", "1"),
                change("CPU_MIN_PERF_ON_AC", "10"),
                change("PLATFORM_PROFILE_ON_AC", "balanced"),
            ],
        );
        assert_eq!(validated.accepted.len(), 4);
        assert!(validated.rejected.is_empty());
    }

    #[test]
    fn unknown_renamed_and_deprecated_keys_are_refused() {
        let schema = schema();
        let mut renamed = BTreeMap::new();
        renamed.insert(
            "CPU_HWP_ON_AC".to_string(),
            "CPU_ENERGY_PERF_POLICY_ON_AC".to_string(),
        );
        let validated = validate(
            &schema,
            &renamed,
            &[
                change("NOT_A_PARAMETER", "1"),
                change("CPU_HWP_ON_AC", "1"),
                change("USB_AUTOSUSPEND", "1"),
            ],
        );
        assert!(validated.accepted.is_empty());
        let reasons: Vec<String> = validated
            .rejected
            .iter()
            .map(|rejection| rejection.reason.clone())
            .collect();
        assert!(reasons[0].contains("no such parameter"), "{reasons:?}");
        assert!(reasons[1].contains("renamed"), "{reasons:?}");
        assert!(reasons[2].contains("deprecated"), "{reasons:?}");
    }

    #[test]
    fn values_are_checked_against_the_type_the_key_declares() {
        let schema = schema();
        let validated = validate(
            &schema,
            &BTreeMap::new(),
            &[
                change("CPU_BOOST_ON_AC", "yes"),
                change("CPU_BOOST_ON_AC", "0"),
                change("CPU_MIN_PERF_ON_AC", "high"),
                change("CPU_MIN_PERF_ON_AC", "-1"),
                change("PLATFORM_PROFILE_ON_AC", "turbo"),
                change("PLATFORM_PROFILE_ON_AC", "performance"),
            ],
        );
        assert_eq!(validated.accepted.len(), 3, "{:?}", validated.accepted);
        let rejected: Vec<&str> = validated
            .rejected
            .iter()
            .map(|rejection| rejection.reason.as_str())
            .collect();
        assert!(rejected
            .iter()
            .any(|reason| reason.contains("whole number")));
        assert!(rejected
            .iter()
            .any(|reason| reason.contains("must be one of")));
    }

    /// TLP carries switches in two spellings and does not translate between
    /// them: parts of it compare a switch to the literal "Y"
    /// (`func.d/05-tlp-func-pm` reads `[ "$WOL_DISABLE" = "Y" ]`), so a key
    /// whose own default is `Y` is silently left alone if the editor is only
    /// ever able to write 1. Both pairs have to pass.
    #[test]
    fn a_switch_accepts_either_spelling() {
        for value in ["0", "1", "Y", "N"] {
            assert!(
                value_allowed(&ValueKind::Bool, value).is_ok(),
                "{} is a switch value",
                value
            );
        }
        for value in ["y", "yes", "on", ""] {
            assert!(
                value_allowed(&ValueKind::Bool, value).is_err(),
                "{} is not a switch value",
                value
            );
        }
    }

    /// TLP reads the shipped defaults first, so they cannot take a value back
    /// from the drop-in. Reporting them as an override would mark almost every
    /// row as losing, and the badge would stop meaning anything.
    #[test]
    fn only_a_file_read_after_the_dropin_overrides_it() {
        let dropin = "/etc/tlp.d/99-thinkpro-vantage.conf";
        let changes = [
            change("TLP_ENABLE", "1"),
            change("WOL_DISABLE", "Y"),
            change("CPU_BOOST_ON_AC", "1"),
            change("RESTORE_DEVICE_STATE_ON_STARTUP", "0"),
        ];
        let mut provenance = BTreeMap::new();
        for (key, path) in [
            ("TLP_ENABLE", "/usr/share/tlp/defaults.conf"),
            ("WOL_DISABLE", dropin),
            ("CPU_BOOST_ON_AC", "/etc/tlp.conf"),
            (
                "RESTORE_DEVICE_STATE_ON_STARTUP",
                "/etc/tlp.d/zz-later.conf",
            ),
        ] {
            provenance.insert(
                key.to_string(),
                tlp::Provenance {
                    path: path.to_string(),
                    line: 1,
                    origin: "user".to_string(),
                },
            );
        }

        let overridden = overridden_keys(&changes, &provenance, dropin);
        let overridden: Vec<&str> = overridden.iter().map(|over| over.key.as_str()).collect();
        assert_eq!(
            overridden,
            ["CPU_BOOST_ON_AC", "RESTORE_DEVICE_STATE_ON_STARTUP"]
        );
    }

    #[test]
    fn a_value_cannot_smuggle_a_second_setting() {
        let schema = schema();
        // Newline, quote, backtick, hash and semicolon all fail the character set.
        for hostile in [
            "powersave\nCPU_BOOST_ON_AC=0",
            "powersave\"",
            "`reboot`",
            "powersave # comment",
            "powersave; rm -rf /",
            "$",
        ] {
            let validated = validate(
                &schema,
                &BTreeMap::new(),
                &[change("CPU_SCALING_GOVERNOR_ON_AC", hostile)],
            );
            assert!(
                validated.accepted.is_empty(),
                "accepted {hostile:?} as {:?}",
                validated.accepted
            );
        }
    }

    #[test]
    fn a_duplicate_key_and_an_empty_key_are_refused() {
        let schema = schema();
        let validated = validate(
            &schema,
            &BTreeMap::new(),
            &[
                change("CPU_BOOST_ON_AC", "1"),
                change("CPU_BOOST_ON_AC", "0"),
                change(" CPU_BOOST_ON_AC", "1"),
            ],
        );
        assert_eq!(validated.accepted.len(), 1);
        assert_eq!(validated.rejected.len(), 2);
        assert!(validated.rejected[0].reason.contains("twice"));
        assert!(validated.rejected[1].reason.contains("padded"));
    }

    #[test]
    fn the_rendered_file_is_sorted_and_announces_that_tlp_conf_wins() {
        let content = render(
            &[
                change("CPU_BOOST_ON_AC", "0"),
                change("CPU_SCALING_GOVERNOR_ON_AC", "powersave"),
                change("DISK_DEVICES", "nvme0n1 sda"),
            ],
            "2026-01-01T00:00:00+0700",
        );
        let body: Vec<&str> = content
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .collect();
        assert_eq!(
            body,
            vec![
                "CPU_BOOST_ON_AC=0",
                "CPU_SCALING_GOVERNOR_ON_AC=powersave",
                "DISK_DEVICES=\"nvme0n1 sda\""
            ]
        );
        assert!(content.contains("/etc/tlp.conf win"));
        assert!(content.contains("2026-01-01T00:00:00+0700"));
    }

    #[test]
    fn a_key_another_file_owns_is_reported_as_overridden() {
        let changes = [
            change("CPU_BOOST_ON_AC", "1"),
            change("CPU_MIN_PERF_ON_AC", "20"),
        ];
        let mut provenance = BTreeMap::new();
        provenance.insert(
            "CPU_BOOST_ON_AC".to_string(),
            tlp::Provenance {
                path: "/etc/tlp.conf".to_string(),
                line: 42,
                origin: "user".to_string(),
            },
        );
        provenance.insert(
            "CPU_MIN_PERF_ON_AC".to_string(),
            tlp::Provenance {
                path: "/etc/tlp.d/99-thinkpro-vantage.conf".to_string(),
                line: 7,
                origin: "dropin".to_string(),
            },
        );
        let overridden =
            overridden_keys(&changes, &provenance, "/etc/tlp.d/99-thinkpro-vantage.conf");
        assert_eq!(overridden.len(), 1);
        assert_eq!(overridden[0].key, "CPU_BOOST_ON_AC");
        assert_eq!(overridden[0].line, 42);
    }

    #[test]
    fn the_install_and_rollback_scripts_keep_a_backup() {
        let dir = std::env::temp_dir().join(format!("tpv-writer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = DropinPaths::under(&dir);
        std::fs::create_dir_all(paths.dropin().parent().expect("parent")).expect("mkdir");
        std::fs::write(paths.dropin(), "CPU_BOOST_ON_AC=1\n").expect("seed");
        let staged = stage(&paths, "CPU_BOOST_ON_AC=0\n").expect("stage");

        // The same text the privileged path runs, without pkexec: the file
        // mechanics are what is being checked here.
        let status = std::process::Command::new("bash")
            .args(["-c", &install_script(&paths, &staged)])
            .status()
            .expect("bash");
        assert!(status.success(), "install script failed: {status}");
        assert_eq!(
            std::fs::read_to_string(paths.dropin()).expect("read"),
            "CPU_BOOST_ON_AC=0\n"
        );
        assert_eq!(
            std::fs::read_to_string(paths.backup()).expect("backup"),
            "CPU_BOOST_ON_AC=1\n"
        );
        assert!(!staged.exists(), "the staged file should be gone");

        let status = std::process::Command::new("bash")
            .args(["-c", &rollback_script(&paths)])
            .status()
            .expect("bash");
        assert!(status.success(), "rollback script failed: {status}");
        assert_eq!(
            std::fs::read_to_string(paths.dropin()).expect("read"),
            "CPU_BOOST_ON_AC=1\n"
        );
        assert!(!paths.backup().exists(), "the backup is consumed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rolling_back_a_first_write_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("tpv-writer-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = DropinPaths::under(&dir);
        std::fs::create_dir_all(paths.dropin().parent().expect("parent")).expect("mkdir");
        let staged = stage(&paths, "CPU_BOOST_ON_AC=0\n").expect("stage");
        assert!(std::process::Command::new("bash")
            .args(["-c", &install_script(&paths, &staged)])
            .status()
            .expect("bash")
            .success());
        assert!(paths.dropin().exists());
        assert!(
            !paths.backup().exists(),
            "nothing to back up on a first write"
        );
        assert!(std::process::Command::new("bash")
            .args(["-c", &rollback_script(&paths)])
            .status()
            .expect("bash")
            .success());
        assert!(
            !paths.dropin().exists(),
            "a first write is undone by deleting it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 2026-11-03: an apply that carried two keys left the operator's drop-in
    /// holding exactly those two and deleted the other four, because the file was
    /// rendered from the changes alone. This is that write, against the writer.
    #[test]
    fn a_write_that_mentions_one_key_keeps_the_keys_it_does_not_mention() {
        const STAMP: &str = "2026-11-03T19:00:00+0700";
        let (root, paths) = scratch("merge-keeps-the-rest");
        std::fs::write(paths.dropin(), OPERATOR_DROPIN).expect("seed");

        let after = next_content(&paths, &[change("CPU_BOOST_ON_BAT", "0")], STAMP)
            .expect("the file the write would install");

        assert!(
            after.contains("CPU_BOOST_ON_BAT=0"),
            "the write lost the key it was handed:\n{after}"
        );
        for pair in [
            "TLP_PROFILE_AC=BAL",
            "TLP_PROFILE_BAT=SAV",
            "TLP_PROFILE_DEFAULT=BAL",
            "MEM_SLEEP_ON_AC=s2idle",
            "MEM_SLEEP_ON_BAT=s2idle",
        ] {
            assert!(
                after.contains(pair),
                "the write dropped {pair}, a key it was never told to touch:\n{after}"
            );
        }
        assert!(
            after.contains("# Written by ThinkPro Vantage"),
            "the header that says who owns the file went missing:\n{after}"
        );
        assert!(
            after.contains(&format!("# last change: {STAMP}")),
            "the last change line is stale:\n{after}"
        );
        assert!(
            !after.contains("last change: restored"),
            "the old last change line is still there:\n{after}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Clearing a key removes it; it does not take its neighbours with it.
    #[test]
    fn a_key_sent_with_an_empty_value_is_removed_and_the_rest_stay() {
        let (root, paths) = scratch("merge-removal");
        std::fs::write(paths.dropin(), OPERATOR_DROPIN).expect("seed");

        let after = next_content(
            &paths,
            &[change("MEM_SLEEP_ON_AC", "")],
            "2026-11-03T19:05:00+0700",
        )
        .expect("the file the write would install");

        assert!(
            !after.contains("MEM_SLEEP_ON_AC"),
            "the cleared key is still in the file:\n{after}"
        );
        for pair in [
            "CPU_BOOST_ON_BAT=1",
            "TLP_PROFILE_AC=BAL",
            "TLP_PROFILE_BAT=SAV",
            "TLP_PROFILE_DEFAULT=BAL",
            "MEM_SLEEP_ON_BAT=s2idle",
        ] {
            assert!(
                after.contains(pair),
                "clearing one key dropped {pair}:\n{after}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A removal cannot be a type error: there is no value to check. It still has
    /// to name a setting the installed TLP knows.
    #[test]
    fn an_empty_value_is_a_removal_for_every_kind() {
        let schema = schema();
        let validated = validate(
            &schema,
            &BTreeMap::new(),
            &[
                change("CPU_SCALING_GOVERNOR_ON_AC", ""),
                change("CPU_BOOST_ON_AC", ""),
                change("CPU_MIN_PERF_ON_AC", ""),
                change("PLATFORM_PROFILE_ON_AC", ""),
            ],
        );
        assert!(validated.rejected.is_empty(), "{:?}", validated.rejected);
        assert_eq!(validated.accepted.len(), 4);
        assert!(validated.accepted.iter().all(Change::is_removal));

        let refused = validate(&schema, &BTreeMap::new(), &[change("NOT_A_PARAMETER", "")]);
        assert!(refused.accepted.is_empty());
        assert_eq!(refused.rejected.len(), 1);
        assert!(refused.rejected[0].reason.contains("no such parameter"));
    }

    /// The operator reads the preview to decide, so the preview has to describe
    /// the file that will be installed rather than the changes that were sent.
    #[test]
    fn the_preview_shows_the_file_the_write_stages() {
        const STAMP: &str = "2026-11-03T19:10:00+0700";
        let (root, paths) = scratch("preview-matches-write");
        std::fs::write(paths.dropin(), OPERATOR_DROPIN).expect("seed");
        let changes = [change("CPU_BOOST_ON_BAT", "0")];

        let staged = stage_next(&paths, &changes, STAMP).expect("stage");
        let installed = std::fs::read_to_string(&staged).expect("read the staged file");
        let previewed = next_content(&paths, &changes, STAMP).expect("the preview");

        assert_eq!(
            previewed, installed,
            "the preview is not the file that gets installed"
        );
        assert!(
            installed.contains("MEM_SLEEP_ON_BAT=s2idle"),
            "what gets installed is not the merge:\n{installed}"
        );

        // The preview the command itself builds, when there is a TLP on this
        // machine to check the changes against: the same settings come out.
        let preview = build_preview(&paths, &changes);
        if preview.available {
            let settings = |text: &str| -> Vec<String> {
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_string)
                    .collect()
            };
            assert_eq!(settings(&preview.content), settings(&installed));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The header is what tells the operator when the file last changed, and it
    /// said "unknown" on every write until this was checked.
    #[test]
    fn the_last_change_line_carries_a_real_timestamp() {
        let stamp = timestamp();
        assert_ne!(stamp, "unknown");
        assert!(
            regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{4}$")
                .expect("pattern")
                .is_match(&stamp),
            "the stamp is not a timestamp: {stamp}"
        );

        let file = render(&[change("CPU_BOOST_ON_BAT", "1")], &stamp);
        assert!(
            file.contains(&format!("# last change: {stamp}")),
            "the header does not report the write:\n{file}"
        );
    }

    /// What the writer puts in the file it can read back and merge again, quoting
    /// and all: without that the second write would not know what to keep.
    #[test]
    fn the_file_it_writes_can_be_read_back_as_settings() {
        let changes = vec![
            change("CPU_BOOST_ON_AC", "0"),
            change("DISK_DEVICES", "nvme0n1 sda"),
        ];
        let content = render(&changes, "2026-11-03T19:00:00+0700");
        assert_eq!(parse_dropin(&content), changes);
    }

    /// The live check, on the machine whose drop-in was emptied on 2026-11-03: a
    /// real write through `apply`, the same code path pkexec runs for the app, and
    /// a real removal after it.
    ///
    /// It writes a setting at the value that setting already has in effect, so
    /// nothing about the machine's behaviour changes while it is in the file, and
    /// clears it afterwards. Ignored by default because it edits
    /// `/etc/tlp.d/99-thinkpro-vantage.conf`; run it as root:
    ///
    /// ```text
    /// sudo target/debug/deps/thinkpro_vantage_lib-<hash> --ignored live_write_keeps --nocapture
    /// ```
    #[test]
    #[ignore = "edits the machine's /etc/tlp.d/99-thinkpro-vantage.conf; needs root and TLP"]
    fn live_write_keeps_the_keys_it_does_not_mention() {
        const OPERATOR_KEYS: [&str; 6] = [
            "CPU_BOOST_ON_BAT",
            "TLP_PROFILE_AC",
            "TLP_PROFILE_BAT",
            "TLP_PROFILE_DEFAULT",
            "MEM_SLEEP_ON_AC",
            "MEM_SLEEP_ON_BAT",
        ];
        let paths = DropinPaths::system();
        let before = std::fs::read_to_string(paths.dropin()).expect("the machine's drop-in");
        for key in OPERATOR_KEYS {
            assert!(
                before.contains(&format!("{key}=")),
                "the drop-in does not hold {key} to begin with:\n{before}"
            );
        }

        // A setting the file does not mention, written at the value it already has
        // in effect: a write that cannot change anything, which is all this test
        // needs to watch the other keys.
        let schema = tlp::tlp_schema().data.expect("TLP's schema");
        let config = tlp::tlp_config().data.expect("TLP's configuration");
        let (key, value) = schema
            .entries
            .iter()
            .filter(|entry| entry.deprecated.is_none() && entry.applies)
            .filter(|entry| !before.contains(&format!("{}=", entry.key)))
            .filter_map(|entry| {
                let value = config
                    .values
                    .get(&entry.key)
                    .cloned()
                    .or_else(|| entry.default_value.clone())?;
                validate(
                    &schema.entries,
                    &schema.renamed,
                    &[change(&entry.key, &value)],
                )
                .accepted
                .first()
                .map(|_| (entry.key.clone(), value))
            })
            .next()
            .expect("a setting whose current value can be written back");

        let applied = apply(&paths, &[change(&key, &value)]).expect("the write");
        assert_eq!(applied.applied.len(), 1, "{:?}", applied.rejected);
        let written = std::fs::read_to_string(paths.dropin()).expect("read the drop-in");
        println!("--- after writing {key}={value} ---\n{written}");
        for name in OPERATOR_KEYS {
            assert!(
                written.contains(&format!("{name}=")),
                "writing {key} dropped {name}:\n{written}"
            );
        }
        assert!(written.contains(&format!("{key}={value}")), "{written}");

        let applied = apply(&paths, &[change(&key, "")]).expect("the removal");
        assert!(applied.applied.iter().all(Change::is_removal));
        let written = std::fs::read_to_string(paths.dropin()).expect("read the drop-in");
        println!("--- after clearing {key} ---\n{written}");
        assert!(
            !written.contains(&format!("{key}=")),
            "the cleared {key} is still in the file:\n{written}"
        );
        for name in OPERATOR_KEYS {
            assert!(
                written.contains(&format!("{name}=")),
                "clearing {key} dropped {name}:\n{written}"
            );
        }

        // The copy Roll back would restore must not carry the key this test
        // invented, or the next roll back would put it back.
        std::fs::copy(paths.dropin(), paths.backup()).expect("tidy the roll-back copy");
    }

    #[test]
    fn too_many_changes_in_one_write_are_refused() {
        let schema = schema();
        let many: Vec<Change> = (0..MAX_CHANGES + 1)
            .map(|_| change("CPU_BOOST_ON_AC", "1"))
            .collect();
        let validated = validate(&schema, &BTreeMap::new(), &many);
        assert!(validated.accepted.is_empty());
        assert!(validated.rejected[0].reason.contains("at most"));
    }
}
