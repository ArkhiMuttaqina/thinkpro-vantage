use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurvePoint {
    pub temp: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    // Fan Control
    pub fan_mode: String,
    pub fan_level: i32,
    pub fan_curve_enabled: bool,
    pub fan_curve_points: Vec<CurvePoint>,

    // Battery
    pub battery_start_threshold: i32,
    pub battery_stop_threshold: i32,

    // Performance
    pub cpu_governor: String,
    pub turbo_boost_enabled: bool,
    pub power_profile: String,

    // App Settings
    pub auto_start: bool,
    pub minimize_to_tray: bool,
    pub theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            // Fan Control defaults
            fan_mode: "auto".to_string(),
            fan_level: 0,
            fan_curve_enabled: false,
            fan_curve_points: vec![
                CurvePoint { temp: 40, level: 0 },
                CurvePoint { temp: 50, level: 1 },
                CurvePoint { temp: 60, level: 3 },
                CurvePoint { temp: 70, level: 5 },
                CurvePoint { temp: 80, level: 7 },
            ],

            // Battery defaults
            battery_start_threshold: 40,
            battery_stop_threshold: 80,

            // Performance defaults
            cpu_governor: "powersave".to_string(),
            turbo_boost_enabled: true,
            power_profile: "balanced".to_string(),

            // App defaults
            auto_start: false,
            minimize_to_tray: true,
            theme: "system".to_string(),
        }
    }
}

const STORE_FILE: &str = "settings.json";
const SETTINGS_KEY: &str = "app_settings";

/// Save all app settings to persistent storage
#[tauri::command]
pub async fn save_app_settings(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("Failed to get store: {}", e))?;

    let settings_json = serde_json::to_value(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    store.set(SETTINGS_KEY, settings_json);
    store
        .save()
        .map_err(|e| format!("Failed to save store: {}", e))?;

    println!("[Settings] All settings saved to store");
    Ok(())
}

/// Load raw app settings from persistent storage without system synchronization
pub fn load_raw_app_settings(app: &AppHandle) -> AppSettings {
    match app.store(STORE_FILE) {
        Ok(store) => {
            if let Some(settings_value) = store.get(SETTINGS_KEY) {
                match serde_json::from_value::<AppSettings>(settings_value.clone()) {
                    Ok(settings) => {
                        println!("[Settings] Settings loaded from store");
                        return settings;
                    }
                    Err(e) => {
                        eprintln!("[Settings] Failed to deserialize settings: {}", e);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[Settings] Failed to get store: {}", e);
        }
    }

    println!("[Settings] Using default settings");
    AppSettings::default()
}

/// Synchronize settings with live hardware, kernel sysfs, and background supervisor state
pub fn sync_settings_with_system(app: &AppHandle, mut settings: AppSettings) -> AppSettings {
    // 1. Fan curve & supervisor mode synchronization
    let curve_config = crate::fan_curve::load_config_from_store(app);
    if !curve_config.points.is_empty() {
        settings.fan_curve_points = curve_config
            .points
            .into_iter()
            .map(|p| CurvePoint {
                temp: p.temp,
                level: p.level,
            })
            .collect();
    }
    settings.fan_curve_enabled = curve_config.enabled;

    if curve_config.enabled {
        settings.fan_mode = "curve".to_string();
    } else if let Some(fan_state) = app.try_state::<crate::fan_curve::FanCurveState>() {
        if let Ok(s) = fan_state.lock() {
            match &s.mode {
                crate::fan_curve::FanTargetMode::Auto => {
                    settings.fan_mode = "auto".to_string();
                }
                crate::fan_curve::FanTargetMode::FullSpeed => {
                    settings.fan_mode = "full".to_string();
                }
                crate::fan_curve::FanTargetMode::Manual(lvl) => {
                    settings.fan_mode = "manual".to_string();
                    if let Ok(l) = lvl.parse::<i32>() {
                        settings.fan_level = l;
                    }
                }
                crate::fan_curve::FanTargetMode::Curve => {
                    settings.fan_mode = "curve".to_string();
                    settings.fan_curve_enabled = true;
                }
            }
        }
    } else if let Ok(proc_fan) = crate::hardware_root::read_to_string("/proc/acpi/ibm/fan") {
        for line in proc_fan.lines() {
            if let Some(rest) = line.strip_prefix("level:") {
                let lvl = rest.trim();
                if lvl == "disengaged" || lvl == "full-speed" {
                    settings.fan_mode = "full".to_string();
                } else if lvl == "auto" {
                    settings.fan_mode = "auto".to_string();
                } else if let Ok(n) = lvl.parse::<i32>() {
                    settings.fan_mode = "manual".to_string();
                    settings.fan_level = n;
                }
                break;
            }
        }
    }

    // 2. Battery charge thresholds from live sysfs
    let bat_resp = crate::battery::get_battery_thresholds();
    if bat_resp.success {
        if let Some(thresholds) = bat_resp.data {
            if thresholds.stop > 0 {
                settings.battery_start_threshold = thresholds.start as i32;
                settings.battery_stop_threshold = thresholds.stop as i32;
            }
        }
    }

    // 3. CPU governor from live sysfs
    let cpu_resp = crate::performance::get_cpu_info();
    if cpu_resp.success {
        if let Some(cpu_info) = cpu_resp.data {
            if !cpu_info.governor.is_empty() {
                settings.cpu_governor = cpu_info.governor;
            }
        }
    }

    // 4. Turbo Boost status from live sysfs
    let turbo_resp = crate::performance::get_turbo_boost_status();
    if turbo_resp.success {
        if let Some(turbo_enabled) = turbo_resp.data {
            settings.turbo_boost_enabled = turbo_enabled;
        }
    }

    // 5. Power Profile from powerprofilesctl / TLP
    let profile_resp = crate::performance::get_power_profile();
    if profile_resp.success {
        if let Some(profile_data) = profile_resp.data {
            if !profile_data.current.is_empty() {
                settings.power_profile = profile_data.current;
            }
        }
    }

    settings
}

/// Load all app settings from persistent storage and synchronize with live hardware
#[tauri::command]
pub async fn load_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    let raw = load_raw_app_settings(&app);
    let synced = sync_settings_with_system(&app, raw);
    // Persist synced settings so cache stays up-to-date
    let _ = save_app_settings(app, synced.clone()).await;
    Ok(synced)
}

/// Update specific setting field
#[tauri::command]
pub async fn update_setting(
    app: AppHandle,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    // Load current settings
    let mut settings = load_app_settings(app.clone()).await?;

    // Update the specific field
    match key.as_str() {
        "fan_mode" => {
            settings.fan_mode = value.as_str().ok_or("Invalid fan_mode value")?.to_string();
        }
        "fan_level" => {
            settings.fan_level = value.as_i64().ok_or("Invalid fan_level value")? as i32;
        }
        "fan_curve_enabled" => {
            settings.fan_curve_enabled =
                value.as_bool().ok_or("Invalid fan_curve_enabled value")?;
        }
        "fan_curve_points" => {
            let points: Vec<CurvePoint> = serde_json::from_value(value)
                .map_err(|e| format!("Invalid fan_curve_points: {}", e))?;
            settings.fan_curve_points = points.clone();
            // Also sync to fan_curve store
            let mut curve_cfg = crate::fan_curve::load_config_from_store(&app);
            curve_cfg.points = points
                .into_iter()
                .map(|p| crate::fan_curve::CurvePoint {
                    temp: p.temp,
                    level: p.level,
                })
                .collect();
            let _ = crate::fan_curve::save_config_to_store(&app, &curve_cfg);
        }
        "battery_start_threshold" => {
            settings.battery_start_threshold = value
                .as_i64()
                .ok_or("Invalid battery_start_threshold value")?
                as i32;
        }
        "battery_stop_threshold" => {
            settings.battery_stop_threshold = value
                .as_i64()
                .ok_or("Invalid battery_stop_threshold value")?
                as i32;
        }
        "cpu_governor" => {
            settings.cpu_governor = value
                .as_str()
                .ok_or("Invalid cpu_governor value")?
                .to_string();
        }
        "turbo_boost_enabled" => {
            settings.turbo_boost_enabled =
                value.as_bool().ok_or("Invalid turbo_boost_enabled value")?;
        }
        "power_profile" => {
            settings.power_profile = value
                .as_str()
                .ok_or("Invalid power_profile value")?
                .to_string();
        }
        "theme" => {
            settings.theme = value.as_str().ok_or("Invalid theme value")?.to_string();
        }
        "auto_start" => {
            settings.auto_start = value.as_bool().ok_or("Invalid auto_start value")?;
        }
        "minimize_to_tray" => {
            settings.minimize_to_tray = value.as_bool().ok_or("Invalid minimize_to_tray value")?;
        }
        _ => return Err(format!("Unknown setting key: {}", key)),
    }

    // Save updated settings
    save_app_settings(app, settings).await
}

/// Export all settings to a pretty-printed JSON string, fully synced with live system state
#[tauri::command]
pub async fn export_settings(app: AppHandle) -> Result<String, String> {
    let settings = load_app_settings(app).await?;
    serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))
}

/// Import, validate and save settings from a JSON string
#[tauri::command]
pub async fn import_settings(app: AppHandle, json_content: String) -> Result<AppSettings, String> {
    let settings: AppSettings = serde_json::from_str(&json_content)
        .map_err(|e| format!("Invalid settings JSON format: {}", e))?;

    validate_settings(&settings)?;
    save_app_settings(app.clone(), settings.clone()).await?;

    // Also sync to fan_curve store if points are included
    if !settings.fan_curve_points.is_empty() {
        let curve_config = crate::fan_curve::FanCurveConfig {
            enabled: settings.fan_curve_enabled,
            points: settings
                .fan_curve_points
                .iter()
                .map(|p| crate::fan_curve::CurvePoint {
                    temp: p.temp,
                    level: p.level,
                })
                .collect(),
        };
        let _ = crate::fan_curve::save_config_to_store(&app, &curve_config);
    }

    println!("[Settings] Settings successfully imported and saved");
    Ok(settings)
}

/// Reset settings to defaults
#[tauri::command]
pub async fn reset_settings(app: AppHandle) -> Result<AppSettings, String> {
    let defaults = AppSettings::default();
    save_app_settings(app.clone(), defaults.clone()).await?;

    // Also reset fan curve config in store
    let default_curve = crate::fan_curve::FanCurveConfig::default();
    let _ = crate::fan_curve::save_config_to_store(&app, &default_curve);

    println!("[Settings] Settings reset to default values");
    Ok(defaults)
}

/// Validate settings values to protect against corrupted or out-of-range configs
pub fn validate_settings(settings: &AppSettings) -> Result<(), String> {
    // Validate fan mode
    match settings.fan_mode.as_str() {
        "auto" | "manual" | "curve" | "full" => {}
        other => {
            return Err(format!(
                "Invalid fan mode '{}'. Must be 'auto', 'manual', 'curve', or 'full'.",
                other
            ))
        }
    }

    // Validate fan level
    if !(0..=7).contains(&settings.fan_level) {
        return Err(format!(
            "Invalid fan level {}. Must be between 0 and 7.",
            settings.fan_level
        ));
    }

    // Validate battery thresholds
    if !(0..=100).contains(&settings.battery_start_threshold) {
        return Err(format!(
            "Invalid battery start threshold {}. Must be between 0 and 100.",
            settings.battery_start_threshold
        ));
    }
    if !(0..=100).contains(&settings.battery_stop_threshold) {
        return Err(format!(
            "Invalid battery stop threshold {}. Must be between 0 and 100.",
            settings.battery_stop_threshold
        ));
    }
    if settings.battery_stop_threshold > 0
        && settings.battery_start_threshold > settings.battery_stop_threshold
    {
        return Err(format!(
            "Battery start threshold ({}) cannot be greater than stop threshold ({}).",
            settings.battery_start_threshold, settings.battery_stop_threshold
        ));
    }

    // Validate fan curve points
    for (i, p) in settings.fan_curve_points.iter().enumerate() {
        if !(0..=120).contains(&p.temp) {
            return Err(format!(
                "Invalid temperature {}°C at curve point {}. Must be between 0 and 120°C.",
                p.temp,
                i + 1
            ));
        }
        if !(0..=7).contains(&p.level) {
            return Err(format!(
                "Invalid fan level {} at curve point {}. Must be between 0 and 7.",
                p.level,
                i + 1
            ));
        }
    }

    // Validate theme
    match settings.theme.as_str() {
        "system" | "dark" | "light" => {}
        other => {
            return Err(format!(
                "Invalid theme '{}'. Must be 'system', 'dark', or 'light'.",
                other
            ))
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_pass_validation() {
        let defaults = AppSettings::default();
        assert!(validate_settings(&defaults).is_ok());
    }

    #[test]
    fn rejects_invalid_fan_mode() {
        let settings = AppSettings {
            fan_mode: "turbo-hyper".to_string(),
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn rejects_out_of_range_fan_level() {
        let settings = AppSettings {
            fan_level: 9,
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn rejects_inverted_battery_thresholds() {
        let settings = AppSettings {
            battery_start_threshold: 85,
            battery_stop_threshold: 80,
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn rejects_invalid_curve_point_temp() {
        let settings = AppSettings {
            fan_curve_points: vec![CurvePoint {
                temp: 150,
                level: 5,
            }],
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn serializes_and_deserializes_settings_roundtrip() {
        let original = AppSettings::default();
        let json = serde_json::to_string_pretty(&original).expect("serializes to json");
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserializes from json");
        assert_eq!(original.fan_mode, parsed.fan_mode);
        assert_eq!(
            original.battery_stop_threshold,
            parsed.battery_stop_threshold
        );
    }

    #[test]
    fn rejects_invalid_theme() {
        let settings = AppSettings {
            theme: "neon-glow".to_string(),
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn rejects_out_of_bounds_curve_point_level() {
        let settings = AppSettings {
            fan_curve_points: vec![CurvePoint { temp: 50, level: 8 }],
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());
    }
}
