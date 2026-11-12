//! XDG autostart: the "start" half of the app's start-and-reapply story.
//!
//! Everything the app owns stops at logout. The fan curve and the GPU clock
//! ceiling are only held while this process runs (sysfs is not persistent, and
//! exiting hands the fan back to firmware on purpose), so unless the app comes
//! back at login every one of those settings silently lapses until the operator
//! launches it by hand.
//!
//! The mechanism is the one every desktop session already implements: a desktop
//! entry in `${XDG_CONFIG_HOME:-$HOME/.config}/autostart`. Not a systemd user
//! unit — a session app wants the session bus, the tray, and the operator's
//! display, and a unit that starts before the session exists has none of them.
//! Not a plugin either: this is a fixed INI file written by three functions.
//!
//! The entry deliberately mirrors the packaged desktop file
//! (`/usr/share/applications/thinkpro-vantage.desktop`): same file name, and
//! its `Exec` line is reused verbatim when that file is present, so the entry
//! follows however this build is installed instead of hardcoding a path that
//! drifts. `--autostart` is appended so the app can tell a login launch from a
//! manual one and start hidden in the tray (see `lib.rs`).
//!
//! Two sources of truth meet here and they are not allowed to disagree:
//!
//! * the store's `auto_start` field is the operator's *intent*, and
//! * the entry on disk is the *state*.
//!
//! `reconcile` runs on every start and forces the state to match the intent in
//! both directions, which is what makes a hand-deleted entry come back (intent
//! says on) and a stale entry left behind by an older version go away (intent
//! says off). The Settings view then reads the state, not the intent, so the
//! toggle can never claim something the filesystem does not show.

use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

/// Shared with the packaged desktop file so the autostart entry and
/// `/usr/share/applications/thinkpro-vantage.desktop` are obviously the same app.
pub const ENTRY_FILE_NAME: &str = "thinkpro-vantage.desktop";

/// The packaged desktop file whose `Exec` this entry reuses when it exists.
const PACKAGED_DESKTOP_FILE: &str = "/usr/share/applications/thinkpro-vantage.desktop";

/// Marks a launch that came from the autostart entry rather than from the user.
/// `lib.rs` uses it to start hidden instead of stealing focus at login.
pub const AUTOSTART_FLAG: &str = "--autostart";

/// GNOME's own delay key, in seconds. A login launch has no operator watching
/// it, so a few seconds of session settling costs nothing, and starting before
/// the session bus is up would cost the tray icon.
const AUTOSTART_DELAY_SECONDS: u32 = 3;

#[derive(Debug, Clone, Serialize)]
pub struct AutostartStatus {
    /// Whether the entry exists on disk. This is what the UI displays.
    pub enabled: bool,
    pub entry_path: String,
    /// The `Exec` the entry currently carries, or would carry.
    pub exec: String,
}

/// Where the session looks for per-user autostart entries.
pub fn autostart_dir() -> PathBuf {
    autostart_dir_from(
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )
}

fn autostart_dir_from(xdg_config_home: Option<String>, home: Option<String>) -> PathBuf {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(xdg).join("autostart");
    }
    let home = home
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| ".".to_string());
    PathBuf::from(home).join(".config/autostart")
}

pub fn entry_path() -> PathBuf {
    autostart_dir().join(ENTRY_FILE_NAME)
}

/// The state, read from the filesystem. Never consults the store: the store is
/// intent, and a UI that showed intent here could claim an entry that is not
/// there.
pub fn is_enabled() -> bool {
    entry_path().is_file()
}

/// Whether this process was launched by the autostart entry.
pub fn launched_from_autostart() -> bool {
    std::env::args().any(|arg| arg == AUTOSTART_FLAG)
}

/// Pull the `Exec` value out of a desktop file's text.
fn parse_exec(desktop_entry: &str) -> Option<String> {
    desktop_entry.lines().find_map(|line| {
        line.strip_prefix("Exec=")
            .map(str::trim)
            .filter(|exec| !exec.is_empty())
            .map(str::to_string)
    })
}

/// The command line a login launch runs: the packaged desktop file's own
/// `Exec` when it exists, else this binary.
fn base_exec() -> String {
    if let Some(exec) = std::fs::read_to_string(PACKAGED_DESKTOP_FILE)
        .ok()
        .as_deref()
        .and_then(parse_exec)
    {
        return exec;
    }
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "thinkpro-vantage".to_string())
}

fn with_autostart_flag(exec: &str) -> String {
    if exec.split_whitespace().any(|part| part == AUTOSTART_FLAG) {
        return exec.to_string();
    }
    format!("{exec} {AUTOSTART_FLAG}")
}

fn desired_exec() -> String {
    with_autostart_flag(&base_exec())
}

/// The entry as a desktop file. `StartupNotify` is off on purpose: it drives
/// the launch-feedback animation and, for an autostart entry, only invites the
/// window manager to raise something nobody asked to see.
fn render_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=ThinkPro Vantage\n\
         Comment=ThinkPad utilities for Linux - Fan control, battery management, and system monitoring\n\
         Exec={exec}\n\
         Icon=thinkpro-vantage\n\
         Terminal=false\n\
         Categories=Settings;HardwareSettings;\n\
         Keywords=thinkpad;fan;battery;performance;monitor;\n\
         StartupNotify=false\n\
         StartupWMClass=thinkpro-vantage\n\
         X-GNOME-Autostart-enabled=true\n\
         X-GNOME-Autostart-Delay={AUTOSTART_DELAY_SECONDS}\n"
    )
}

pub fn status() -> AutostartStatus {
    let path = entry_path();
    let enabled = path.is_file();
    let exec = if enabled {
        std::fs::read_to_string(&path)
            .ok()
            .as_deref()
            .and_then(parse_exec)
            .unwrap_or_else(desired_exec)
    } else {
        desired_exec()
    };
    AutostartStatus {
        enabled,
        entry_path: path.to_string_lossy().into_owned(),
        exec,
    }
}

/// Write the entry. Idempotent: a second call overwrites the same file rather
/// than adding a second entry, which is the only way `autostart` can behave.
pub fn enable() -> Result<AutostartStatus, String> {
    let dir = autostart_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
    let exec = desired_exec();
    std::fs::write(entry_path(), render_entry(&exec))
        .map_err(|e| format!("Failed to write autostart entry: {}", e))?;
    println!("[thinkpro-vantage] autostart: entry written ({exec})");
    Ok(status())
}

/// Remove the entry. A missing file is success, not an error: the caller asked
/// for a state, and that state is already the one on disk.
pub fn disable() -> Result<AutostartStatus, String> {
    let path = entry_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
        println!("[thinkpro-vantage] autostart: entry removed");
    }
    Ok(status())
}

/// Make the filesystem match the stored intent, in both directions.
///
/// Called once from `lib.rs` setup so the setting is true the first time the
/// app is ever run (a fresh install has no store, so `auto_start` defaults to
/// true) and self-heals afterwards. Failures are logged, never propagated: a
/// read-only home directory must not stop the fan curve from starting.
pub fn reconcile(app: &AppHandle) {
    let intended = crate::settings::load_raw_app_settings(app).auto_start;
    let present = is_enabled();
    let result = match (intended, present) {
        (true, false) => enable().map(|_| "enabled from saved setting"),
        (false, true) => disable().map(|_| "removed (setting is off)"),
        (true, true) => Ok("already enabled"),
        (false, false) => Ok("already disabled"),
    };
    match result {
        Ok(action) => println!(
            "[thinkpro-vantage] autostart: {} ({action})",
            entry_path().display()
        ),
        Err(e) => eprintln!("[thinkpro-vantage] autostart reconcile failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The entry's real state, for the Settings view.
#[tauri::command]
pub fn get_autostart_status() -> AutostartStatus {
    status()
}

/// Create or remove the entry, and record the resulting state as the intent so
/// the next start reconciles in the same direction instead of undoing it.
#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> Result<AutostartStatus, String> {
    let status = if enabled { enable()? } else { disable()? };
    crate::settings::update_setting(
        app,
        "auto_start".to_string(),
        serde_json::Value::Bool(status.enabled),
    )
    .await?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exec_reads_the_value_and_ignores_the_rest() {
        let text = "[Desktop Entry]\nType=Application\nName=ThinkPro Vantage\nExec=thinkpro-vantage\nIcon=thinkpro-vantage\n";
        assert_eq!(parse_exec(text), Some("thinkpro-vantage".to_string()));
    }

    #[test]
    fn parse_exec_is_none_without_an_exec_line() {
        assert_eq!(parse_exec("[Desktop Entry]\nType=Application\n"), None);
        assert_eq!(parse_exec("Exec=   \n"), None);
    }

    #[test]
    fn with_autostart_flag_appends_once() {
        assert_eq!(
            with_autostart_flag("thinkpro-vantage"),
            "thinkpro-vantage --autostart"
        );
        assert_eq!(
            with_autostart_flag("thinkpro-vantage --autostart"),
            "thinkpro-vantage --autostart"
        );
    }

    #[test]
    fn rendered_entry_is_a_valid_autostart_desktop_file() {
        let entry = render_entry("thinkpro-vantage --autostart");
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nType=Application\n"));
        assert!(entry.contains("\nExec=thinkpro-vantage --autostart\n"));
        assert!(entry.contains("\nX-GNOME-Autostart-enabled=true\n"));
        // GNOME reads the file out of $XDG_CONFIG_HOME/autostart, so the name
        // has to match the packaged desktop file's.
        assert_eq!(ENTRY_FILE_NAME, "thinkpro-vantage.desktop");
    }

    #[test]
    fn autostart_dir_prefers_xdg_config_home() {
        assert_eq!(
            autostart_dir_from(Some("/xdg".to_string()), Some("/home/op".to_string())),
            PathBuf::from("/xdg/autostart")
        );
        // An empty XDG_CONFIG_HOME is not a directory, it is an unset variable.
        assert_eq!(
            autostart_dir_from(Some("".to_string()), Some("/home/op".to_string())),
            PathBuf::from("/home/op/.config/autostart")
        );
        assert_eq!(
            autostart_dir_from(None, Some("/home/op".to_string())),
            PathBuf::from("/home/op/.config/autostart")
        );
    }
}
