//! What stands between this machine and working power management.
//!
//! The overview page answers "what is my laptop doing"; this answers "why can I
//! not change it". Every finding names one thing, says whether it is blocking,
//! and carries the remedy that actually applies -- either a command the user runs
//! (the app must not install system packages behind their back) or an action the
//! app can perform itself through the privileged helper.
//!
//! `evaluate` is pure over `DoctorInputs`, so every branch is reachable in a test
//! without a machine in that state.

use serde::{Deserialize, Serialize};

use super::arbiter::{self, Capability, Severity};
use super::tlp::{self, ConfigWarning};
use crate::fan_control::{self, FanReadiness};

/// Install command per package manager, as TLP packages itself.
const PACKAGE_MANAGERS: [(&str, &str); 3] = [
    ("/usr/bin/pacman", "sudo pacman -S tlp tlp-pd"),
    ("/usr/bin/apt", "sudo apt install tlp tlp-rdw"),
    ("/usr/bin/dnf", "sudo dnf install tlp tlp-rdw"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// Nothing to do.
    Ok,
    /// Worth knowing; a feature may be unavailable but nothing is wrong.
    Notice,
    /// Something the user should fix: the affected setting will not take effect.
    Problem,
    /// Two things own the same knob; this app's writes cannot stick at all.
    Blocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemedyKind {
    /// The app runs this itself when the button is pressed.
    Action,
    /// Shell text for the user to run. The app does not install packages or
    /// touch the administrator's configuration on its own.
    Command,
    /// Nothing to run.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remedy {
    pub kind: RemedyKind,
    pub label: String,
    /// A command name for `Action`, shell text for `Command`, prose for `Manual`.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable identifier, so the frontend can special-case one without matching
    /// on the title text.
    pub id: String,
    pub title: String,
    pub state: CheckState,
    pub detail: String,
    /// Files or units this finding is about.
    pub locations: Vec<String>,
    pub remedy: Option<Remedy>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub capability: Capability,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub blocking: usize,
    pub problems: usize,
}

/// Everything `evaluate` is allowed to look at.
#[derive(Debug, Clone)]
pub struct DoctorInputs {
    pub capability: Capability,
    pub ownership_summary: String,
    pub conflicts: Vec<arbiter::Conflict>,
    pub tlp_installed: bool,
    pub tlp_version: Option<String>,
    pub service_enabled: Option<bool>,
    pub service_active: Option<bool>,
    pub config_files: Vec<String>,
    pub config_warnings: Vec<ConfigWarning>,
    pub helper_path: Option<String>,
    pub helper_packaged: bool,
    pub polkit_rule_present: bool,
    pub fan_readiness: FanReadiness,
    pub fan_modprobe_conf_present: bool,
    pub charge_thresholds_available: bool,
    pub battery_present: bool,
    /// The install command for this distribution, when one is recognised.
    pub package_manager: Option<&'static str>,
}

/// TLP reports its own version on the first line of `tlp-stat -s`.
pub fn parse_tlp_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let rest = line
            .strip_prefix("--- TLP ")
            .or_else(|| line.strip_prefix("TLP "));
        if let Some(rest) = rest {
            let version: String = rest
                .trim_start_matches('v')
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }
    None
}

fn finding(
    id: &str,
    title: &str,
    state: CheckState,
    detail: String,
    locations: Vec<String>,
    remedy: Option<Remedy>,
) -> Finding {
    Finding {
        id: id.to_string(),
        title: title.to_string(),
        state,
        detail,
        locations,
        remedy,
    }
}

fn action(label: &str, command: &str) -> Remedy {
    Remedy {
        kind: RemedyKind::Action,
        label: label.to_string(),
        value: command.to_string(),
    }
}

fn shell(label: &str, command: &str) -> Remedy {
    Remedy {
        kind: RemedyKind::Command,
        label: label.to_string(),
        value: command.to_string(),
    }
}

/// State a configuration warning deserves, and how to describe it.
fn config_finding(warning: &ConfigWarning) -> Finding {
    let location = format!("{}:{}", warning.path, warning.line);
    match &warning.replacement {
        Some(next) => finding(
            "tlp.config.renamed",
            &format!("{} was renamed", warning.key),
            CheckState::Problem,
            format!(
                "TLP no longer reads {}. The value on {} is ignored until the line uses {}.",
                warning.key, location, next
            ),
            vec![location.clone()],
            Some(Remedy {
                kind: RemedyKind::Manual,
                label: "Edit the line".to_string(),
                value: format!(
                    "Line {} of {}: replace {} with {}.",
                    warning.line, warning.path, warning.key, next
                ),
            }),
        ),
        None => {
            let deprecated = warning.message.contains("deprecated");
            finding(
                if deprecated {
                    "tlp.config.deprecated"
                } else {
                    "tlp.config.unknown"
                },
                &format!("{} is not usable", warning.key),
                if deprecated {
                    CheckState::Notice
                } else {
                    CheckState::Problem
                },
                format!("{} ({})", warning.message, location),
                vec![location],
                Some(Remedy {
                    kind: RemedyKind::Manual,
                    label: "Remove the line".to_string(),
                    value: format!("Delete line {} of {}.", warning.line, warning.path),
                }),
            )
        }
    }
}

/// Turn collected facts into findings, worst first.
pub fn evaluate(inputs: &DoctorInputs) -> Diagnosis {
    let mut findings: Vec<Finding> = Vec::new();

    // 1. Two owners of one knob: nothing this app writes will survive.
    for conflict in &inputs.conflicts {
        let state = match conflict.severity {
            Severity::Blocking => CheckState::Blocking,
            Severity::Warning => CheckState::Problem,
            Severity::Info => CheckState::Notice,
        };
        findings.push(finding(
            "daemon.conflict",
            &conflict
                .knobs
                .iter()
                .map(|knob| knob.label())
                .collect::<Vec<_>>()
                .join(", "),
            state,
            conflict.message.clone(),
            Vec::new(),
            Some(Remedy {
                kind: RemedyKind::Manual,
                label: "Choose one owner".to_string(),
                value: conflict.remedy.clone(),
            }),
        ));
    }

    // 2. TLP itself.
    if !inputs.tlp_installed {
        let remedy = match inputs.package_manager {
            Some(command) => shell("Install TLP", command),
            None => Remedy {
                kind: RemedyKind::Manual,
                label: "Install TLP".to_string(),
                value: "Install the tlp package with your distribution's package manager."
                    .to_string(),
            },
        };
        findings.push(finding(
            "tlp.missing",
            "TLP is not installed",
            CheckState::Notice,
            "Without TLP the app can still write individual settings, but it cannot express per-source (AC / battery / save) values.".to_string(),
            Vec::new(),
            Some(remedy),
        ));
    } else {
        findings.push(finding(
            "tlp.present",
            "TLP is installed",
            CheckState::Ok,
            match &inputs.tlp_version {
                Some(version) => format!(
                    "TLP {} is installed and its configuration is read from {}. ",
                    version,
                    inputs
                        .config_files
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "its package defaults".to_string())
                ),
                None => "TLP is installed; its version could not be read.".to_string(),
            },
            inputs.config_files.clone(),
            None,
        ));

        let enabled = inputs.service_enabled.unwrap_or(false);
        let active = inputs.service_active.unwrap_or(false);
        if !enabled || !active {
            let detail = match (enabled, active) {
                (true, false) => "The tlp service is enabled but not running, so nothing applies the configuration.",
                (false, true) => "The tlp service is running but not enabled, so it will not come back after a reboot.",
                _ => "The tlp service is neither enabled nor running, so no setting is applied.",
            };
            findings.push(finding(
                "tlp.service",
                "The tlp service is not active",
                CheckState::Problem,
                detail.to_string(),
                vec!["tlp.service".to_string()],
                Some(shell(
                    "Enable and start it",
                    "sudo systemctl enable --now tlp",
                )),
            ));
        } else {
            findings.push(finding(
                "tlp.service",
                "The tlp service is running",
                CheckState::Ok,
                "Configuration changes take effect on the next `tlp start` or service restart."
                    .to_string(),
                vec!["tlp.service".to_string()],
                None,
            ));
        }
    }

    // 3. Configuration the editor would otherwise present as if it worked.
    for warning in &inputs.config_warnings {
        findings.push(config_finding(warning));
    }

    // 4. The privileged path the app needs for fan control and the root-only
    //    readings on the overview page.
    if inputs.helper_path.is_none() {
        findings.push(finding(
            "helper.missing",
            "The privileged helper is not installed",
            CheckState::Problem,
            "Fan speed changes and the root-only readings (serial, package power, memory speed, disk health) need it. It accepts a fixed command whitelist and nothing else.".to_string(),
            Vec::new(),
            Some(action("Install it", "setup_permissions")),
        ));
    } else if !inputs.polkit_rule_present {
        findings.push(finding(
            "polkit.rule",
            "The polkit rule is missing",
            CheckState::Problem,
            "The helper is installed, but every use will ask for a password without the rule that allows it.".to_string(),
            vec![fan_control::POLKIT_RULE_PACKAGED_PATH.to_string()],
            Some(action("Install it", "setup_permissions")),
        ));
    } else {
        findings.push(finding(
            "helper.present",
            "The privileged helper is ready",
            CheckState::Ok,
            match (inputs.helper_packaged, &inputs.helper_path) {
                (true, Some(path)) => format!(
                    "{} comes from the distribution package, so updates replace it.",
                    path
                ),
                (false, Some(path)) => format!(
                    "{} was installed by the app; a package update would install its own copy.",
                    path
                ),
                _ => "The helper is installed.".to_string(),
            },
            inputs.helper_path.iter().cloned().collect(),
            None,
        ));
    }

    // 5. Fan control: the module parameter is the one fan problem permissions
    //    cannot fix, which is why it gets its own action.
    match inputs.fan_readiness {
        FanReadiness::Ready => findings.push(finding(
            "fan.module",
            "Fan control is available",
            CheckState::Ok,
            "The kernel accepts fan level writes on this machine.".to_string(),
            vec!["/proc/acpi/ibm/fan".to_string()],
            None,
        )),
        FanReadiness::NeedsModuleParam if inputs.fan_modprobe_conf_present => findings.push(finding(
            "fan.module",
            "Fan control is configured but not active yet",
            CheckState::Problem,
            "The module parameter is written, and the running module was loaded before it existed. Reload thinkpad_acpi or reboot.".to_string(),
            vec![
                fan_control::MODPROBE_CONF_PATH.to_string(),
                "tlp.service".to_string(),
            ],
            Some(shell(
                "Reload the module",
                "sudo modprobe -r thinkpad_acpi && sudo modprobe thinkpad_acpi",
            )),
        )),
        FanReadiness::NeedsModuleParam => findings.push(finding(
            "fan.module",
            "Fan control needs a kernel module setting",
            CheckState::Problem,
            "The thinkpad_acpi module was loaded without fan_control=1, so the kernel refuses fan changes. This is a module setting, not a permissions problem.".to_string(),
            vec!["/proc/acpi/ibm/fan".to_string()],
            Some(action("Fix now", "enable_fan_control")),
        )),
        FanReadiness::NoThinkpadFan => findings.push(finding(
            "fan.module",
            "No ThinkPad fan interface",
            CheckState::Notice,
            "This model, or a module that is not loaded, does not expose /proc/acpi/ibm/fan. Fan control is unavailable.".to_string(),
            Vec::new(),
            None,
        )),
    }

    // 6. Battery features that depend on the firmware.
    if !inputs.battery_present {
        findings.push(finding(
            "battery.missing",
            "No battery reported",
            CheckState::Notice,
            "No battery device is present, so battery pages have nothing to show.".to_string(),
            Vec::new(),
            None,
        ));
    } else if !inputs.charge_thresholds_available {
        findings.push(finding(
            "battery.thresholds",
            "Charge thresholds are not exposed",
            CheckState::Notice,
            "This firmware does not offer charge start/stop thresholds, so the battery page cannot set them.".to_string(),
            Vec::new(),
            None,
        ));
    }

    let blocking = findings
        .iter()
        .filter(|finding| finding.state == CheckState::Blocking)
        .count();
    let problems = findings
        .iter()
        .filter(|finding| finding.state == CheckState::Problem)
        .count();

    let summary = if blocking > 0 {
        format!(
            "{} blocking conflict{}: power settings written by this app will not stick until one owner is chosen.",
            blocking,
            if blocking == 1 { "" } else { "s" }
        )
    } else if problems > 0 {
        format!(
            "{} problem{} found. {}",
            problems,
            if problems == 1 { "" } else { "s" },
            inputs.ownership_summary
        )
    } else {
        format!(
            "No power-management problems found. {}",
            inputs.ownership_summary
        )
    };

    Diagnosis {
        capability: inputs.capability,
        summary,
        findings,
        blocking,
        problems,
    }
}

// ---------------------------------------------------------------------------
// Host collection
// ---------------------------------------------------------------------------

fn unit_state(unit: &str) -> (Option<bool>, Option<bool>) {
    let read = |verb: &str| {
        std::process::Command::new("systemctl")
            .args([verb, unit])
            .output()
            .ok()
            .map(|output| output.status.success())
    };
    (read("is-enabled"), read("is-active"))
}

fn tlp_version() -> Option<String> {
    let output = std::process::Command::new("tlp-stat")
        .arg("-s")
        .output()
        .ok()?;
    parse_tlp_version(&String::from_utf8_lossy(&output.stdout))
}

fn package_manager() -> Option<&'static str> {
    PACKAGE_MANAGERS
        .iter()
        .find(|(binary, _)| std::path::Path::new(binary).exists())
        .map(|(_, command)| *command)
}

/// Read the host. Blocking: units, a version probe and two configuration reads.
pub fn gather() -> DoctorInputs {
    let probe = arbiter::probe_host();
    let ownership = arbiter::decide(&probe);
    let (service_enabled, service_active) = unit_state("tlp");

    let config = tlp::tlp_config();
    let (config_files, config_warnings) = match config.data {
        Some(config) => (config.files, config.warnings),
        None => (Vec::new(), Vec::new()),
    };

    let fan = fan_control::get_fan_capability().data;
    let (fan_readiness, fan_modprobe_conf_present) = match fan {
        Some(capability) => (capability.readiness, capability.modprobe_conf_present),
        None => (FanReadiness::NoThinkpadFan, false),
    };

    DoctorInputs {
        capability: ownership.capability,
        ownership_summary: ownership.summary,
        conflicts: ownership.conflicts,
        tlp_installed: probe.tlp_installed,
        tlp_version: if probe.tlp_installed {
            tlp_version()
        } else {
            None
        },
        service_enabled,
        service_active,
        config_files,
        config_warnings,
        helper_path: fan_control::helper_path().map(|path| path.to_string()),
        helper_packaged: fan_control::helper_is_packaged(),
        polkit_rule_present: std::path::Path::new(fan_control::POLKIT_RULE_PACKAGED_PATH).exists()
            || std::path::Path::new(fan_control::POLKIT_RULE_PATH).exists(),
        fan_readiness,
        fan_modprobe_conf_present,
        charge_thresholds_available: probe.charge_thresholds_available,
        battery_present: std::path::Path::new("/sys/class/power_supply/BAT0").exists(),
        package_manager: package_manager(),
    }
}

#[tauri::command]
pub async fn power_doctor() -> crate::battery::ApiResponse<Diagnosis> {
    match tokio::task::spawn_blocking(|| evaluate(&gather())).await {
        Ok(diagnosis) => crate::battery::ApiResponse {
            success: true,
            data: Some(diagnosis),
            error: None,
        },
        Err(error) => crate::battery::ApiResponse {
            success: false,
            data: None,
            error: Some(format!("The diagnosis failed to run: {}", error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::arbiter::{Conflict, Knob, Severity};

    fn inputs() -> DoctorInputs {
        DoctorInputs {
            capability: Capability::FullTlp,
            ownership_summary: "TLP owns the power knobs.".to_string(),
            conflicts: Vec::new(),
            tlp_installed: true,
            tlp_version: Some("1.10.2".to_string()),
            service_enabled: Some(true),
            service_active: Some(true),
            config_files: vec!["/usr/share/tlp/defaults.conf".to_string()],
            config_warnings: Vec::new(),
            helper_path: Some("/usr/lib/thinkpro-vantage/thinkpro-ctl".to_string()),
            helper_packaged: true,
            polkit_rule_present: true,
            fan_readiness: FanReadiness::Ready,
            fan_modprobe_conf_present: false,
            charge_thresholds_available: true,
            battery_present: true,
            package_manager: Some("sudo pacman -S tlp tlp-pd"),
        }
    }

    fn ids(diagnosis: &Diagnosis) -> Vec<&str> {
        diagnosis.findings.iter().map(|f| f.id.as_str()).collect()
    }

    fn state_of(diagnosis: &Diagnosis, id: &str) -> CheckState {
        diagnosis
            .findings
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.state)
            .unwrap_or_else(|| panic!("{} present", id))
    }

    #[test]
    fn a_healthy_machine_reports_no_problems() {
        let diagnosis = evaluate(&inputs());
        assert_eq!(diagnosis.problems, 0);
        assert_eq!(diagnosis.blocking, 0);
        assert!(diagnosis
            .summary
            .starts_with("No power-management problems found"));
        assert!(ids(&diagnosis).contains(&"tlp.present"));
        assert!(ids(&diagnosis).contains(&"helper.present"));
    }

    #[test]
    fn version_parsing_survives_the_real_reports() {
        assert_eq!(
            parse_tlp_version("--- TLP 1.10.2 ---------------------------------\n"),
            Some("1.10.2".to_string())
        );
        assert_eq!(
            parse_tlp_version("TLP 1.7\nmore\n"),
            Some("1.7".to_string())
        );
        assert_eq!(parse_tlp_version("--- System Info ---\n"), None);
        assert_eq!(parse_tlp_version(""), None);
    }

    #[test]
    fn a_stopped_service_is_a_problem_with_the_command_to_start_it() {
        let mut probe = inputs();
        probe.service_active = Some(false);
        let diagnosis = evaluate(&probe);
        assert_eq!(state_of(&diagnosis, "tlp.service"), CheckState::Problem);
        let remedy = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "tlp.service")
            .and_then(|f| f.remedy.clone())
            .expect("remedy");
        assert_eq!(remedy.kind, RemedyKind::Command);
        assert!(remedy.value.contains("systemctl enable --now tlp"));
        assert_eq!(diagnosis.problems, 1);
    }

    #[test]
    fn a_blocking_conflict_outranks_every_other_problem() {
        let mut probe = inputs();
        probe.capability = Capability::LimitedPpd;
        probe.tlp_installed = false;
        probe.conflicts = vec![Conflict {
            severity: Severity::Blocking,
            message: "TLP and power-profiles-daemon both write the platform profile.".to_string(),
            knobs: vec![Knob::PlatformProfile],
            remedy: "sudo systemctl disable --now power-profiles-daemon".to_string(),
        }];
        let diagnosis = evaluate(&probe);
        assert_eq!(diagnosis.blocking, 1);
        assert_eq!(
            state_of(&diagnosis, "daemon.conflict"),
            CheckState::Blocking
        );
        assert!(diagnosis.summary.contains("blocking conflict"));
        // The member that clears the conflict has to name every knob involved.
        let conflict = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "daemon.conflict")
            .expect("conflict reported");
        assert!(!conflict.title.is_empty());
    }

    #[test]
    fn a_missing_helper_is_an_action_the_app_can_take() {
        let mut probe = inputs();
        probe.helper_path = None;
        let diagnosis = evaluate(&probe);
        let remedy = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "helper.missing")
            .and_then(|f| f.remedy.clone())
            .expect("remedy");
        assert_eq!(remedy.kind, RemedyKind::Action);
        assert_eq!(remedy.value, "setup_permissions");
    }

    #[test]
    fn a_helper_without_the_rule_says_what_will_happen() {
        let mut probe = inputs();
        probe.polkit_rule_present = false;
        let diagnosis = evaluate(&probe);
        assert_eq!(state_of(&diagnosis, "polkit.rule"), CheckState::Problem);
        assert!(!ids(&diagnosis).contains(&"helper.present"));
    }

    #[test]
    fn the_module_parameter_case_offers_the_action_and_the_reload() {
        let mut probe = inputs();
        probe.fan_readiness = FanReadiness::NeedsModuleParam;
        let diagnosis = evaluate(&probe);
        let remedy = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "fan.module")
            .and_then(|f| f.remedy.clone())
            .expect("remedy");
        assert_eq!(remedy.kind, RemedyKind::Action);
        assert_eq!(remedy.value, "enable_fan_control");

        // Once the setting is on disk the remaining step is a reload, not the fix.
        probe.fan_modprobe_conf_present = true;
        let diagnosis = evaluate(&probe);
        let remedy = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "fan.module")
            .and_then(|f| f.remedy.clone())
            .expect("remedy");
        assert_eq!(remedy.kind, RemedyKind::Command);
        assert!(remedy.value.contains("modprobe -r thinkpad_acpi"));
    }

    #[test]
    fn configuration_warnings_become_findings_with_their_line() {
        let mut probe = inputs();
        probe.config_warnings = vec![
            ConfigWarning {
                key: "CPU_HWP_ON_AC".to_string(),
                message: "renamed to CPU_ENERGY_PERF_POLICY_ON_AC".to_string(),
                replacement: Some("CPU_ENERGY_PERF_POLICY_ON_AC".to_string()),
                path: "/etc/tlp.conf".to_string(),
                line: 12,
            },
            ConfigWarning {
                key: "USB_AUTOSUSPND".to_string(),
                message: "not a parameter this TLP version knows".to_string(),
                replacement: None,
                path: "/etc/tlp.conf".to_string(),
                line: 20,
            },
            ConfigWarning {
                key: "RESTORE_DEVICE_STATE_ON_STARTUP".to_string(),
                message: "Parameter is deprecated: feature will be removed".to_string(),
                replacement: None,
                path: "/usr/share/tlp/defaults.conf".to_string(),
                line: 63,
            },
        ];
        let diagnosis = evaluate(&probe);
        assert_eq!(diagnosis.problems, 2);
        assert_eq!(
            state_of(&diagnosis, "tlp.config.renamed"),
            CheckState::Problem
        );
        assert_eq!(
            state_of(&diagnosis, "tlp.config.unknown"),
            CheckState::Problem
        );
        assert_eq!(
            state_of(&diagnosis, "tlp.config.deprecated"),
            CheckState::Notice
        );
        let renamed = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "tlp.config.renamed")
            .expect("renamed");
        assert_eq!(renamed.locations, vec!["/etc/tlp.conf:12"]);
        assert!(renamed
            .remedy
            .as_ref()
            .expect("remedy")
            .value
            .contains("CPU_ENERGY_PERF_POLICY_ON_AC"));
    }

    #[test]
    fn a_machine_without_tlp_gets_its_own_install_command() {
        let mut probe = inputs();
        probe.tlp_installed = false;
        probe.capability = Capability::LimitedPpd;
        probe.service_enabled = None;
        probe.service_active = None;
        let diagnosis = evaluate(&probe);
        let remedy = diagnosis
            .findings
            .iter()
            .find(|f| f.id == "tlp.missing")
            .and_then(|f| f.remedy.clone())
            .expect("remedy");
        assert!(
            remedy.value.contains("pacman -S tlp"),
            "got {}",
            remedy.value
        );
        // Nothing claims the service state of a package that is not installed.
        assert!(!ids(&diagnosis).contains(&"tlp.service"));
    }

    #[test]
    fn firmware_limits_are_notices_not_problems() {
        let mut probe = inputs();
        probe.charge_thresholds_available = false;
        probe.fan_readiness = FanReadiness::NoThinkpadFan;
        let diagnosis = evaluate(&probe);
        assert_eq!(diagnosis.problems, 0);
        assert!(!ids(&diagnosis).contains(&"battery.missing"));
        assert_eq!(
            state_of(&diagnosis, "battery.thresholds"),
            CheckState::Notice
        );
        assert_eq!(state_of(&diagnosis, "fan.module"), CheckState::Notice);
    }

    #[test]
    fn a_machine_without_a_battery_reports_that_instead_of_thresholds() {
        // Thresholds cannot be exposed when there is no battery to charge, so the
        // notice would be noise.
        let mut probe = inputs();
        probe.battery_present = false;
        probe.charge_thresholds_available = false;
        let diagnosis = evaluate(&probe);
        assert_eq!(state_of(&diagnosis, "battery.missing"), CheckState::Notice);
        assert!(!ids(&diagnosis).contains(&"battery.thresholds"));
    }
}
