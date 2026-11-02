use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

pub const EXTENSION_UUID: &str = "thinkutils@tysontranthai.site";
const METADATA_JSON: &str =
    include_str!("../../gnome-extension/thinkutils@tysontranthai.site/metadata.json");
const EXTENSION_JS: &str =
    include_str!("../../gnome-extension/thinkutils@tysontranthai.site/extension.js");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GnomeExtensionStatus {
    pub is_gnome: bool,
    pub is_installed: bool,
    pub is_enabled: bool,
    pub is_active: bool,
    pub needs_restart: bool,
    pub shell_version: Option<String>,
    pub extension_version: Option<i32>,
    pub install_path: String,
    pub message: Option<String>,
}

pub fn get_extension_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local/share/gnome-shell/extensions")
        .join(EXTENSION_UUID)
}

pub fn detect_is_gnome() -> bool {
    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
        if desktop.to_uppercase().contains("GNOME") {
            return true;
        }
    }
    Command::new("which")
        .arg("gnome-shell")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn get_shell_version() -> Option<String> {
    let output = Command::new("gnome-shell").arg("--version").output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        text.split_whitespace().last().map(|s| s.trim().to_string())
    } else {
        None
    }
}

pub fn parse_enabled_extensions(text: &str) -> Vec<String> {
    let mut list = Vec::new();
    let cleaned = text.replace(['[', ']'], "").replace("@as", "");
    for part in cleaned.split(',') {
        let item = part.trim().trim_matches('\'').trim_matches('"').trim();
        if !item.is_empty() {
            list.push(item.to_string());
        }
    }
    list
}

pub fn get_enabled_extensions() -> Vec<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            return parse_enabled_extensions(&text);
        }
    }
    Vec::new()
}

pub fn set_enabled_extensions(list: &[String]) -> Result<(), String> {
    let formatted = if list.is_empty() {
        "@as []".to_string()
    } else {
        format!(
            "[{}]",
            list.iter()
                .map(|s| format!("'{}'", s))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let output = Command::new("gsettings")
        .args(["set", "org.gnome.shell", "enabled-extensions", &formatted])
        .output()
        .map_err(|e| format!("Failed to update gsettings: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gsettings error: {}", err));
    }
    Ok(())
}

pub fn check_is_active() -> bool {
    let output = Command::new("gnome-extensions")
        .args(["info", EXTENSION_UUID])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            return text.contains("State: ACTIVE") || text.contains("State: 1");
        }
    }
    false
}

#[tauri::command]
pub fn get_gnome_extension_status() -> Result<GnomeExtensionStatus, String> {
    let ext_dir = get_extension_dir();
    let is_gnome = detect_is_gnome();
    let shell_version = get_shell_version();
    let is_installed =
        ext_dir.join("metadata.json").exists() && ext_dir.join("extension.js").exists();

    let enabled_list = get_enabled_extensions();
    let is_enabled = enabled_list.iter().any(|id| id == EXTENSION_UUID);
    let is_active = check_is_active();

    // If installed and enabled, but gnome-extensions info reports not active, Wayland needs re-login
    let needs_restart = is_installed && is_enabled && !is_active;

    let message = if !is_gnome {
        Some("GNOME Desktop not detected. This indicator is designed for GNOME Shell.".to_string())
    } else if !is_installed {
        Some("Top panel extension is not yet installed.".to_string())
    } else if !is_enabled {
        Some("Top panel extension is installed but disabled.".to_string())
    } else if needs_restart {
        Some(
            "Extension is enabled! Please log out and back in to display it on the GNOME top bar (Wayland session restart required)."
                .to_string(),
        )
    } else {
        Some("Top panel extension is active and displaying live hardware stats.".to_string())
    };

    Ok(GnomeExtensionStatus {
        is_gnome,
        is_installed,
        is_enabled,
        is_active,
        needs_restart,
        shell_version,
        extension_version: Some(1),
        install_path: ext_dir.to_string_lossy().to_string(),
        message,
    })
}

#[tauri::command]
pub fn install_gnome_extension() -> Result<GnomeExtensionStatus, String> {
    let ext_dir = get_extension_dir();
    std::fs::create_dir_all(&ext_dir)
        .map_err(|e| format!("Failed to create extension directory: {}", e))?;

    std::fs::write(ext_dir.join("metadata.json"), METADATA_JSON)
        .map_err(|e| format!("Failed to write metadata.json: {}", e))?;

    std::fs::write(ext_dir.join("extension.js"), EXTENSION_JS)
        .map_err(|e| format!("Failed to write extension.js: {}", e))?;

    // Enable extension after writing files
    enable_gnome_extension(true)
}

#[tauri::command]
pub fn enable_gnome_extension(enable: bool) -> Result<GnomeExtensionStatus, String> {
    let ext_dir = get_extension_dir();
    if enable && (!ext_dir.join("metadata.json").exists() || !ext_dir.join("extension.js").exists())
    {
        return install_gnome_extension();
    }

    let mut enabled_list = get_enabled_extensions();
    if enable {
        if !enabled_list.iter().any(|id| id == EXTENSION_UUID) {
            enabled_list.push(EXTENSION_UUID.to_string());
            let _ = set_enabled_extensions(&enabled_list);
        }
        let _ = Command::new("gnome-extensions")
            .args(["enable", EXTENSION_UUID])
            .output();
    } else {
        enabled_list.retain(|id| id != EXTENSION_UUID);
        let _ = set_enabled_extensions(&enabled_list);
        let _ = Command::new("gnome-extensions")
            .args(["disable", EXTENSION_UUID])
            .output();
    }

    get_gnome_extension_status()
}

#[tauri::command]
pub fn uninstall_gnome_extension() -> Result<GnomeExtensionStatus, String> {
    let _ = enable_gnome_extension(false);
    let ext_dir = get_extension_dir();
    if ext_dir.exists() {
        let _ = std::fs::remove_dir_all(&ext_dir);
    }
    get_gnome_extension_status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_json_is_valid() {
        let parsed: serde_json::Value =
            serde_json::from_str(METADATA_JSON).expect("metadata.json is valid JSON");
        assert_eq!(
            parsed["uuid"].as_str(),
            Some(EXTENSION_UUID),
            "UUID in metadata.json must match EXTENSION_UUID"
        );
        assert!(parsed["shell-version"].is_array());
    }

    #[test]
    fn extension_js_has_required_lifecycle_methods() {
        assert!(
            EXTENSION_JS.contains("class ThinkUtilsExtension extends Extension"),
            "Extension class definition missing"
        );
        assert!(EXTENSION_JS.contains("enable()"), "enable() missing");
        assert!(EXTENSION_JS.contains("disable()"), "disable() missing");
        assert!(
            EXTENSION_JS.contains("addToStatusArea"),
            "addToStatusArea missing"
        );
    }

    #[test]
    fn parse_enabled_extensions_handles_empty_and_populated_arrays() {
        let empty = parse_enabled_extensions("@as []");
        assert!(empty.is_empty());

        let single = parse_enabled_extensions("['thinkutils@tysontranthai.site']");
        assert_eq!(single, vec!["thinkutils@tysontranthai.site"]);

        let multiple =
            parse_enabled_extensions("['caffeine@patapon.info', 'thinkutils@tysontranthai.site']");
        assert_eq!(
            multiple,
            vec!["caffeine@patapon.info", "thinkutils@tysontranthai.site"]
        );
    }
}
