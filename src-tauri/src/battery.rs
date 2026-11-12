use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const BAT0_PATH: &str = "/sys/class/power_supply/BAT0";
const BAT1_PATH: &str = "/sys/class/power_supply/BAT1";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BatteryInfo {
    pub name: String,
    pub status: String,
    pub capacity: u8,
    pub health: u8,
    pub cycles: u32,
    pub voltage: f32,
    pub current: f32,
    pub power: f32,
    pub energy_now: f32,
    pub energy_full: f32,
    pub energy_design: f32,
    pub technology: String,
    pub manufacturer: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatteryThresholds {
    pub start: u8,
    pub stop: u8,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn get_battery_info() -> ApiResponse<Vec<BatteryInfo>> {
    let mut batteries = Vec::new();

    for (index, bat_path) in [BAT0_PATH, BAT1_PATH].iter().enumerate() {
        if Path::new(bat_path).exists() {
            match read_battery_info(bat_path, index) {
                Ok(info) => batteries.push(info),
                Err(e) => println!("[Battery] Error reading {}: {}", bat_path, e),
            }
        }
    }

    if batteries.is_empty() {
        return ApiResponse {
            success: false,
            data: None,
            error: Some("No batteries found".to_string()),
        };
    }

    ApiResponse {
        success: true,
        data: Some(batteries),
        error: None,
    }
}

/// Instantaneous power of one battery pack, in watts.
///
/// `power_now` is the kernel's own measurement and is preferred whenever it is
/// present and non-zero. It is not always usable: some packs report the file as
/// absent, and some report a permanent 0 (this AMD ThinkPad exposes
/// `power_now` but no `current_now` at all). Treating that 0 as the reading
/// would silently claim the battery is not moving charge, so the
/// `voltage_now x current_now` pair is the fallback.
///
/// The units are micro: µV x µA is picowatts, hence the 1e12 divisor. A pack
/// that exposes neither source is honestly 0.0 W — which is also the correct
/// reading for a full or idle pack sitting on AC.
fn power_watts(path: &str) -> f32 {
    let micro = |file: &str| -> Option<f32> {
        fs::read_to_string(format!("{}/{}", path, file))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
    };

    match micro("power_now") {
        Some(p) if p > 0.0 => p / 1_000_000.0,
        _ => match (micro("voltage_now"), micro("current_now")) {
            (Some(v), Some(c)) => v * c / 1_000_000_000_000.0,
            _ => 0.0,
        },
    }
}

/// System-level status combined from the per-pack statuses.
///
/// A dual-battery machine is charging if either pack is charging and
/// discharging if either pack is discharging, so the most active status wins
/// over the pack order. Ties keep the later pack's wording, which is the same
/// string for every ranked status.
fn aggregate_status(statuses: &[String]) -> String {
    let rank = |s: &str| match s {
        "Charging" => 3,
        "Discharging" => 2,
        "Full" => 1,
        "Not charging" => 0,
        _ => -1,
    };

    statuses
        .iter()
        .max_by_key(|s| rank(s))
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string())
}

fn read_battery_info(path: &str, index: usize) -> Result<BatteryInfo, String> {
    let read_file = |file: &str| -> Result<String, String> {
        fs::read_to_string(format!("{}/{}", path, file))
            .map(|s| s.trim().to_string())
            .map_err(|e| e.to_string())
    };

    let read_u32 = |file: &str| -> u32 {
        read_file(file)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };

    let read_f32 = |file: &str| -> f32 {
        read_file(file)
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .map(|v| v / 1_000_000.0) // Convert from µV/µA to V/A
            .unwrap_or(0.0)
    };

    let capacity = read_u32("capacity") as u8;
    let energy_now = read_f32("energy_now");
    let energy_full = read_f32("energy_full");
    let energy_design = read_f32("energy_full_design");

    let health = if energy_design > 0.0 {
        ((energy_full / energy_design) * 100.0) as u8
    } else {
        100
    };

    let voltage = read_f32("voltage_now");

    let power = power_watts(path);

    let current = if let Ok(c_str) = read_file("current_now") {
        c_str.parse::<f32>().map(|v| v / 1_000_000.0).unwrap_or(0.0)
    } else if voltage > 0.0 {
        power / voltage
    } else {
        0.0
    };

    Ok(BatteryInfo {
        name: format!("BAT{}", index),
        status: read_file("status").unwrap_or_else(|_| "Unknown".to_string()),
        capacity,
        health,
        cycles: read_u32("cycle_count"),
        voltage,
        current,
        power,
        energy_now,
        energy_full,
        energy_design,
        technology: read_file("technology").unwrap_or_else(|_| "Unknown".to_string()),
        manufacturer: read_file("manufacturer").unwrap_or_else(|_| "Unknown".to_string()),
    })
}

/// Attribute names for the charge thresholds, most-standard first.
///
/// `charge_control_*` is the generic kernel power-supply API and works beyond
/// ThinkPads. `charge_*_threshold` is the older thinkpad_acpi-specific spelling.
///
/// On a ThinkPad BOTH exist and report the same value, but they are separate
/// sysfs files — so a chmod on one does not affect the other. That is exactly
/// how this broke: permissions.rs granted access to the legacy pair while
/// battery.rs wrote the standard pair, so "Grant Permissions" never made battery
/// thresholds writable and every change fell through to a password prompt.
const THRESHOLD_ATTRS: &[(&str, &str)] = &[
    (
        "charge_control_start_threshold",
        "charge_control_end_threshold",
    ),
    ("charge_start_threshold", "charge_stop_threshold"),
];

/// The threshold file pair this machine actually exposes.
///
/// Returns the first pair where both files exist. Every caller must go through
/// here — the duplication between modules is what allowed them to disagree.
pub fn threshold_paths() -> Option<(String, String)> {
    THRESHOLD_ATTRS.iter().find_map(|(start, stop)| {
        let start_path = format!("{}/{}", BAT0_PATH, start);
        let stop_path = format!("{}/{}", BAT0_PATH, stop);
        (Path::new(&start_path).exists() && Path::new(&stop_path).exists())
            .then_some((start_path, stop_path))
    })
}

#[tauri::command]
pub fn get_battery_thresholds() -> ApiResponse<BatteryThresholds> {
    let (start_path, stop_path) = match threshold_paths() {
        Some(pair) => pair,
        None => {
            // Preserve the previous defaults so callers that ignore `success`
            // keep behaving as before.
            return ApiResponse {
                success: true,
                data: Some(BatteryThresholds {
                    start: 0,
                    stop: 100,
                }),
                error: None,
            };
        }
    };

    let start = fs::read_to_string(&start_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let stop = fs::read_to_string(&stop_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(100);

    ApiResponse {
        success: true,
        data: Some(BatteryThresholds { start, stop }),
        error: None,
    }
}

/// Whether the new start threshold must be written before the new stop threshold.
///
/// The firmware rejects any write that would leave start >= stop, even
/// transiently, so the order depends on where the new stop lands relative to the
/// start already on the hardware.
///
/// Writing stop first leaves the intermediate state (current_start, new_stop).
/// That is only safe while current_start < new_stop. The boundary case matters:
/// at new_stop == current_start the intermediate state is (X, X), which is
/// start == stop and is rejected — hence >=, not >.
fn write_start_first(current_start: u8, new_stop: u8) -> bool {
    new_stop <= current_start
}

#[tauri::command]
pub async fn set_battery_thresholds(start: u8, stop: u8) -> ApiResponse<String> {
    if start >= stop {
        return ApiResponse {
            success: false,
            data: None,
            error: Some("Start threshold must be less than stop threshold".to_string()),
        };
    }

    if start > 100 || stop > 100 {
        return ApiResponse {
            success: false,
            data: None,
            error: Some("Thresholds must be between 0 and 100".to_string()),
        };
    }

    let (start_path, stop_path) = match threshold_paths() {
        Some(pair) => pair,
        None => {
            return ApiResponse {
                success: false,
                data: None,
                error: Some(
                    "This machine exposes no battery charge threshold controls.".to_string(),
                ),
            }
        }
    };

    // get_battery_thresholds() has no failure path — it substitutes defaults on a
    // failed read — so there is nothing to match on. Note the substituted default
    // for start is 0, which makes write_start_first() return false and yields the
    // stop-first order. That is the safe fallback: it is the order that was in use
    // before this ordering logic existed.
    let current = get_battery_thresholds().data.unwrap_or(BatteryThresholds {
        start: 0,
        stop: 100,
    });

    let (first_path, first_value, second_path, second_value) =
        if write_start_first(current.start, stop) {
            (start_path, start.to_string(), stop_path, stop.to_string())
        } else {
            (stop_path, stop.to_string(), start_path, start.to_string())
        };

    // Try direct write first
    if fs::write(&first_path, &first_value).is_ok()
        && fs::write(&second_path, &second_value).is_ok()
    {
        return ApiResponse {
            success: true,
            data: Some(format!("Thresholds set: {}%-{}%", start, stop)),
            error: None,
        };
    }

    // Need elevated permissions. Writes stay in the order chosen above.
    let script_content = format!(
        "#!/bin/bash\nset -e\necho {} > {}\necho {} > {}\nexit 0\n",
        first_value, first_path, second_value, second_path
    );

    match crate::privileged::run_script(&script_content).await {
        Ok(output) => {
            if output.status.success() {
                ApiResponse {
                    success: true,
                    data: Some(format!("Thresholds set: {}%-{}%", start, stop)),
                    error: None,
                }
            } else {
                ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Permission denied".to_string()),
                }
            }
        }
        Err(e) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("Failed to execute: {}", e)),
        },
    }
}

/// Hours an energy reserve lasts, or would take to move, at a given rate.
///
/// `None` when there is nothing to divide by or nothing to move. A rate of
/// zero is not an infinite runtime, and a delta of zero is not an instant: both
/// are "no estimate", which the UI renders as an em dash rather than a number
/// it made up.
fn hours_at_rate(delta_wh: f32, power_w: f32) -> Option<f32> {
    if power_w <= 0.05 || delta_wh <= 0.0 {
        return None;
    }
    Some(delta_wh / power_w)
}

/// Energy a pack holds when charging stops at `stop_threshold`.
///
/// A machine with no threshold control, or a limit set at 100%, stops at the
/// pack's full energy. `None` when the full energy is unknown, so the caller
/// can say so instead of projecting from a zero.
fn energy_at_threshold(energy_full_wh: f32, stop_threshold: Option<u8>) -> Option<f32> {
    if energy_full_wh <= 0.0 {
        return None;
    }
    match stop_threshold {
        Some(stop) if stop < 100 => Some(energy_full_wh * stop as f32 / 100.0),
        _ => Some(energy_full_wh),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PowerConsumptionData {
    pub power_w: f32,
    pub is_charging: bool,
    pub is_ac_online: bool,
    pub status: String,
    pub capacity: u8,
    pub runtime_hours: Option<f32>,
    pub time_to_full_hours: Option<f32>,
    /// Energy the packs hold right now, in Wh.
    pub energy_now_wh: f32,
    /// Energy the packs hold when full, in Wh.
    pub energy_full_wh: f32,
    /// Energy the packs held when new, in Wh.
    pub energy_design_wh: f32,
    /// Energy the packs will hold when charging stops, in Wh. Absent when no
    /// pack reports a usable capacity.
    pub limit_energy_wh: Option<f32>,
    /// Firmware charge window. Absent on machines with no threshold control.
    pub start_threshold: Option<u8>,
    pub stop_threshold: Option<u8>,
}

/// Whether the machine is on mains power.
///
/// The one place `AC/online` is read. The overview, the power view and the GPU
/// ceiling all need the same answer, and a copy of the path in each of them is a
/// copy that can drift. A machine that exposes no such file reads as not on
/// mains, which is also what the driver reports while the adapter is unplugged.
pub fn is_ac_online() -> bool {
    fs::read_to_string("/sys/class/power_supply/AC/online")
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_power_consumption() -> ApiResponse<PowerConsumptionData> {
    let is_ac_online = is_ac_online();

    let mut power_w: f32 = 0.0;
    let mut energy_now_wh: f32 = 0.0;
    let mut energy_full_wh: f32 = 0.0;
    let mut energy_design_wh: f32 = 0.0;
    let mut statuses: Vec<String> = Vec::new();
    // Capacity is averaged over the packs weighted by how much energy each one
    // holds, so a small second cell cannot skew the system percentage.
    let mut capacity_weighted: f32 = 0.0;
    let mut capacity_weight: f32 = 0.0;

    for bat_path in [BAT0_PATH, BAT1_PATH] {
        if Path::new(bat_path).exists() {
            let read_f32 = |file: &str| -> f32 {
                fs::read_to_string(format!("{}/{}", bat_path, file))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .map(|v| v / 1_000_000.0)
                    .unwrap_or(0.0)
            };

            power_w += power_watts(bat_path);
            energy_now_wh += read_f32("energy_now");
            let energy_full = read_f32("energy_full");
            energy_full_wh += energy_full;
            energy_design_wh += read_f32("energy_full_design");

            if let Ok(st) = fs::read_to_string(format!("{}/status", bat_path)) {
                let st_trimmed = st.trim().to_string();
                if st_trimmed != "Unknown" && !st_trimmed.is_empty() {
                    statuses.push(st_trimmed);
                }
            }

            if let Ok(cap_str) = fs::read_to_string(format!("{}/capacity", bat_path)) {
                if let Ok(cap) = cap_str.trim().parse::<u8>() {
                    let weight = if energy_full > 0.0 { energy_full } else { 1.0 };
                    capacity_weighted += cap as f32 * weight;
                    capacity_weight += weight;
                }
            }
        }
    }

    let status = aggregate_status(&statuses);
    let capacity = if capacity_weight > 0.0 {
        (capacity_weighted / capacity_weight).round() as u8
    } else {
        0
    };

    let is_charging = status == "Charging";

    // Read through the shared attribute lookup so the generic
    // `charge_control_*` names and the legacy thinkpad_acpi spellings both work.
    let (start_threshold, stop_threshold) = match threshold_paths() {
        Some((start_path, stop_path)) => (read_u8(&start_path), read_u8(&stop_path)),
        None => (None, None),
    };

    // Only while discharging: with the cable in, power_w is the charge rate, and
    // dividing the energy left by it answers a question nobody asked.
    let runtime_hours = if !is_charging && !is_ac_online {
        hours_at_rate(energy_now_wh, power_w)
    } else {
        None
    };

    let time_to_full_hours = if is_charging {
        hours_at_rate(energy_full_wh - energy_now_wh, power_w)
    } else {
        None
    };

    ApiResponse {
        success: true,
        data: Some(PowerConsumptionData {
            power_w,
            is_charging,
            is_ac_online,
            status,
            capacity,
            runtime_hours,
            time_to_full_hours,
            energy_now_wh,
            energy_full_wh,
            energy_design_wh,
            limit_energy_wh: energy_at_threshold(energy_full_wh, stop_threshold),
            start_threshold,
            stop_threshold,
        }),
        error: None,
    }
}

/// One whole-number sysfs attribute, trimmed; `None` when absent or unparsable.
fn read_u8(path: &str) -> Option<u8> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Model the firmware constraint: a write is rejected if it would leave
    /// start >= stop, even for an instant. Replays the two writes in the chosen
    /// order and reports whether every intermediate state stayed legal.
    fn ordering_is_safe(current: (u8, u8), target: (u8, u8)) -> bool {
        let (cur_start, cur_stop) = current;
        let (new_start, new_stop) = target;

        let states = if write_start_first(cur_start, new_stop) {
            // write start, then stop
            [(new_start, cur_stop), (new_start, new_stop)]
        } else {
            // write stop, then start
            [(cur_start, new_stop), (new_start, new_stop)]
        };

        states.iter().all(|(s, e)| s < e)
    }

    /// The case the original ordering logic missed. With `new_stop < current_start`
    /// this picks stop-first, producing the intermediate state (60, 60) — start
    /// equal to stop, which the firmware rejects with the same "Invalid argument"
    /// the ordering was introduced to prevent.
    #[test]
    fn boundary_new_stop_equals_current_start() {
        assert!(
            write_start_first(60, 60),
            "new_stop == current_start must write start first"
        );
        assert!(ordering_is_safe((60, 80), (40, 60)));
    }

    #[test]
    fn lowering_thresholds_writes_start_first() {
        assert!(write_start_first(60, 50));
        assert!(ordering_is_safe((60, 80), (30, 50)));
    }

    #[test]
    fn raising_thresholds_writes_stop_first() {
        assert!(!write_start_first(40, 90));
        assert!(ordering_is_safe((40, 60), (70, 90)));
    }

    /// Exhaustive sweep over every legal current/target pair. Any ordering rule
    /// that admits an illegal intermediate state fails here.
    #[test]
    fn no_transition_passes_through_an_illegal_state() {
        for cur_start in 0..=100u8 {
            for cur_stop in (cur_start + 1)..=100u8 {
                for new_start in 0..=100u8 {
                    for new_stop in (new_start + 1)..=100u8 {
                        assert!(
                            ordering_is_safe((cur_start, cur_stop), (new_start, new_stop)),
                            "illegal intermediate state going from ({}, {}) to ({}, {})",
                            cur_start,
                            cur_stop,
                            new_start,
                            new_stop
                        );
                    }
                }
            }
        }
    }

    /// A failed sysfs read substitutes start = 0, which must fall back to the
    /// stop-first order that predates this logic rather than doing something novel.
    #[test]
    fn unreadable_current_start_falls_back_to_stop_first() {
        assert!(!write_start_first(0, 80));
    }
}

#[cfg(test)]
mod threshold_path_tests {
    use super::*;

    /// The generic kernel API must be preferred. Both spellings exist on a
    /// ThinkPad and report the same value, but only the generic one exists on
    /// other hardware, so choosing the legacy pair first would silently limit
    /// support to ThinkPads.
    #[test]
    fn prefers_the_generic_kernel_attribute_names() {
        assert_eq!(
            THRESHOLD_ATTRS[0],
            (
                "charge_control_start_threshold",
                "charge_control_end_threshold"
            )
        );
    }

    /// The legacy thinkpad_acpi spelling stays as a fallback for older kernels
    /// that expose only it.
    #[test]
    fn keeps_the_legacy_spelling_as_a_fallback() {
        assert!(THRESHOLD_ATTRS
            .iter()
            .any(|(s, e)| *s == "charge_start_threshold" && *e == "charge_stop_threshold"));
    }

    /// Start and stop must never come from different naming schemes: writing a
    /// generic start and a legacy stop would touch two different sysfs files and
    /// could leave the pair inconsistent.
    #[test]
    fn each_candidate_pair_uses_one_naming_scheme() {
        for (start, stop) in THRESHOLD_ATTRS {
            let start_is_generic = start.starts_with("charge_control_");
            let stop_is_generic = stop.starts_with("charge_control_");
            assert_eq!(
                start_is_generic, stop_is_generic,
                "mixed naming scheme in pair ({}, {})",
                start, stop
            );
        }
    }
}

#[cfg(test)]
mod power_tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// A throwaway sysfs stand-in, so the wattage rules are exercised with real
    /// file reads instead of a mocked reader.
    fn battery_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thinkpro-battery-{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn attr(dir: &Path, file: &str, value: &str) {
        fs::write(dir.join(file), value).unwrap();
    }

    fn watts(dir: &Path) -> f32 {
        power_watts(dir.to_str().unwrap())
    }

    /// This machine, in the state it was audited in: `power_now` exists and
    /// reads 0, `current_now` does not exist, the pack is Full on AC. Nothing
    /// here may be invented — 0.0 W is the correct reading, not a failure.
    #[test]
    fn full_pack_without_current_now_reads_zero() {
        let dir = battery_dir("full-idle");
        attr(
            &dir,
            "power_now",
            "0
",
        );
        attr(
            &dir,
            "voltage_now",
            "17214000
",
        );
        attr(
            &dir, "status", "Full
",
        );
        assert_eq!(watts(&dir), 0.0);
    }

    /// Packs without `power_now` must fall back to voltage x current.
    /// Microvolts x microamps is picowatts: 17.214 V x 2 A = 34.428 W, and a
    /// 1e6 divisor instead of 1e12 would report 34.4 MW.
    #[test]
    fn falls_back_to_voltage_times_current() {
        let dir = battery_dir("fallback");
        attr(
            &dir,
            "voltage_now",
            "17214000
",
        );
        attr(
            &dir,
            "current_now",
            "2000000
",
        );
        let w = watts(&dir);
        assert!((w - 34.428).abs() < 0.001, "expected 34.428 W, got {w}");
    }

    /// A zero `power_now` must not short-circuit the fallback, which is the
    /// regression the missing-current-now ThinkPads hit.
    #[test]
    fn zero_power_now_still_uses_the_fallback() {
        let dir = battery_dir("zero-plus-current");
        attr(
            &dir,
            "power_now",
            "0
",
        );
        attr(
            &dir,
            "voltage_now",
            "17214000
",
        );
        attr(
            &dir,
            "current_now",
            "2000000
",
        );
        let w = watts(&dir);
        assert!((w - 34.428).abs() < 0.001, "expected 34.428 W, got {w}");
    }

    /// A real `power_now` is the pack's own measurement and wins over the
    /// product of the other two files.
    #[test]
    fn power_now_wins_over_the_fallback() {
        let dir = battery_dir("preferred");
        attr(
            &dir,
            "power_now",
            "35060000
",
        );
        attr(
            &dir,
            "voltage_now",
            "17214000
",
        );
        attr(
            &dir,
            "current_now",
            "1000000
",
        );
        let w = watts(&dir);
        assert!((w - 35.06).abs() < 0.001, "expected 35.06 W, got {w}");
    }

    /// Neither source present: 0.0 W, never a parse panic or a nonsense value.
    #[test]
    fn no_source_at_all_reads_zero() {
        let dir = battery_dir("empty");
        assert_eq!(watts(&dir), 0.0);
    }

    /// The system is only as idle as its most active pack: a full laptop
    /// plugged in with one pack charging is charging.
    #[test]
    fn system_status_prefers_the_active_pack() {
        assert_eq!(
            aggregate_status(&["Full".to_string(), "Charging".to_string()]),
            "Charging"
        );
        assert_eq!(
            aggregate_status(&["Full".to_string(), "Discharging".to_string()]),
            "Discharging"
        );
        assert_eq!(aggregate_status(&["Full".to_string()]), "Full");
        assert_eq!(aggregate_status(&[]), "Unknown");
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    /// The energy a limit stops at: a 57.00 Wh design pack with a firmware stop
    /// of 80% leaves 45.60 Wh.
    #[test]
    fn energy_at_threshold_scales_the_pack_to_the_stop_limit() {
        let limit = energy_at_threshold(57.0, Some(80)).unwrap();
        assert!(
            (limit - 45.6).abs() < 1e-4,
            "expected 45.60 Wh, got {limit}"
        );
    }

    /// No threshold control, or one already at 100%: the pack stops full.
    #[test]
    fn energy_at_threshold_is_the_whole_pack_without_a_lower_limit() {
        assert_eq!(energy_at_threshold(57.0, None), Some(57.0));
        assert_eq!(energy_at_threshold(57.0, Some(100)), Some(57.0));
    }

    /// A pack that reports no `energy_full` has no projection to give; the
    /// caller must say so rather than project zero hours.
    #[test]
    fn energy_at_threshold_gives_nothing_for_an_unreported_pack() {
        assert_eq!(energy_at_threshold(0.0, Some(80)), None);
    }

    /// Hours from energy over rate: 45 Wh at 9 W is 5 h.
    #[test]
    fn hours_at_rate_divides_energy_by_power() {
        assert_eq!(hours_at_rate(45.0, 9.0), Some(5.0));
    }

    /// This machine full on AC: `power_now` reads 0, so the honest answer is no
    /// estimate. Dividing by zero, or by the 0.02 W of noise a sleeping pack
    /// reports, would invent a runtime of days.
    #[test]
    fn hours_at_rate_refuses_a_rate_that_is_not_flowing() {
        assert_eq!(hours_at_rate(45.0, 0.0), None);
        assert_eq!(hours_at_rate(45.0, 0.02), None);
    }

    /// Nothing left to move is not an instant trip: it is no estimate. The
    /// reference behaves the same way when the remaining charge is zero.
    #[test]
    fn hours_at_rate_refuses_an_empty_delta() {
        assert_eq!(hours_at_rate(0.0, 9.0), None);
        assert_eq!(hours_at_rate(-1.0, 9.0), None);
    }
}
