//! Ownership detection and conflict reporting.
//!
//! The decision logic is a pure function over a `HostProbe`, so every rule is
//! testable without the machine that has the problem. The probe itself is the
//! only part that reads the system.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

use crate::battery::ApiResponse;

/// A power knob this application can read and — when it owns it — write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Knob {
    PlatformProfile,
    CpuEnergyPerformance,
    CpuGovernor,
    CpuBoost,
    GpuPerformanceLevel,
    ChargeThresholds,
    WifiPowerSave,
    Scheduler,
    FanControl,
}

impl Knob {
    /// Every knob the arbiter reasons about.
    pub const ALL: [Knob; 9] = [
        Knob::PlatformProfile,
        Knob::CpuEnergyPerformance,
        Knob::CpuGovernor,
        Knob::CpuBoost,
        Knob::GpuPerformanceLevel,
        Knob::ChargeThresholds,
        Knob::WifiPowerSave,
        Knob::Scheduler,
        Knob::FanControl,
    ];

    /// Human-readable name, spelled the way the UI shows it.
    pub fn label(self) -> &'static str {
        match self {
            Knob::PlatformProfile => "Platform profile",
            Knob::CpuEnergyPerformance => "Energy performance preference",
            Knob::CpuGovernor => "CPU governor",
            Knob::CpuBoost => "CPU boost",
            Knob::GpuPerformanceLevel => "GPU performance level",
            Knob::ChargeThresholds => "Charge thresholds",
            Knob::WifiPowerSave => "Wi-Fi power save",
            Knob::Scheduler => "Scheduler power mode",
            Knob::FanControl => "Fan control",
        }
    }
}

/// Which component last claimed a knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Tlp,
    Ppd,
    Lact,
    ScxLoader,
    ThinkPro,
    Kernel,
}

/// Which power daemon provides the `org.freedesktop.UPower.PowerProfiles` API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileProvider {
    /// Plain power-profiles-daemon: three profiles, no TLP behind them.
    Ppd,
    /// tlp-pd, which serves the same D-Bus API on top of TLP's configuration.
    TlpPd,
}

/// One knob and the component holding it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub knob: Knob,
    pub label: String,
    pub owner: Owner,
    /// Why we believe that, in the words the UI can show verbatim.
    pub detail: String,
}

/// How badly a finding blocks the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Two daemons hold the same knob; nothing this app writes will stick.
    Blocking,
    /// Values will drift, or a feature is unavailable.
    Warning,
    /// Worth knowing, nothing to fix.
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub severity: Severity,
    pub message: String,
    pub knobs: Vec<Knob>,
    /// What the user (or the app, with permission) can do about it.
    pub remedy: String,
}

/// How much of the power surface is usable on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// TLP is installed and running: the full TLP editor is available.
    FullTlp,
    /// Only power-profiles-daemon: three profiles and its own actions. TLP's
    /// settings cannot be expressed, so the TLP page is gated behind an install.
    LimitedPpd,
    /// Neither daemon: the app can still own sysfs knobs directly through the
    /// helper, but per-profile (AC/BAT/SAV) configuration is impossible.
    LimitedDirect,
}

/// What was found on the machine. Injected so tests need no particular host.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostProbe {
    pub tlp_installed: bool,
    pub tlp_service_enabled: bool,
    pub tlp_service_active: bool,
    pub profile_provider: Option<ProfileProvider>,
    pub profile_provider_active: bool,
    pub lact_installed: bool,
    pub lact_daemon_active: bool,
    /// Value of `power_dpm_force_performance_level`, when the GPU exposes it.
    pub gpu_performance_level: Option<String>,
    pub scx_loader_active: bool,
    pub platform_profile: Option<String>,
    pub energy_performance_preference: Option<String>,
    pub charge_thresholds_available: bool,
    pub fan_control_available: bool,
}

/// The full answer for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ownership {
    pub capability: Capability,
    /// One sentence the header can show without further interpretation.
    pub summary: String,
    pub claims: Vec<Claim>,
    pub conflicts: Vec<Conflict>,
}

/// Decide ownership from a probe. Pure: no I/O, no clock, no environment.
pub fn decide(probe: &HostProbe) -> Ownership {
    let tlp_owns = probe.tlp_installed && (probe.tlp_service_enabled || probe.tlp_service_active);
    let ppd_active = probe.profile_provider_active;

    let mut claims = Vec::with_capacity(Knob::ALL.len());
    let mut conflicts = Vec::new();

    // CPU and platform knobs are TLP's when TLP is running, PPD's while PPD is
    // the active profile provider, and nobody's otherwise.
    let cpu_owner = if tlp_owns {
        (
            Owner::Tlp,
            "TLP applies PLATFORM_PROFILE_ON_* / CPU_ENERGY_PERF_POLICY_ON_*".to_string(),
        )
    } else if ppd_active {
        (
            Owner::Ppd,
            "power-profiles-daemon is the active profile provider".to_string(),
        )
    } else {
        (
            Owner::ThinkPro,
            "no power daemon found; the app writes sysfs directly".to_string(),
        )
    };

    for knob in [
        Knob::PlatformProfile,
        Knob::CpuEnergyPerformance,
        Knob::CpuGovernor,
        Knob::CpuBoost,
    ] {
        claims.push(Claim {
            knob,
            label: knob.label().to_string(),
            owner: cpu_owner.0,
            detail: cpu_owner.1.clone(),
        });
    }

    // GPU: LACT is the default owner. It pins the DPM level through lactd and
    // keeps it pinned, so TLP's RADEON_DPM_* settings lose that fight.
    let gpu = if probe.lact_installed && probe.lact_daemon_active {
        let detail = match probe.gpu_performance_level.as_deref() {
            Some(level) => {
                format!("lactd is running and holds power_dpm_force_performance_level={level}")
            }
            None => "lactd is running".to_string(),
        };
        (Owner::Lact, detail)
    } else if tlp_owns {
        (Owner::Tlp, "TLP owns the Radeon DPM level".to_string())
    } else {
        (
            Owner::ThinkPro,
            "the app takes the GPU level while no daemon holds it".to_string(),
        )
    };
    claims.push(Claim {
        knob: Knob::GpuPerformanceLevel,
        label: Knob::GpuPerformanceLevel.label().to_string(),
        owner: gpu.0,
        detail: gpu.1,
    });

    // Charge thresholds: TLP when it is running (it also honours NATACPI),
    // otherwise the app writes the sysfs attributes itself.
    let thresholds = if tlp_owns {
        (
            Owner::Tlp,
            "TLP sets the NATACPI charge thresholds".to_string(),
        )
    } else {
        (
            Owner::ThinkPro,
            "the app writes charge_control_* directly".to_string(),
        )
    };
    claims.push(Claim {
        knob: Knob::ChargeThresholds,
        label: Knob::ChargeThresholds.label().to_string(),
        owner: thresholds.0,
        detail: thresholds.1,
    });

    // Wi-Fi power save is TLP's when TLP runs, NetworkManager's otherwise. NM
    // is not probed here, so the honest answer is TLP, or unknown-but-not-us.
    claims.push(Claim {
        knob: Knob::WifiPowerSave,
        label: Knob::WifiPowerSave.label().to_string(),
        owner: if tlp_owns { Owner::Tlp } else { Owner::Kernel },
        detail: if tlp_owns {
            "TLP applies WIFI_PWR_ON_*".to_string()
        } else {
            "not managed by this app; the desktop or NetworkManager sets it".to_string()
        },
    });

    claims.push(Claim {
        knob: Knob::Scheduler,
        label: Knob::Scheduler.label().to_string(),
        owner: if probe.scx_loader_active {
            Owner::ScxLoader
        } else {
            Owner::Kernel
        },
        detail: if probe.scx_loader_active {
            "scx_loader is active and sets the scheduler power mode".to_string()
        } else {
            "the kernel's default scheduler is in use".to_string()
        },
    });

    claims.push(Claim {
        knob: Knob::FanControl,
        label: Knob::FanControl.label().to_string(),
        owner: Owner::ThinkPro,
        detail: if probe.fan_control_available {
            "thinkpad_acpi exposes fan control; the helper writes it".to_string()
        } else {
            "no fan control interface on this machine".to_string()
        },
    });

    // ---- conflicts -------------------------------------------------------
    if tlp_owns && ppd_active {
        conflicts.push(Conflict {
            severity: Severity::Blocking,
            message: "TLP and power-profiles-daemon are both active and claim the same CPU knobs"
                .to_string(),
            knobs: vec![Knob::PlatformProfile, Knob::CpuEnergyPerformance],
            remedy:
                "Install tlp-pd so one daemon serves the profile API, or stop TLP while PPD runs"
                    .to_string(),
        });
    }

    if probe.lact_installed && probe.lact_daemon_active && tlp_owns {
        conflicts.push(Conflict {
            severity: Severity::Warning,
            message: "lactd holds the GPU performance level, so TLP's RADEON_DPM_* keys will be reverted"
                .to_string(),
            knobs: vec![Knob::GpuPerformanceLevel],
            remedy: "Keep RADEON_DPM_PERF_LEVEL_* unset in TLP, or let LACT own the GPU and edit it there"
                .to_string(),
        });
    }

    if probe.lact_installed
        && !probe.lact_daemon_active
        && probe.gpu_performance_level.as_deref() == Some("low")
    {
        conflicts.push(Conflict {
            severity: Severity::Warning,
            message:
                "the GPU is pinned to its lowest DPM level and lactd is not running to release it"
                    .to_string(),
            knobs: vec![Knob::GpuPerformanceLevel],
            remedy: "Set the level back to auto; with lactd stopped nothing else will".to_string(),
        });
    }

    if probe.tlp_installed && !tlp_owns {
        conflicts.push(Conflict {
            severity: Severity::Info,
            message: "TLP is installed but neither enabled nor running, so its settings are inert"
                .to_string(),
            knobs: vec![],
            remedy: "sudo systemctl enable --now tlp".to_string(),
        });
    }

    let capability = if tlp_owns {
        Capability::FullTlp
    } else if ppd_active {
        Capability::LimitedPpd
    } else {
        Capability::LimitedDirect
    };

    let summary = match capability {
        Capability::FullTlp => "TLP is running: every TLP setting is editable here".to_string(),
        Capability::LimitedPpd => {
            "power-profiles-daemon is in charge: three profiles only. Install tlp + tlp-pd for the full power editor".to_string()
        }
        Capability::LimitedDirect => {
            "no power daemon detected: the app writes sysfs directly, without per-source profiles".to_string()
        }
    };

    Ownership {
        capability,
        summary,
        claims,
        conflicts,
    }
}

// ---------------------------------------------------------------------------
// Host probing (the only part that touches the system)
// ---------------------------------------------------------------------------

fn unit_state(unit: &str) -> (bool, bool) {
    let enabled = Command::new("systemctl")
        .args(["is-enabled", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let active = Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    (enabled, active)
}

/// Which daemon serves the profile API, read from the unit that owns the name.
fn profile_provider() -> Option<ProfileProvider> {
    let unit = "/usr/lib/systemd/system/power-profiles-daemon.service";
    let text = std::fs::read_to_string(unit).ok()?;
    // tlp-pd ships under the same unit name because it provides the same D-Bus
    // interface; its ExecStart is what tells the two apart.
    if text.to_lowercase().contains("tlp") {
        Some(ProfileProvider::TlpPd)
    } else {
        Some(ProfileProvider::Ppd)
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// First GPU that exposes the DPM level knob.
fn gpu_performance_level() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let card = entry.path();
        if !card
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("card") && !n.contains('-'))
            .unwrap_or(false)
        {
            continue;
        }
        let level = card.join("device/power_dpm_force_performance_level");
        if level.exists() {
            return read_trimmed(level.to_string_lossy().as_ref());
        }
    }
    None
}

/// Read the machine. Never fails: an absent path means "not present".
pub fn probe_host() -> HostProbe {
    let (tlp_enabled, tlp_active) = unit_state("tlp");
    let (_, ppd_active) = unit_state("power-profiles-daemon");
    let (_, lact_active) = unit_state("lactd");
    let (_, scx_active) = unit_state("scx_loader");

    HostProbe {
        tlp_installed: Path::new("/usr/bin/tlp").exists(),
        tlp_service_enabled: tlp_enabled,
        tlp_service_active: tlp_active,
        profile_provider: profile_provider(),
        profile_provider_active: ppd_active,
        lact_installed: Path::new("/usr/bin/lact").exists(),
        lact_daemon_active: lact_active || Path::new("/run/lactd.sock").exists(),
        gpu_performance_level: gpu_performance_level(),
        scx_loader_active: scx_active,
        platform_profile: read_trimmed("/sys/firmware/acpi/platform_profile"),
        energy_performance_preference: read_trimmed(
            "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference",
        ),
        charge_thresholds_available: Path::new(
            "/sys/class/power_supply/BAT0/charge_control_end_threshold",
        )
        .exists(),
        fan_control_available: Path::new("/proc/acpi/ibm/fan").exists(),
    }
}

/// Report who owns what, and what that costs.
#[tauri::command]
pub fn power_owners() -> ApiResponse<Ownership> {
    let probe = probe_host();
    let ownership = decide(&probe);
    ApiResponse {
        success: true,
        data: Some(ownership),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tlp_box() -> HostProbe {
        HostProbe {
            tlp_installed: true,
            tlp_service_enabled: true,
            tlp_service_active: true,
            profile_provider: Some(ProfileProvider::TlpPd),
            profile_provider_active: true,
            lact_installed: true,
            lact_daemon_active: true,
            gpu_performance_level: Some("low".to_string()),
            charge_thresholds_available: true,
            fan_control_available: true,
            ..HostProbe::default()
        }
    }

    fn ppd_box() -> HostProbe {
        HostProbe {
            profile_provider: Some(ProfileProvider::Ppd),
            profile_provider_active: true,
            charge_thresholds_available: true,
            fan_control_available: true,
            ..HostProbe::default()
        }
    }

    fn owner_of(ship: &Ownership, knob: Knob) -> Owner {
        ship.claims
            .iter()
            .find(|c| c.knob == knob)
            .map(|c| c.owner)
            .expect("every knob is claimed")
    }

    #[test]
    fn every_knob_gets_exactly_one_claim() {
        for probe in [ppd_box(), tlp_box(), HostProbe::default()] {
            let ship = decide(&probe);
            assert_eq!(ship.claims.len(), Knob::ALL.len());
            for knob in Knob::ALL {
                let count = ship.claims.iter().filter(|c| c.knob == knob).count();
                assert_eq!(count, 1, "{:?} claimed {} times", knob, count);
            }
        }
    }

    #[test]
    fn power_profiles_daemon_alone_is_a_limited_machine() {
        let ship = decide(&ppd_box());
        assert_eq!(ship.capability, Capability::LimitedPpd);
        assert_eq!(owner_of(&ship, Knob::PlatformProfile), Owner::Ppd);
        assert!(
            ship.summary.contains("tlp + tlp-pd"),
            "the summary has to name the way out: {}",
            ship.summary
        );
    }

    #[test]
    fn no_daemon_at_all_still_lets_the_app_own_sysfs() {
        let ship = decide(&HostProbe {
            charge_thresholds_available: true,
            ..HostProbe::default()
        });
        assert_eq!(ship.capability, Capability::LimitedDirect);
        assert_eq!(owner_of(&ship, Knob::CpuGovernor), Owner::ThinkPro);
        assert_eq!(owner_of(&ship, Knob::ChargeThresholds), Owner::ThinkPro);
    }

    #[test]
    fn tlp_running_unlocks_the_full_editor_and_owns_the_cpu_knobs() {
        let ship = decide(&tlp_box());
        assert_eq!(ship.capability, Capability::FullTlp);
        assert_eq!(owner_of(&ship, Knob::CpuGovernor), Owner::Tlp);
        assert_eq!(owner_of(&ship, Knob::ChargeThresholds), Owner::Tlp);
    }

    #[test]
    fn installed_but_inactive_tlp_is_inert_and_says_so() {
        let probe = HostProbe {
            tlp_installed: true,
            ..HostProbe::default()
        };
        let ship = decide(&probe);
        assert_eq!(ship.capability, Capability::LimitedDirect);
        let info = ship
            .conflicts
            .iter()
            .find(|c| c.severity == Severity::Info)
            .expect("inert TLP is reported");
        assert!(info.remedy.contains("systemctl enable"));
    }

    #[test]
    fn lact_keeps_the_gpu_even_when_tlp_is_running() {
        let ship = decide(&tlp_box());
        assert_eq!(owner_of(&ship, Knob::GpuPerformanceLevel), Owner::Lact);
        assert!(
            ship.conflicts
                .iter()
                .any(|c| c.severity == Severity::Warning
                    && c.knobs == vec![Knob::GpuPerformanceLevel]),
            "the LACT/TLP overlap is reported"
        );
    }

    #[test]
    fn a_pinned_gpu_with_no_daemon_to_release_it_is_flagged() {
        let probe = HostProbe {
            lact_installed: true,
            lact_daemon_active: false,
            gpu_performance_level: Some("low".to_string()),
            ..HostProbe::default()
        };
        let ship = decide(&probe);
        assert_eq!(owner_of(&ship, Knob::GpuPerformanceLevel), Owner::ThinkPro);
        assert!(ship.conflicts.iter().any(|c| c.remedy.contains("auto")));
    }

    #[test]
    fn tlp_and_ppd_together_is_blocking_not_a_warning() {
        let ship = decide(&tlp_box());
        let blocking = ship
            .conflicts
            .iter()
            .find(|c| c.severity == Severity::Blocking)
            .expect("two profile owners are blocking");
        assert!(blocking.remedy.contains("tlp-pd"));
    }

    #[test]
    fn scx_loader_owns_the_scheduler_when_it_is_active() {
        let mut probe = ppd_box();
        probe.scx_loader_active = true;
        assert_eq!(owner_of(&decide(&probe), Knob::Scheduler), Owner::ScxLoader);
        probe.scx_loader_active = false;
        assert_eq!(owner_of(&decide(&probe), Knob::Scheduler), Owner::Kernel);
    }

    #[test]
    fn knob_labels_are_unique_and_non_empty() {
        let mut seen = std::collections::HashSet::new();
        for knob in Knob::ALL {
            assert!(!knob.label().is_empty());
            assert!(seen.insert(knob.label()), "duplicate label {:?}", knob);
        }
    }
}
