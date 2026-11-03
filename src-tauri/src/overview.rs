//! Machine overview: what this laptop is, and what it is doing right now.
//!
//! This is the data behind the overview page: identity (model, serial), the CPU
//! and GPU as they currently run, memory with its configured speed, storage with
//! its SMART health, and the battery/charger side of the power picture.
//!
//! Sources are chosen so that the page is useful without a password prompt:
//! sysfs, /proc and the hwmon devices cover almost everything. Four values are
//! mode 0400 root-only on a default install -- the machine serial, the package
//! energy counter, the DMI memory table and the SMART counters -- and those go
//! through the existing privileged helper as read-only verbs rather than through
//! a second privileged entry point.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::battery::ApiResponse;
use crate::hardware_root;

const HWMON_DIR: &str = "/sys/class/hwmon";
const DRM_DIR: &str = "/sys/class/drm";
const DMI_DIR: &str = "/sys/class/dmi/id";
const CPUINFO: &str = "/proc/cpuinfo";
const CPUFREQ: &str = "/sys/devices/system/cpu/cpu0/cpufreq";
const DISKSTATS: &str = "/proc/diskstats";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Identity {
    /// Marketing name, e.g. "ThinkPad T14s Gen 2a".
    pub model: String,
    /// Machine type/model code, e.g. "20XGS02K00".
    pub model_code: String,
    pub vendor: String,
    pub bios_version: String,
    /// The serial is root-only, so it is fetched on demand, not here.
    pub serial_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuOverview {
    pub model: String,
    pub cores: u32,
    pub threads: u32,
    pub governor: String,
    pub min_mhz: u32,
    pub max_mhz: u32,
    pub current_mhz: u32,
    pub temperature_c: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuOverview {
    pub name: String,
    pub driver: String,
    pub pci_id: String,
    pub busy_percent: Option<u32>,
    pub current_mhz: Option<u32>,
    pub max_mhz: Option<u32>,
    pub vram_total_mb: Option<u64>,
    pub vram_used_mb: Option<u64>,
    pub temperature_c: Option<f32>,
    pub watts: Option<f32>,
    /// Result of the performance-level override, when the driver exposes one.
    pub dpm_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryModule {
    pub locator: String,
    pub size_mb: u64,
    pub kind: String,
    pub speed_mts: Option<u32>,
    pub configured_speed_mts: Option<u32>,
    pub manufacturer: String,
    pub part_number: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryOverview {
    pub total_mb: u64,
    pub used_mb: u64,
    pub available_mb: u64,
    pub usage_percent: f64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    /// Populated only after the memory table has been read through the helper.
    pub modules: Vec<MemoryModule>,
    pub speed_mts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageHealth {
    pub passed: bool,
    pub critical_warning: u8,
    pub percentage_used: Option<u8>,
    pub available_spare_percent: Option<u8>,
    pub data_written_tb: Option<f64>,
    pub data_read_tb: Option<f64>,
    pub power_on_hours: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub media_errors: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageOverview {
    pub device: String,
    pub model: String,
    pub firmware: String,
    pub size_gb: f64,
    pub temperature_c: Option<f32>,
    pub read_mb_s: Option<f64>,
    pub write_mb_s: Option<f64>,
    pub health: Option<StorageHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PowerOverview {
    pub ac_online: bool,
    pub charging: bool,
    pub status: String,
    pub capacity: u8,
    pub voltage_v: f32,
    pub current_a: f32,
    pub battery_watts: f32,
    pub energy_now_wh: f32,
    pub energy_full_wh: f32,
    pub runtime_hours: Option<f32>,
    pub time_to_full_hours: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemOverview {
    pub identity: Identity,
    pub cpu: CpuOverview,
    pub gpu: Option<GpuOverview>,
    pub memory: MemoryOverview,
    pub storage: Vec<StorageOverview>,
    pub power: PowerOverview,
    /// Whether the privileged helper is present, i.e. whether the four
    /// root-only readings above can be fetched at all.
    pub helper_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagePower {
    pub available: bool,
    pub reason: Option<String>,
    pub watts: Option<f64>,
    pub micro_joules: Option<u64>,
}

// ---------------------------------------------------------------------------
// Small readers
// ---------------------------------------------------------------------------

fn read(path: &str) -> Option<String> {
    hardware_root::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_u32(path: &str) -> Option<u32> {
    read(path)?.parse().ok()
}

fn read_u64(path: &str) -> Option<u64> {
    read(path)?.parse().ok()
}

/// Directory of the hwmon device with this name, if the machine has one.
fn hwmon_named(name: &str) -> Option<String> {
    let base = hardware_root::resolve(HWMON_DIR);
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let path = entry.path();
        let found = std::fs::read_to_string(path.join("name"))
            .ok()
            .map(|value| value.trim().to_string());
        if found.as_deref() == Some(name) {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

fn hwmon_reading(name: &str, file: &str, scale: f32) -> Option<f32> {
    let dir = hwmon_named(name)?;
    let raw: f32 = std::fs::read_to_string(format!("{}/{}", dir, file))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(raw / scale)
}

/// CPU temperature: AMD and Intel publish it under different names, and every
/// machine has an ACPI zone as a last resort.
fn cpu_temperature() -> Option<f32> {
    for driver in ["k10temp", "coretemp", "zenpower", "acpitz"] {
        if let Some(temp) = hwmon_reading(driver, "temp1_input", 1000.0) {
            return Some(temp);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

fn identity() -> Identity {
    // Lenovo puts the marketing name in product_version and the machine type in
    // product_name; other vendors differ, so fall back in that order.
    let product_name = read(&format!("{}/product_name", DMI_DIR)).unwrap_or_default();
    let product_version = read(&format!("{}/product_version", DMI_DIR)).unwrap_or_default();
    let model = if product_version.is_empty() {
        product_name.clone()
    } else {
        product_version
    };
    let model_code = if product_name == model {
        String::new()
    } else {
        product_name
    };
    let serial_source = crate::fan_control::helper_path().map(|path| path.to_string());
    Identity {
        model,
        model_code,
        vendor: read(&format!("{}/sys_vendor", DMI_DIR)).unwrap_or_default(),
        bios_version: read(&format!("{}/bios_version", DMI_DIR)).unwrap_or_default(),
        serial_source,
    }
}

// ---------------------------------------------------------------------------
// CPU
// ---------------------------------------------------------------------------

fn cpu_model(cpuinfo: &str) -> String {
    cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("model name"))
        .and_then(|rest| {
            rest.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        })
        .unwrap_or_default()
}

fn cpu_counts(cpuinfo: &str) -> (u32, u32) {
    let threads = cpuinfo
        .lines()
        .filter(|line| line.starts_with("processor\t") || line.starts_with("processor :"))
        .count() as u32;
    let cores = cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("cpu cores"))
        .and_then(|rest| {
            rest.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        })
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(threads);
    (cores, threads)
}

fn cpu() -> CpuOverview {
    let cpuinfo = read(CPUINFO).unwrap_or_default();
    let (cores, threads) = cpu_counts(&cpuinfo);
    CpuOverview {
        model: cpu_model(&cpuinfo),
        cores,
        threads,
        governor: read(&format!("{}/scaling_governor", CPUFREQ)).unwrap_or_default(),
        min_mhz: read_u32(&format!("{}/cpuinfo_min_freq", CPUFREQ))
            .map(|khz| khz / 1000)
            .unwrap_or(0),
        max_mhz: read_u32(&format!("{}/cpuinfo_max_freq", CPUFREQ))
            .map(|khz| khz / 1000)
            .unwrap_or(0),
        current_mhz: read_u32(&format!("{}/scaling_cur_freq", CPUFREQ))
            .map(|khz| khz / 1000)
            .unwrap_or(0),
        temperature_c: cpu_temperature(),
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// Current and maximum clock from an amdgpu DPM table. The active level is the
/// one marked with `*`.
pub fn parse_sclk(table: &str) -> (Option<u32>, Option<u32>) {
    let mut current = None;
    let mut max = None;
    for line in table.lines() {
        let Some((_, rest)) = line.split_once(':') else {
            continue;
        };
        let digits: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let Ok(mhz) = digits.parse::<u32>() else {
            continue;
        };
        max = Some(max.map_or(mhz, |value: u32| value.max(mhz)));
        if rest.contains('*') {
            current = Some(mhz);
        }
    }
    (current, max)
}

fn gpu_card_dir() -> Option<String> {
    let base = hardware_root::resolve(DRM_DIR);
    let mut cards: Vec<std::path::PathBuf> = std::fs::read_dir(base)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.starts_with("card") && !name.contains('-')
        })
        .collect();
    cards.sort();
    for card in cards {
        let device = card.join("device");
        let driver = std::fs::read_link(device.join("driver"))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| name.to_string())
            });
        // A card without a bound driver, or a virtual one, is not a GPU.
        if driver.as_deref() == Some("amdgpu")
            || driver.as_deref() == Some("i915")
            || driver.as_deref() == Some("xe")
        {
            return Some(device.to_string_lossy().to_string());
        }
    }
    None
}

fn gpu_name(device: &str, driver: &str) -> String {
    if let Some(name) = read(&format!("{}/product_name", device)) {
        return name;
    }
    let vendor = read(&format!("{}/vendor", device)).unwrap_or_default();
    let label = match vendor.as_str() {
        "0x1002" => "AMD Radeon",
        "0x8086" => "Intel Graphics",
        "0x10de" => "NVIDIA",
        _ => "GPU",
    };
    format!("{} ({})", label, driver)
}

fn gpu() -> Option<GpuOverview> {
    let device = gpu_card_dir()?;
    let driver = std::fs::read_link(format!("{}/driver", device))
        .ok()
        .and_then(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|name| name.to_string())
        })
        .unwrap_or_default();

    let (current_mhz, max_mhz) = read(&format!("{}/pp_dpm_sclk", device))
        .map(|table| parse_sclk(&table))
        .unwrap_or((None, None));

    // The card directory is a symlink into the PCI tree; its basename is the
    // slot the lspci database is keyed on.
    let slot = std::fs::canonicalize(&device)
        .ok()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
        })
        .unwrap_or_default();
    Some(GpuOverview {
        name: gpu_marketing_name(&slot).unwrap_or_else(|| gpu_name(&device, &driver)),
        driver: driver.clone(),
        pci_id: format!(
            "{}:{}",
            read(&format!("{}/vendor", device)).unwrap_or_default(),
            read(&format!("{}/device", device)).unwrap_or_default()
        ),
        busy_percent: read_u32(&format!("{}/gpu_busy_percent", device)),
        current_mhz,
        max_mhz,
        vram_total_mb: read_u64(&format!("{}/mem_info_vram_total", device))
            .map(|bytes| bytes / 1024 / 1024),
        vram_used_mb: read_u64(&format!("{}/mem_info_vram_used", device))
            .map(|bytes| bytes / 1024 / 1024),
        temperature_c: hwmon_reading(&driver, "temp1_input", 1000.0),
        // amdgpu exposes power1_input on some kernels and power1_average on
        // others; i915 only ever has the former.
        watts: hwmon_reading(&driver, "power1_average", 1_000_000.0)
            .or_else(|| hwmon_reading(&driver, "power1_input", 1_000_000.0)),
        dpm_level: read(&format!("{}/power_dpm_force_performance_level", device)),
    })
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

fn memory() -> MemoryOverview {
    let mut overview = MemoryOverview::default();
    if let Ok(stats) = crate::monitor::get_memory_stats() {
        // /proc/meminfo is in kB throughout this codebase.
        overview.total_mb = stats.total / 1024;
        overview.used_mb = stats.used / 1024;
        overview.available_mb = stats.available / 1024;
        overview.usage_percent = (stats.usage_percent * 10.0).round() / 10.0;
        overview.swap_total_mb = stats.swap_total / 1024;
        overview.swap_used_mb = stats.swap_used / 1024;
    }
    overview
}

/// One block of `dmidecode -t 17` output.
pub fn parse_memory_modules(dmi: &str) -> Vec<MemoryModule> {
    let mut modules = Vec::new();
    for block in dmi.split("Memory Device").skip(1) {
        let mut module = MemoryModule::default();
        let mut installed = true;
        for line in block.lines() {
            let Some((key, value)) = line.trim().split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "Size" => {
                    if value.contains("No Module") || value.starts_with("0 ") {
                        installed = false;
                    }
                    module.size_mb = parse_size_mb(value).unwrap_or(0);
                }
                "Locator" => module.locator = value.to_string(),
                "Type" => {
                    if value != "Unknown" {
                        module.kind = value.to_string();
                    }
                }
                "Speed" => module.speed_mts = parse_mts(value),
                "Configured Memory Speed" => module.configured_speed_mts = parse_mts(value),
                "Manufacturer" => module.manufacturer = value.to_string(),
                "Part Number" => module.part_number = value.to_string(),
                _ => {}
            }
        }
        if installed && module.size_mb > 0 {
            modules.push(module);
        }
    }
    modules
}

fn parse_size_mb(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let number: f64 = parts.next()?.parse().ok()?;
    let unit = parts.next().unwrap_or("MB");
    // dmidecode reports memory in binary units on current systems ("8 GiB") and
    // in decimal ones on older firmware ("4096 MB"), so both spellings have to be
    // understood; getting this wrong silently drops the module from the list.
    let mb = match unit {
        "kB" | "KB" => number / 1024.0,
        "MB" => number,
        "GB" => number * 1024.0,
        "TB" => number * 1024.0 * 1024.0,
        "KiB" => number / 1024.0,
        "MiB" => number,
        "GiB" => number * 1024.0,
        "TiB" => number * 1024.0 * 1024.0,
        _ => return None,
    };
    Some(mb.round() as u64)
}

fn parse_mts(value: &str) -> Option<u32> {
    value.split_whitespace().next()?.parse().ok()
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

struct IoCounters {
    sectors_read: u64,
    sectors_written: u64,
    at: Instant,
}

/// Last I/O counters per device, so the next read can be turned into a rate.
type IoState = Mutex<Vec<(String, IoCounters)>>;
static IO_STATE: OnceLock<IoState> = OnceLock::new();

fn io_state() -> &'static IoState {
    IO_STATE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Read `sectors read/written` for a block device from /proc/diskstats.
pub fn parse_diskstats(text: &str, device: &str) -> Option<(u64, u64)> {
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 || fields[2] != device {
            continue;
        }
        let sectors_read: u64 = fields[5].parse().ok()?;
        let sectors_written: u64 = fields[9].parse().ok()?;
        return Some((sectors_read, sectors_written));
    }
    None
}

/// Turn two cumulative counters into a rate, in MB/s.
fn rate_mb_s(previous: u64, current: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 || current < previous {
        return 0.0;
    }
    let sectors = (current - previous) as f64;
    sectors * 512.0 / 1_000_000.0 / seconds
}

fn storage_devices() -> Vec<String> {
    let base = hardware_root::resolve("/sys/class/block");
    let mut names: Vec<String> = std::fs::read_dir(base)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| {
                    name.starts_with("nvme") || name.starts_with("sd") || name.starts_with("vd")
                })
                // Partitions carry a `partition` attribute; whole disks do not.
                .filter(|name| {
                    !hardware_root::exists(&format!("/sys/class/block/{}/partition", name))
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.truncate(4);
    names
}

fn storage() -> Vec<StorageOverview> {
    let diskstats = read(DISKSTATS).unwrap_or_default();
    let mut previous = io_state().lock().ok();
    let mut out = Vec::new();

    for device in storage_devices() {
        let base = format!("/sys/class/block/{}", device);
        let sectors = read_u64(&format!("{}/size", base)).unwrap_or(0);
        let temperature = hwmon_named("nvme")
            .and_then(|dir| {
                std::fs::read_to_string(format!("{}/temp1_input", dir))
                    .ok()
                    .and_then(|value| value.trim().parse::<f32>().ok())
            })
            .map(|milli| milli / 1000.0);

        let (read_mb_s, write_mb_s) = match (
            previous.as_deref_mut(),
            parse_diskstats(&diskstats, &device),
        ) {
            (Some(state), Some((sectors_read, sectors_written))) => {
                let now = Instant::now();
                let computed = match state.iter_mut().find(|(name, _)| name == &device) {
                    Some((_, counters)) => {
                        let seconds = now.duration_since(counters.at).as_secs_f64();
                        let read = rate_mb_s(counters.sectors_read, sectors_read, seconds);
                        let write = rate_mb_s(counters.sectors_written, sectors_written, seconds);
                        counters.sectors_read = sectors_read;
                        counters.sectors_written = sectors_written;
                        counters.at = now;
                        (Some(read), Some(write))
                    }
                    None => {
                        state.push((
                            device.clone(),
                            IoCounters {
                                sectors_read,
                                sectors_written,
                                at: now,
                            },
                        ));
                        (None, None)
                    }
                };
                computed
            }
            _ => (None, None),
        };

        out.push(StorageOverview {
            device: device.clone(),
            model: read(&format!("{}/device/model", base)).unwrap_or_default(),
            firmware: read(&format!("{}/device/firmware_rev", base)).unwrap_or_default(),
            // The kernel reports 512-byte sectors here regardless of the drive's
            // physical sector size.
            size_gb: (sectors as f64 * 512.0 / 1_000_000_000.0 * 10.0).round() / 10.0,
            temperature_c: temperature,
            read_mb_s,
            write_mb_s,
            health: None,
        });
    }
    out
}

/// SMART counters, as returned by `smartctl -j -H -A`.
pub fn parse_smart_json(text: &str) -> Option<StorageHealth> {
    #[derive(Deserialize)]
    struct SmartStatus {
        passed: Option<bool>,
    }
    #[derive(Deserialize)]
    struct NvmeLog {
        critical_warning: Option<u8>,
        available_spare: Option<u8>,
        percentage_used: Option<u8>,
        data_units_read: Option<u64>,
        data_units_written: Option<u64>,
        power_on_hours: Option<u64>,
        unsafe_shutdowns: Option<u64>,
        media_errors: Option<u64>,
    }
    #[derive(Deserialize)]
    struct Root {
        smart_status: Option<SmartStatus>,
        nvme_smart_health_information_log: Option<NvmeLog>,
    }

    let root: Root = serde_json::from_str(text).ok()?;
    let log = root.nvme_smart_health_information_log.unwrap_or(NvmeLog {
        critical_warning: None,
        available_spare: None,
        percentage_used: None,
        data_units_read: None,
        data_units_written: None,
        power_on_hours: None,
        unsafe_shutdowns: None,
        media_errors: None,
    });
    // NVMe counts data units of 512,000 bytes.
    let tb = |units: Option<u64>| {
        units.map(|value| (value as f64 * 512_000.0 / 1e12 * 100.0).round() / 100.0)
    };
    Some(StorageHealth {
        passed: root
            .smart_status
            .and_then(|status| status.passed)
            .unwrap_or(false),
        critical_warning: log.critical_warning.unwrap_or(0),
        percentage_used: log.percentage_used,
        available_spare_percent: log.available_spare,
        data_written_tb: tb(log.data_units_written),
        data_read_tb: tb(log.data_units_read),
        power_on_hours: log.power_on_hours,
        unsafe_shutdowns: log.unsafe_shutdowns,
        media_errors: log.media_errors,
    })
}

// ---------------------------------------------------------------------------
// Power
// ---------------------------------------------------------------------------

fn power() -> PowerOverview {
    let mut overview = PowerOverview::default();
    let batteries = crate::battery::get_battery_info();
    if let Some(battery) = batteries.data.and_then(|list| list.into_iter().next()) {
        {
            overview.capacity = battery.capacity;
            overview.status = battery.status.clone();
            overview.voltage_v = battery.voltage;
            overview.current_a = battery.current;
            overview.energy_now_wh = battery.energy_now;
            overview.energy_full_wh = battery.energy_full;
            overview.charging = battery.status.eq_ignore_ascii_case("charging");
        }
    }
    let consumption = crate::battery::get_power_consumption();
    if let Some(consumption) = consumption.data {
        {
            overview.ac_online = consumption.is_ac_online;
            overview.battery_watts = consumption.power_w;
            overview.runtime_hours = consumption.runtime_hours;
            overview.time_to_full_hours = consumption.time_to_full_hours;
            if overview.status.is_empty() {
                overview.status = consumption.status;
            }
        }
    }
    overview
}

/// Turn one line of `lspci -mm` into a display name.
///
/// The format is positional: an unquoted slot, then quoted class, vendor and
/// device, then whatever else the entry carries. Only the quoted fields are
/// wanted, and they are every other field after the slot.
pub fn parse_lspci_mm(text: &str) -> Option<String> {
    let quoted: Vec<&str> = text.split('"').skip(1).step_by(2).collect();
    let vendor = short_vendor(quoted.get(1).copied().unwrap_or_default().trim());
    let device = clean_device(quoted.get(2).copied().unwrap_or_default().trim());
    match (vendor.is_empty(), device.is_empty()) {
        (_, true) => None,
        (true, false) => Some(device),
        (false, false) => Some(format!("{} {}", vendor, device)),
    }
}

/// PCI vendors spell themselves out; the card title does not need it.
fn short_vendor(vendor: &str) -> String {
    let lowered = vendor.to_lowercase();
    if lowered.contains("advanced micro devices") {
        return "AMD".to_string();
    }
    if lowered.contains("intel") {
        return "Intel".to_string();
    }
    if lowered.contains("nvidia") {
        return "NVIDIA".to_string();
    }
    vendor
        .split('[')
        .next()
        .unwrap_or(vendor)
        .trim()
        .to_string()
}

/// Prefer the marketing name when the PCI database keeps one in brackets.
///
/// An AMD integrated GPU is listed as "Cezanne [Radeon Vega Series / Radeon Vega
/// Mobile Series]": the codename is what the bracket-free part says, and the
/// model the user recognises is inside.
fn clean_device(device: &str) -> String {
    if let (Some(open), Some(close)) = (device.find('['), device.rfind(']')) {
        if close > open {
            let inside = device[open + 1..close].trim();
            let marketing = inside.contains("Radeon")
                || inside.contains("Graphics")
                || inside.contains("GeForce")
                || inside.contains("Arc");
            if marketing {
                return inside
                    .split('/')
                    .next()
                    .unwrap_or(inside)
                    .trim()
                    .to_string();
            }
        }
    }
    device
        .split('[')
        .next()
        .unwrap_or(device)
        .trim()
        .to_string()
}

/// Marketing name for the GPU.
///
/// AMD's driver does not publish one, and the integrated GPU shares the CPU's
/// name, so lspci's database is asked once and the result cached; without it the
/// page still shows something truthful.
fn gpu_marketing_name(pci_slot: &str) -> Option<String> {
    /// Slot -> name, so lspci is asked once per card.
    type NameCache = Mutex<Vec<(String, Option<String>)>>;
    static CACHE: OnceLock<NameCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(entries) = cache.lock() {
        if let Some((_, name)) = entries.iter().find(|(slot, _)| slot == pci_slot) {
            return name.clone();
        }
    }
    let name = std::process::Command::new("lspci")
        .args(["-mm", "-s", pci_slot])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| parse_lspci_mm(&String::from_utf8_lossy(&output.stdout)));
    if let Ok(mut entries) = cache.lock() {
        entries.push((pci_slot.to_string(), name.clone()));
    }
    name
}

struct EnergySample {
    micro_joules: u64,
    max_range: u64,
    at: Instant,
    watts: Option<f64>,
}

static RAPL_STATE: OnceLock<Mutex<Option<EnergySample>>> = OnceLock::new();

fn rapl_state() -> &'static Mutex<Option<EnergySample>> {
    RAPL_STATE.get_or_init(|| Mutex::new(None))
}

/// Package power from two cumulative energy counters.
///
/// The counter wraps at `max_range`, so a decrease is treated as a wrap rather
/// than as negative energy.
pub fn watts_from_energy(
    previous: u64,
    previous_max: u64,
    current: u64,
    current_max: u64,
    seconds: f64,
) -> Option<f64> {
    if seconds <= 0.0 {
        return None;
    }
    let delta = if current >= previous {
        current - previous
    } else {
        let range = current_max.max(previous_max);
        if range <= previous {
            return None;
        }
        range - previous + current
    };
    Some(delta as f64 / 1_000_000.0 / seconds)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn unavailable(reason: String) -> PackagePower {
    PackagePower {
        available: false,
        reason: Some(reason),
        watts: None,
        micro_joules: None,
    }
}

/// Collect everything the overview page shows. Blocking: several small reads
/// across sysfs and /proc.
pub fn collect() -> SystemOverview {
    SystemOverview {
        identity: identity(),
        cpu: cpu(),
        gpu: gpu(),
        memory: memory(),
        storage: storage(),
        power: power(),
        helper_available: crate::fan_control::helper_path().is_some(),
    }
}

#[tauri::command]
pub async fn get_system_overview() -> ApiResponse<SystemOverview> {
    let data = tokio::task::spawn_blocking(collect).await;

    match data {
        Ok(overview) => ApiResponse {
            success: true,
            data: Some(overview),
            error: None,
        },
        Err(error) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("Overview collection failed to run: {}", error)),
        },
    }
}

fn run_helper(command: &str) -> Result<String, String> {
    let helper = crate::fan_control::helper_path()
        .ok_or_else(|| "The privileged helper is not installed.".to_string())?;
    let output = std::process::Command::new("pkexec")
        .arg(helper)
        .arg(command)
        .output()
        .map_err(|error| format!("Failed to run the helper: {}", error))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("The helper exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The machine serial: root-only in sysfs, so it is read on request.
#[tauri::command]
pub async fn get_device_serial() -> ApiResponse<String> {
    run_helper_command("serial").await
}

async fn run_helper_command(command: &'static str) -> ApiResponse<String> {
    match tokio::task::spawn_blocking(move || run_helper(command)).await {
        Ok(Ok(value)) => ApiResponse {
            success: true,
            data: Some(value),
            error: None,
        },
        Ok(Err(error)) => ApiResponse {
            success: false,
            data: None,
            error: Some(error),
        },
        Err(error) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("Helper task failed: {}", error)),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryDetail {
    pub available: bool,
    pub reason: Option<String>,
    pub modules: Vec<MemoryModule>,
}

/// The DMI memory table, for the module speed and part numbers.
#[tauri::command]
pub async fn get_memory_speed() -> ApiResponse<MemoryDetail> {
    let result = tokio::task::spawn_blocking(|| run_helper("dmi-memory")).await;
    let data = match result {
        Ok(Ok(text)) => MemoryDetail {
            available: true,
            reason: None,
            modules: parse_memory_modules(&text),
        },
        Ok(Err(error)) => MemoryDetail {
            available: false,
            reason: Some(error),
            modules: Vec::new(),
        },
        Err(error) => MemoryDetail {
            available: false,
            reason: Some(format!("Memory table task failed: {}", error)),
            modules: Vec::new(),
        },
    };
    ApiResponse {
        success: true,
        data: Some(data),
        error: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageDetail {
    pub available: bool,
    pub reason: Option<String>,
    pub health: Option<StorageHealth>,
}

/// NVMe SMART counters, for wear and error counts.
#[tauri::command]
pub async fn get_storage_health() -> ApiResponse<StorageDetail> {
    let result = tokio::task::spawn_blocking(|| run_helper("smart-nvme")).await;
    let data = match result {
        Ok(Ok(text)) => match parse_smart_json(&text) {
            Some(health) => StorageDetail {
                available: true,
                reason: None,
                health: Some(health),
            },
            None => StorageDetail {
                available: false,
                reason: Some("smartctl did not report NVMe health data".to_string()),
                health: None,
            },
        },
        Ok(Err(error)) => StorageDetail {
            available: false,
            reason: Some(error),
            health: None,
        },
        Err(error) => StorageDetail {
            available: false,
            reason: Some(format!("SMART task failed: {}", error)),
            health: None,
        },
    };
    ApiResponse {
        success: true,
        data: Some(data),
        error: None,
    }
}

/// Package power, sampled at most once every few seconds so that polling the
/// page does not spawn a pkexec process per tick.
#[tauri::command]
pub async fn get_package_watts() -> ApiResponse<PackagePower> {
    if crate::fan_control::helper_path().is_none() {
        return ApiResponse {
            success: true,
            data: Some(unavailable(
                "The privileged helper is not installed.".to_string(),
            )),
            error: None,
        };
    }

    let result = tokio::task::spawn_blocking(|| run_helper("rapl-package")).await;
    let text = match result {
        Ok(Ok(text)) => text,
        Ok(Err(error)) => {
            return ApiResponse {
                success: true,
                data: Some(unavailable(error)),
                error: None,
            }
        }
        Err(error) => {
            return ApiResponse {
                success: true,
                data: Some(unavailable(format!("Energy read failed: {}", error))),
                error: None,
            }
        }
    };

    let mut parts = text.split_whitespace();
    let energy: u64 = match parts.next().and_then(|value| value.parse().ok()) {
        Some(value) => value,
        None => {
            return ApiResponse {
                success: true,
                data: Some(unavailable(
                    "No package energy counter on this machine".to_string(),
                )),
                error: None,
            }
        }
    };
    let max_range: u64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let now = Instant::now();

    let watts = match rapl_state().lock() {
        Ok(mut state) => match state.as_mut() {
            Some(sample) => {
                let seconds = now.duration_since(sample.at).as_secs_f64();
                if seconds < 2.0 {
                    sample.micro_joules = energy;
                    sample.watts
                } else {
                    let watts = watts_from_energy(
                        sample.micro_joules,
                        sample.max_range,
                        energy,
                        max_range,
                        seconds,
                    );
                    sample.micro_joules = energy;
                    sample.max_range = max_range;
                    sample.at = now;
                    sample.watts = watts;
                    watts
                }
            }
            None => {
                *state = Some(EnergySample {
                    micro_joules: energy,
                    max_range,
                    at: now,
                    watts: None,
                });
                None
            }
        },
        Err(_) => None,
    };

    ApiResponse {
        success: true,
        data: Some(PackagePower {
            available: true,
            reason: None,
            watts,
            micro_joules: Some(energy),
        }),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_model_and_counts_come_from_proc_cpuinfo() {
        let cpuinfo = "processor\t: 0\nmodel name\t: AMD Ryzen 5 PRO 5650U with Radeon Graphics\ncpu cores\t: 6\n\nprocessor\t: 1\nmodel name\t: AMD Ryzen 5 PRO 5650U with Radeon Graphics\n\n";
        assert_eq!(
            cpu_model(cpuinfo),
            "AMD Ryzen 5 PRO 5650U with Radeon Graphics"
        );
        assert_eq!(cpu_counts(cpuinfo), (6, 2));
    }

    #[test]
    fn lspci_lines_become_a_display_name() {
        // Copied from this machine's card1 entry.
        let line = "05:00.0 \"VGA compatible controller\" \"Advanced Micro Devices, Inc. [AMD/ATI]\" \"Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]\" -rd2 -p00 \"Lenovo\" \"Device 5095\"\n";
        assert_eq!(
            parse_lspci_mm(line).as_deref(),
            Some("AMD Radeon Vega Series")
        );
        // Intel keeps the model outside the brackets.
        let intel = "00:02.0 \"VGA compatible controller\" \"Intel Corporation\" \"Alder Lake-UP3 GT2 [Iris Xe Graphics]\"\n";
        assert_eq!(
            parse_lspci_mm(intel).as_deref(),
            Some("Intel Iris Xe Graphics")
        );
        assert_eq!(parse_lspci_mm(""), None);
    }

    #[test]
    fn dpm_table_marks_the_active_level() {
        let table = "0: 200Mhz *\n1: 400Mhz\n2: 1800Mhz\n";
        assert_eq!(parse_sclk(table), (Some(200), Some(1800)));
        assert_eq!(parse_sclk("garbage\n"), (None, None));
        assert_eq!(parse_sclk("3: 1600Mhz *\n"), (Some(1600), Some(1600)));
    }

    #[test]
    fn memory_modules_skip_empty_slots() {
        let dmi = "Handle 0x0009, DMI type 17, 40 bytes\nMemory Device\n\tLocator: DIMM 0\n\tSize: 8 GB\n\tType: DDR4\n\tSpeed: 3200 MT/s\n\tConfigured Memory Speed: 3200 MT/s\n\tManufacturer: Samsung\n\tPart Number: M471A1G44AB0-CWE\n\nMemory Device\n\tLocator: DIMM 1\n\tSize: No Module Installed\n\tType: Unknown\n";
        let modules = parse_memory_modules(dmi);
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].size_mb, 8192);
        assert_eq!(modules[0].speed_mts, Some(3200));
        assert_eq!(modules[0].configured_speed_mts, Some(3200));
        assert_eq!(modules[0].part_number, "M471A1G44AB0-CWE");
        assert_eq!(modules[0].kind, "DDR4");
    }

    #[test]
    fn memory_sizes_convert_between_units() {
        assert_eq!(parse_size_mb("8 GB"), Some(8192));
        assert_eq!(parse_size_mb("512 MB"), Some(512));
        assert_eq!(parse_size_mb("1048576 kB"), Some(1024));
        assert_eq!(parse_size_mb("No Module Installed"), None);
        // What dmidecode actually prints on this machine, binary units included.
        assert_eq!(parse_size_mb("8 GiB"), Some(8192));
        assert_eq!(parse_size_mb("256 MiB"), Some(256));
        assert_eq!(parse_size_mb("1 TiB"), Some(1_048_576));
    }

    #[test]
    fn a_soldered_memory_block_is_read_as_reported() {
        // Verbatim from this machine's dmidecode -t 17, trimmed.
        let dmi = "Handle 0x0008, DMI type 17, 40 bytes\nMemory Device\n\tArray Handle: 0x0000\n\tLocator: DIMM 0\n\tBank Locator: P0 CHANNEL A\n\tSize: 8 GiB\n\tType: LPDDR4\n\tSpeed: 4266 MT/s\n\tManufacturer: Samsung\n\tPart Number: K4UBE3D4AA-MGCR\n\tConfigured Memory Speed: 4266 MT/s\n\tVolatile Size: 8 GiB\n";
        let modules = parse_memory_modules(dmi);
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].size_mb, 8192);
        assert_eq!(modules[0].kind, "LPDDR4");
        assert_eq!(modules[0].configured_speed_mts, Some(4266));
        assert_eq!(modules[0].manufacturer, "Samsung");
        assert_eq!(modules[0].part_number, "K4UBE3D4AA-MGCR");
    }

    #[test]
    fn diskstats_counters_are_read_per_device() {
        let stats = "   259       0 nvme0n1 11510275 0 1000000 0 5000000 0 4000000 0 0 0 0\n     8       0 sda 100 0 200 0 300 0 400 0 0 0 0\n";
        assert_eq!(parse_diskstats(stats, "nvme0n1"), Some((1000000, 4000000)));
        assert_eq!(parse_diskstats(stats, "sda"), Some((200, 400)));
        assert_eq!(parse_diskstats(stats, "nvme1n1"), None);
    }

    #[test]
    fn io_rates_are_megabytes_per_second_and_survive_resets() {
        // 2048 sectors = 1 MiB, over two seconds = 0.5 MB/s.
        assert!((rate_mb_s(0, 2048, 2.0) - 0.524_288).abs() < 1e-6);
        // A counter that went backwards is a stats reset, not negative traffic.
        assert_eq!(rate_mb_s(1000, 10, 2.0), 0.0);
        assert_eq!(rate_mb_s(0, 2048, 0.0), 0.0);
    }

    #[test]
    fn smart_json_yields_wear_and_error_counts() {
        let json = r#"{
          "smart_status": {"passed": true},
          "temperature": {"current": 38},
          "nvme_smart_health_information_log": {
            "critical_warning": 0,
            "available_spare": 100,
            "percentage_used": 3,
            "data_units_read": 195312500,
            "data_units_written": 97656250,
            "power_on_hours": 1234,
            "unsafe_shutdowns": 12,
            "media_errors": 0
          }
        }"#;
        let health = parse_smart_json(json).expect("parsed");
        assert!(health.passed);
        assert_eq!(health.percentage_used, Some(3));
        assert_eq!(health.power_on_hours, Some(1234));
        assert_eq!(health.unsafe_shutdowns, Some(12));
        // 97656250 data units of 512000 bytes = 50 TB.
        assert_eq!(health.data_written_tb, Some(50.0));
        assert!(parse_smart_json("not json").is_none());
    }

    #[test]
    fn energy_counters_convert_to_watts() {
        // 60 J over 10 s is 6 W.
        let watts =
            watts_from_energy(0, 1_000_000_000, 60_000_000, 1_000_000_000, 10.0).expect("sample");
        assert!((watts - 6.0).abs() < 1e-9);
        // A wrap adds the range instead of subtracting.
        let wrapped = watts_from_energy(999_000_000, 1_000_000_000, 1_000_000, 1_000_000_000, 1.0)
            .expect("wrap");
        assert!((wrapped - 2.0).abs() < 1e-9);
        // A counter that went backwards with no usable range is not a sample.
        assert_eq!(watts_from_energy(100, 50, 10, 50, 1.0), None);
        assert_eq!(watts_from_energy(0, 0, 10, 0, 0.0), None);
    }
}
