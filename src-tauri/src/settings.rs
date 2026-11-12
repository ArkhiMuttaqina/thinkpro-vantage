use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurvePoint {
    pub temp: i32,
    pub level: i32,
}

/// Which power source a GPU ceiling is held for.
///
/// The ids are the suffixes TLP gives its own per-source keys — `AC`, `BAT`,
/// `SAV` — because the CPU ceilings on the same view are keyed by exactly those,
/// and the two controls have to name the same columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GpuPowerSource {
    /// Mains power.
    Ac,
    /// On battery, with a profile that is not the power-saver one.
    Bat,
    /// The power-saver profile.
    Sav,
}

impl GpuPowerSource {
    /// How this source is named in a message the operator reads.
    pub fn label(self) -> &'static str {
        match self {
            GpuPowerSource::Ac => "on AC",
            GpuPowerSource::Bat => "on battery",
            GpuPowerSource::Sav => "in power saver",
        }
    }
}

/// The GPU clock ceilings this app holds, one per power source.
///
/// Every field is independently absent: a source with no ceiling is "this app
/// holds nothing there", which is a different state from a ceiling of `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuCeilings {
    #[serde(default)]
    pub ac: Option<u32>,
    #[serde(default)]
    pub bat: Option<u32>,
    #[serde(default)]
    pub sav: Option<u32>,
}

impl GpuCeilings {
    /// The ceiling held for `source`, or `None` when this app holds nothing.
    pub fn get(self, source: GpuPowerSource) -> Option<u32> {
        match source {
            GpuPowerSource::Ac => self.ac,
            GpuPowerSource::Bat => self.bat,
            GpuPowerSource::Sav => self.sav,
        }
    }

    /// Hold `level` for `source`, or hold nothing there with `None`.
    pub fn set(&mut self, source: GpuPowerSource, level: Option<u32>) {
        match source {
            GpuPowerSource::Ac => self.ac = level,
            GpuPowerSource::Bat => self.bat = level,
            GpuPowerSource::Sav => self.sav = level,
        }
    }

    /// Whether this holds no ceiling for any source at all.
    pub fn is_empty(self) -> bool {
        self.ac.is_none() && self.bat.is_none() && self.sav.is_none()
    }
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

    /// The GPU clock ceilings this app is holding, one per power source.
    ///
    /// These are *targets*, not readings of the hardware, so unlike every other
    /// field here they are never re-derived from sysfs: they are what the app
    /// re-writes to the GPU after a reboot, after `lactd` takes the card back,
    /// or after the machine moves to a different power source.
    ///
    /// A source left out holds nothing, which is not the same as a ceiling of
    /// `0`: level 0 is a real ceiling — the card's lowest level — and it is
    /// exactly what the battery and power-saver columns of this machine ask for.
    ///
    /// Written by `performance::set_gpu_clock_level`, cleared per source by
    /// `performance::clear_gpu_clock_cap` and wholesale by
    /// `performance::reattach_gpu`.
    #[serde(default)]
    pub gpu_clock_levels: GpuCeilings,

    /// The machine-wide GPU ceiling this replaced, read for migration only.
    ///
    /// A file written when the ceiling was one machine-wide level carries this
    /// key and no `gpu_clock_levels`; [`AppSettings::migrated`] copies it onto
    /// every source. `skip_serializing` is what stops that copy from leaving the
    /// key behind on every save afterwards, so the field retires itself.
    #[serde(default, skip_serializing)]
    pub gpu_clock_level: Option<u32>,

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

            // Nothing is held on a fresh install: the GPU stays LACT's, or the
            // driver's, until a ceiling is chosen here.
            gpu_clock_levels: GpuCeilings::default(),
            gpu_clock_level: None,

            // App defaults
            //
            // On by default: a fresh install is expected to come back at login
            // and re-apply the fan curve and the GPU ceiling, which are only
            // held while the app runs. Autostart is reconciled against this
            // value on every start (see `crate::autostart::reconcile`), so a
            // store written before this default changed keeps its own value --
            // only installs that never saved one get the new default.
            auto_start: true,
            minimize_to_tray: true,
            theme: "system".to_string(),
        }
    }
}

impl AppSettings {
    /// Fold the machine-wide GPU ceiling of an older file into the per-source
    /// form.
    ///
    /// A file written when the ceiling was a single level carries
    /// `gpu_clock_level` and no `gpu_clock_levels`. Keeping that level for every
    /// source is the only reading of it that does not silently drop what the
    /// operator asked for, and it is a reading that holds on this machine: the
    /// level is what the app wrote, and it wrote it regardless of the source.
    ///
    /// The legacy key is then gone for good — it is never serialized again — so
    /// the migration happens once per file and leaves nothing behind. Per-source
    /// values win over it, which is the only case a hand-edited file can present
    /// both keys in.
    pub fn migrated(mut self) -> Self {
        if let Some(level) = self.gpu_clock_level {
            if self.gpu_clock_levels.is_empty() {
                self.gpu_clock_levels = GpuCeilings {
                    ac: Some(level),
                    bat: Some(level),
                    sav: Some(level),
                };
            }
        }
        self
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

/// What the GPU watcher reads from the store on each pass.
///
/// One read for both fields. `load_raw_app_settings` logs a line every time it
/// runs, which is right for the handful of places that load settings and wrong
/// for something that reads two fields every five seconds — see
/// [`crate::performance::gpu_clock_watcher`]. Nothing else in the store is
/// touched, and a store that cannot be read is a fresh install: nothing held, and
/// the default profile.
pub struct GpuWatchState {
    pub ceilings: GpuCeilings,
    pub power_profile: String,
}

pub fn load_gpu_watch_state(app: &AppHandle) -> GpuWatchState {
    let settings = app
        .store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(SETTINGS_KEY))
        .and_then(|stored| serde_json::from_value::<AppSettings>(stored).ok())
        .map(AppSettings::migrated)
        .unwrap_or_default();

    GpuWatchState {
        ceilings: settings.gpu_clock_levels,
        power_profile: settings.power_profile,
    }
}

/// Remember, or forget, the GPU ceilings this app is holding.
///
/// Reads the rest of the file back first, so a ceiling recorded here cannot drop
/// the settings the operator saved from the Settings view. Called only after the
/// write to the GPU it describes has already succeeded: level and hardware state
/// have to move together, or a start would re-apply a ceiling that was refused.
pub async fn save_gpu_ceilings(app: &AppHandle, ceilings: GpuCeilings) -> Result<(), String> {
    let mut settings = load_raw_app_settings(app);
    if settings.gpu_clock_levels == ceilings {
        return Ok(());
    }

    settings.gpu_clock_levels = ceilings;
    save_app_settings(app.clone(), settings).await
}

/// Remember the power profile the app last saw.
///
/// `power_profile` is the profile this app already follows over D-Bus, and the
/// battery columns of the GPU ceiling are told apart by it — so
/// [`crate::performance::power_profile_watcher`] writes the profile it read back
/// here, and the store never shows one that is no longer in force.
pub async fn save_tracked_power_profile(app: &AppHandle, profile: &str) -> Result<(), String> {
    let mut settings = load_raw_app_settings(app);
    if settings.power_profile == profile {
        return Ok(());
    }

    settings.power_profile = profile.to_string();
    save_app_settings(app.clone(), settings).await
}

/// Load raw app settings from persistent storage without system synchronization
pub fn load_raw_app_settings(app: &AppHandle) -> AppSettings {
    match app.store(STORE_FILE) {
        Ok(store) => {
            if let Some(settings_value) = store.get(SETTINGS_KEY) {
                match serde_json::from_value::<AppSettings>(settings_value.clone()) {
                    Ok(settings) => {
                        println!("[Settings] Settings loaded from store");
                        return settings.migrated();
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
    let mut settings: AppSettings = serde_json::from_str(&json_content)
        .map_err(|e| format!("Invalid settings JSON format: {}", e))?;

    validate_settings(&mut settings)?;
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

    // The imported `auto_start` is the new intent; make the login entry match
    // it now rather than at the next start, so the Settings view (which reads
    // the filesystem) shows the truth immediately after the import.
    crate::autostart::reconcile(&app);

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

    // Defaults mean autostart on; the entry has to follow the reset, or the
    // Settings view would immediately show the value it is not enforcing.
    crate::autostart::reconcile(&app);

    println!("[Settings] Settings reset to default values");
    Ok(defaults)
}

/// Validate settings values to protect against corrupted or out-of-range configs
pub fn validate_settings(settings: &mut AppSettings) -> Result<(), String> {
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
        let mut defaults = AppSettings::default();
        assert!(validate_settings(&mut defaults).is_ok());
    }

    #[test]
    fn rejects_invalid_fan_mode() {
        let mut settings = AppSettings {
            fan_mode: "turbo-hyper".to_string(),
            ..Default::default()
        };
        assert!(validate_settings(&mut settings).is_err());
    }

    #[test]
    fn rejects_out_of_range_fan_level() {
        let mut settings = AppSettings {
            fan_level: 9,
            ..Default::default()
        };
        assert!(validate_settings(&mut settings).is_err());
    }

    #[test]
    fn rejects_inverted_battery_thresholds() {
        let mut settings = AppSettings {
            battery_start_threshold: 85,
            battery_stop_threshold: 80,
            ..Default::default()
        };
        assert!(validate_settings(&mut settings).is_err());
    }

    #[test]
    fn rejects_invalid_curve_point_temp() {
        let mut settings = AppSettings {
            fan_curve_points: vec![CurvePoint {
                temp: 150,
                level: 5,
            }],
            ..Default::default()
        };
        assert!(validate_settings(&mut settings).is_err());
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
        let mut settings = AppSettings {
            theme: "neon-glow".to_string(),
            ..Default::default()
        };
        assert!(validate_settings(&mut settings).is_err());
    }

    #[test]
    fn rejects_out_of_bounds_curve_point_level() {
        let mut settings = AppSettings {
            fan_curve_points: vec![CurvePoint { temp: 50, level: 8 }],
            ..Default::default()
        };
        assert!(validate_settings(&mut settings).is_err());
    }

    /// This machine's actual settings.json, written before the per-profile clock
    /// ceilings were removed. `AppSettings` no longer has a `clock_caps` field, so
    /// the loader must ignore the unknown key; a `deny_unknown_fields` anywhere on
    /// the struct would make every field here vanish into `AppSettings::default()`.
    #[test]
    fn an_old_settings_file_with_clock_limits_still_loads() {
        let json = r#"{
            "fan_mode": "auto",
            "fan_level": 0,
            "fan_curve_enabled": false,
            "fan_curve_points": [{ "temp": 40, "level": 1 }, { "temp": 88, "level": 7 }],
            "battery_start_threshold": 95,
            "battery_stop_threshold": 100,
            "clock_caps": [
                {
                    "auto_apply": false,
                    "cpu_max_khz": 1100000,
                    "cpu_min_khz": null,
                    "gpu_max_level": 0,
                    "profile": "power-saver"
                },
                {
                    "auto_apply": false,
                    "cpu_max_khz": 3100000,
                    "cpu_min_khz": null,
                    "gpu_max_level": null,
                    "profile": "balanced"
                },
                {
                    "auto_apply": false,
                    "cpu_max_khz": null,
                    "cpu_min_khz": null,
                    "gpu_max_level": null,
                    "profile": "performance"
                }
            ],
            "auto_start": false,
            "cpu_governor": "powersave",
            "minimize_to_tray": true,
            "power_profile": "power-saver",
            "theme": "system",
            "turbo_boost_enabled": true
        }"#;

        let parsed: AppSettings = serde_json::from_str(json)
            .expect("a settings file written before the change still loads");

        assert_eq!(parsed.power_profile, "power-saver", "nothing else is reset");
        assert_eq!(parsed.cpu_governor, "powersave");
        assert_eq!(parsed.battery_start_threshold, 95);
        assert!(parsed.minimize_to_tray);
        assert!(
            parsed.gpu_clock_levels.is_empty(),
            "a file that predates the GPU ceiling holds none for any source: it \
             must not make the app take the GPU on the next start"
        );
    }

    /// The GPU ceilings have to survive a restart of the app, and the store is how
    /// they do: written per source, read back as the same three, cleared per
    /// source — which is also what the app does when a column is emptied.
    ///
    /// `sav: None` is the point of the shape: an unset source has to come back
    /// unset, and never as a ceiling of 0.
    #[test]
    fn the_gpu_ceilings_round_trip_through_the_settings_file() {
        let ceilings = GpuCeilings {
            ac: Some(2),
            bat: Some(0),
            sav: None,
        };
        let mut settings = AppSettings {
            gpu_clock_levels: ceilings,
            ..Default::default()
        };
        validate_settings(&mut settings).expect("a GPU level is not out of range");

        let json = serde_json::to_string(&settings).expect("settings serialize");
        let parsed: AppSettings = serde_json::from_str(&json).expect("settings parse back");

        assert_eq!(parsed.gpu_clock_levels, ceilings);
    }

    /// A ceiling set when it was one machine-wide level survives the change to
    /// per-source ceilings with its value intact: every source keeps it, because
    /// the app held it whatever the machine was plugged into.
    ///
    /// The old key then retires itself. A file that kept writing it would migrate
    /// again on every load, which is how a cleared ceiling would come back.
    #[test]
    fn a_machine_wide_gpu_ceiling_migrates_onto_every_source_once() {
        let mut legacy = serde_json::to_value(AppSettings::default()).expect("settings serialize");
        legacy["gpu_clock_level"] = serde_json::json!(1);

        let parsed: AppSettings =
            serde_json::from_value(legacy).expect("a file with the old key still loads");
        let migrated = parsed.migrated();

        assert_eq!(
            migrated.gpu_clock_levels,
            GpuCeilings {
                ac: Some(1),
                bat: Some(1),
                sav: Some(1),
            },
            "the level held machine-wide is the level every source holds"
        );

        let saved = serde_json::to_value(&migrated).expect("the migrated settings serialize");
        assert_eq!(saved["gpu_clock_levels"]["ac"], serde_json::json!(1));
        assert!(
            saved.get("gpu_clock_level").is_none(),
            "the legacy key must not be written again"
        );
    }

    /// Per-source values win over the legacy key, which is the only shape a
    /// hand-edited file can carry both in. A source left out of the per-source
    /// object stays empty rather than inheriting the old machine-wide level: the
    /// operator has spoken about the columns, and an absent one is a decision.
    #[test]
    fn per_source_gpu_ceilings_win_over_the_legacy_key() {
        let mut both = serde_json::to_value(AppSettings::default()).expect("settings serialize");
        both["gpu_clock_level"] = serde_json::json!(1);
        both["gpu_clock_levels"] = serde_json::json!({ "ac": 2 });

        let parsed: AppSettings = serde_json::from_value(both).expect("settings parse");
        let merged = parsed.migrated();

        assert_eq!(merged.gpu_clock_levels.ac, Some(2));
        assert_eq!(
            merged.gpu_clock_levels.bat, None,
            "a source left out of the object holds nothing"
        );
    }
}
