//! TLP configuration: schema, effective values, and provenance.
//!
//! TLP ships no machine-readable schema. What it does ship is two files that
//! together describe every setting:
//!
//! * `/usr/share/tlp/defaults.conf` — bare `KEY=value` lines: the intrinsic
//!   defaults, which give us the complete key list and each key's default value.
//! * `/etc/tlp.conf` — the commented template, where the block above each key
//!   documents the setting, lists the accepted enum values, and marks the
//!   default with `(*)`.
//! * `/usr/share/tlp/rename.conf` and `deprecated.conf` — the migration tables.
//!
//! These are read from the installed package at runtime, deliberately. Copying
//! TLP's text into this repository would both age (TLP 1.3 to 1.10 each changed
//! keys) and drag a GPL-2.0-or-later document into a tree that is LGPL-3.0
//! throughout. Reading the files the user already has keeps the schema correct
//! for whatever version is installed.
//!
//! Everything that needs TLP installed returns an explicit "unavailable" answer
//! instead of pretending: the UI has to gate on that anyway.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::battery::ApiResponse;

pub const DEFAULTS_CONF: &str = "/usr/share/tlp/defaults.conf";
pub const TEMPLATE_CONF: &str = "/etc/tlp.conf";
pub const DROPIN_DIR: &str = "/etc/tlp.d";
pub const RENAME_CONF: &str = "/usr/share/tlp/rename.conf";
pub const DEPRECATED_CONF: &str = "/usr/share/tlp/deprecated.conf";

/// Which power source a setting's variant applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Ac,
    Bat,
    Sav,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Ac => "AC",
            Source::Bat => "Battery",
            Source::Sav => "Power saver",
        }
    }
}

/// Editor grouping. TLP's template has no reliable section markers, so the
/// grouping is this application's own taxonomy over the key prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    General,
    Cpu,
    Platform,
    Graphics,
    Disk,
    Pcie,
    Usb,
    Radio,
    Sound,
    Battery,
    Other,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::General => "General",
            Group::Cpu => "CPU",
            Group::Platform => "Platform",
            Group::Graphics => "Graphics",
            Group::Disk => "Disks",
            Group::Pcie => "PCIe",
            Group::Usb => "USB",
            Group::Radio => "Radios",
            Group::Sound => "Audio",
            Group::Battery => "Battery care",
            Group::Other => "Other",
        }
    }

    /// Display order in the editor.
    pub const ORDER: [Group; 11] = [
        Group::General,
        Group::Cpu,
        Group::Platform,
        Group::Graphics,
        Group::Disk,
        Group::Pcie,
        Group::Usb,
        Group::Radio,
        Group::Sound,
        Group::Battery,
        Group::Other,
    ];
}

/// What kind of value a setting takes, as far as the shipped text lets us tell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ValueKind {
    Bool,
    Int,
    Enum {
        values: Vec<String>,
        default: Option<String>,
    },
    Text,
}

/// Documentation block above one setting in the template.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingDoc {
    pub description: String,
    pub default_note: Option<String>,
    pub requires: Option<String>,
    pub enum_values: Vec<String>,
    pub enum_default: Option<String>,
    /// The description spells out Y/N, so the editor can show a switch.
    pub bool_hint: bool,
}

/// One editable setting: a row, plus which source variant it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaEntry {
    pub key: String,
    /// Key without the `_ON_AC`/`_ON_BAT`/`_ON_SAV` suffix.
    pub row: String,
    pub source: Option<Source>,
    pub group: Group,
    pub group_label: String,
    pub kind: ValueKind,
    pub default_value: Option<String>,
    pub doc: Option<SettingDoc>,
    /// True when TLP's own docs say the parameter is deprecated or removed.
    pub deprecated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlpSchema {
    /// False when TLP is not installed; `reason` then says what is missing.
    pub available: bool,
    pub reason: Option<String>,
    pub entries: Vec<SchemaEntry>,
    /// Keys the installed TLP has renamed, old -> new.
    pub renamed: BTreeMap<String, String>,
}

/// Where one effective value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub path: String,
    pub line: usize,
    /// `defaults` for the intrinsic defaults, `dropin` for /etc/tlp.d,
    /// `user` for /etc/tlp.conf.
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigWarning {
    pub key: String,
    pub message: String,
    /// The replacement key, when TLP renamed it.
    pub replacement: Option<String>,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlpConfig {
    pub available: bool,
    pub reason: Option<String>,
    /// Effective key -> value after defaults, drop-ins and tlp.conf are merged.
    pub values: BTreeMap<String, String>,
    /// Effective key -> where that value came from.
    pub provenance: BTreeMap<String, Provenance>,
    /// Every file consulted, in the order TLP reads them.
    pub files: Vec<String>,
    pub warnings: Vec<ConfigWarning>,
}

// ---------------------------------------------------------------------------
// Parsing (pure; every function below takes the file contents as text)
// ---------------------------------------------------------------------------

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// `KEY=value` lines, comments and blank lines ignored.
pub fn parse_defaults(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            out.insert(key.trim().to_string(), unquote(value));
        }
    }
    out
}

fn is_enum_candidate(line: &str) -> Option<(Vec<String>, Option<String>)> {
    // `#   active, passive, guided(*).` — a comma list of bare words, one of
    // which may be marked as the default.
    let body = line.trim_start_matches('#').trim();
    if body.is_empty() || !body.contains(',') {
        return None;
    }
    let body = body.trim_end_matches('.').trim();
    let mut values = Vec::new();
    let mut default = None;
    for part in body.split(',') {
        let part = part.trim();
        let (value, marked) = match part.strip_suffix("(*)") {
            Some(value) => (value.trim(), true),
            None => (part, false),
        };
        if value.is_empty()
            || !value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return None;
        }
        if marked {
            default = Some(value.to_string());
        }
        values.push(value.to_string());
    }
    if values.len() < 2 {
        return None;
    }
    Some((values, default))
}

fn doc_from_block(block: &[String]) -> SettingDoc {
    let mut doc = SettingDoc::default();
    let mut description: Vec<String> = Vec::new();
    let mut requires: Vec<String> = Vec::new();
    let mut in_requires = false;

    for raw in block {
        let line = raw.trim_start_matches('#');
        let line = line.strip_prefix(' ').unwrap_or(line);
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Default:") {
            doc.default_note = Some(rest.trim().to_string());
            // Anything between a Default line and the key is boilerplate.
            continue;
        }
        if trimmed.starts_with("Requires:") {
            in_requires = true;
            let rest = trimmed.trim_start_matches("Requires:").trim();
            if !rest.is_empty() {
                requires.push(rest.to_string());
            }
            continue;
        }
        if in_requires {
            // One entry per requirement, each with its own indented continuation
            // lines. Everything until the block ends belongs to the requirement
            // list, so nothing leaks into the description.
            requires.push(trimmed.to_string());
            continue;
        }
        if let Some((values, default)) = is_enum_candidate(raw) {
            doc.enum_values = values;
            doc.enum_default = default;
            continue;
        }
        description.push(trimmed.to_string());
    }

    doc.description = description
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !requires.is_empty() {
        doc.requires = Some(requires.join(" "));
    }
    let lowered = doc.description.to_lowercase();
    doc.bool_hint = lowered.contains("y/n")
        || lowered.contains("yes/no")
        || lowered.contains("activate: y")
        || lowered.contains("(y/n)");
    doc
}

fn looks_like_key(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Documentation for every key the template mentions, commented out or not.
///
/// Commented-out keys are the normal case: TLP's template ships every
/// assignment disabled. Treating those lines as ordinary comments is a silent
/// failure — the schema simply comes out empty — so the key check runs first.
pub fn parse_template(text: &str) -> BTreeMap<String, SettingDoc> {
    let mut out = BTreeMap::new();
    let mut block: Vec<String> = Vec::new();
    let mut last_block: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !block.is_empty() {
                last_block = std::mem::take(&mut block);
            }
            continue;
        }
        let candidate = trimmed.trim_start_matches('#').trim();
        if let Some((key_part, _)) = candidate.split_once('=') {
            let key = key_part.trim().trim_end_matches('+').trim();
            if looks_like_key(key) {
                if !last_block.is_empty() {
                    out.insert(key.to_string(), doc_from_block(&last_block));
                }
                block.clear();
                last_block.clear();
                continue;
            }
        }
        if trimmed.starts_with('#') {
            block.push(trimmed.to_string());
            continue;
        }
        block.clear();
        last_block.clear();
    }
    out
}

/// `OLD_KEY   NEW_KEY` pairs.
pub fn parse_rename(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(old), Some(new)) = (parts.next(), parts.next()) {
            out.insert(old.to_string(), new.to_string());
        }
    }
    out
}

/// `KEY # reason` pairs.
pub fn parse_deprecated(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once('#') {
            Some((key, reason)) => {
                out.insert(key.trim().to_string(), reason.trim().to_string());
            }
            None => {
                out.insert(line.to_string(), String::new());
            }
        }
    }
    out
}

/// Strip the per-source suffix, if there is one.
pub fn split_source(key: &str) -> (String, Option<Source>) {
    for (suffix, source) in [
        ("_ON_AC", Source::Ac),
        ("_ON_BAT", Source::Bat),
        ("_ON_SAV", Source::Sav),
    ] {
        if let Some(row) = key.strip_suffix(suffix) {
            return (row.to_string(), Some(source));
        }
    }
    (key.to_string(), None)
}

pub fn group_of(key: &str) -> Group {
    let prefixes: [(&str, Group); 20] = [
        ("CPU_", Group::Cpu),
        ("PLATFORM_PROFILE", Group::Platform),
        ("INTEL_GPU", Group::Graphics),
        ("RADEON_", Group::Graphics),
        ("AMDGPU_", Group::Graphics),
        ("DISK_", Group::Disk),
        ("AHCI_", Group::Disk),
        ("SATA_", Group::Disk),
        ("BAY_", Group::Disk),
        ("PCIE_", Group::Pcie),
        ("RUNTIME_PM", Group::Pcie),
        ("USB_", Group::Usb),
        ("WIFI_", Group::Radio),
        ("WOL_", Group::Radio),
        ("SOUND_", Group::Sound),
        ("NATACPI_", Group::Battery),
        ("TPSMAPI_", Group::Battery),
        ("RESTORE_", Group::Battery),
        ("TLP_", Group::General),
        ("NMI_", Group::General),
    ];
    for (prefix, group) in prefixes {
        if key.starts_with(prefix) {
            return group;
        }
    }
    Group::Other
}

fn kind_of(key: &str, default: Option<&str>, doc: Option<&SettingDoc>) -> ValueKind {
    if let Some(doc) = doc {
        if !doc.enum_values.is_empty() {
            return ValueKind::Enum {
                values: doc.enum_values.clone(),
                default: doc.enum_default.clone(),
            };
        }
        if doc.bool_hint {
            return ValueKind::Bool;
        }
    }
    match default.map(str::trim) {
        Some("0") | Some("1") | Some("Y") | Some("N") => ValueKind::Bool,
        Some(value) if value.parse::<i64>().is_ok() => ValueKind::Int,
        Some(_) if key.ends_with("_SECS") || key.starts_with("MAX_LOST") => ValueKind::Int,
        _ => ValueKind::Text,
    }
}

/// Assemble the editor schema from what TLP ships: the default values give the
/// key list, the template gives the documentation, and the deprecation table
/// marks the entries the editor must not present as ordinary settings.
pub fn build_schema(
    defaults: &BTreeMap<String, String>,
    template: &BTreeMap<String, SettingDoc>,
    deprecated: &BTreeMap<String, String>,
) -> Vec<SchemaEntry> {
    let mut keys: Vec<&String> = defaults.keys().chain(template.keys()).collect();
    keys.sort();
    keys.dedup();

    let mut entries = Vec::with_capacity(keys.len());
    for key in keys {
        let (row, source) = split_source(key);
        let group = group_of(key);
        let default_value = defaults.get(key).cloned();
        let doc = template.get(key).cloned();
        entries.push(SchemaEntry {
            key: key.clone(),
            row,
            source,
            group,
            group_label: group.label().to_string(),
            kind: kind_of(key, default_value.as_deref(), doc.as_ref()),
            default_value,
            doc,
            deprecated: deprecated.get(key).cloned(),
        });
    }
    entries
}

/// One parsed `KEY=value` assignment, with where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub key: String,
    pub value: String,
    /// `KEY+=` means "add to the list" rather than "replace it".
    pub append: bool,
    pub line: usize,
}

/// Parse a config file: comments, blanks and `KEY=value` / `KEY+="value"`.
pub fn parse_config(text: &str) -> Vec<Assignment> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key_part, value_part) = match line.split_once('=') {
            Some(parts) => parts,
            None => continue,
        };
        let key_part = key_part.trim();
        let append = key_part.ends_with('+');
        let key = key_part.trim_end_matches('+').trim().to_string();
        if key.is_empty() {
            continue;
        }
        out.push(Assignment {
            key,
            value: unquote(value_part),
            append,
            line: index + 1,
        });
    }
    out
}

fn origin_of(path: &str) -> &'static str {
    if path == DEFAULTS_CONF {
        "defaults"
    } else if path.starts_with(DROPIN_DIR) {
        "dropin"
    } else {
        "user"
    }
}

/// Merge the files in the order TLP reads them: defaults, then /etc/tlp.d in
/// lexical order, then /etc/tlp.conf — which therefore wins over any drop-in.
pub fn merge_config(
    files: &[(String, String)],
    known_keys: &[String],
    renamed: &BTreeMap<String, String>,
    deprecated: &BTreeMap<String, String>,
) -> (
    BTreeMap<String, String>,
    BTreeMap<String, Provenance>,
    Vec<ConfigWarning>,
) {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut provenance: BTreeMap<String, Provenance> = BTreeMap::new();
    let mut warnings = Vec::new();

    for (path, text) in files {
        for assignment in parse_config(text) {
            let mut key = assignment.key.clone();
            if let Some(replacement) = renamed.get(&key) {
                warnings.push(ConfigWarning {
                    key: key.clone(),
                    message: format!("renamed to {}", replacement),
                    replacement: Some(replacement.clone()),
                    path: path.clone(),
                    line: assignment.line,
                });
                key = replacement.clone();
            }
            if let Some(reason) = deprecated.get(&key) {
                let message = if reason.is_empty() {
                    "deprecated in this TLP version".to_string()
                } else {
                    reason.clone()
                };
                warnings.push(ConfigWarning {
                    key: key.clone(),
                    message,
                    replacement: None,
                    path: path.clone(),
                    line: assignment.line,
                });
            }
            if !known_keys.iter().any(|k| k == &key) {
                warnings.push(ConfigWarning {
                    key: key.clone(),
                    message: "not a parameter this TLP version knows".to_string(),
                    replacement: None,
                    path: path.clone(),
                    line: assignment.line,
                });
            }

            let merged = if assignment.append {
                match values.get(&key) {
                    Some(existing) if !existing.is_empty() => {
                        format!("{} {}", existing, assignment.value)
                    }
                    _ => assignment.value.clone(),
                }
            } else {
                assignment.value.clone()
            };
            values.insert(key.clone(), merged);
            provenance.insert(
                key,
                Provenance {
                    path: path.clone(),
                    line: assignment.line,
                    origin: origin_of(path).to_string(),
                },
            );
        }
    }

    (values, provenance, warnings)
}

// ---------------------------------------------------------------------------
// Reading the machine
// ---------------------------------------------------------------------------

fn read_optional(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn read_required(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("cannot read {}: {}", path, error))
}

/// Config files in TLP's own read order, skipping the ones that are absent.
pub fn config_files() -> Vec<String> {
    let mut files = Vec::new();
    if Path::new(DEFAULTS_CONF).exists() {
        files.push(DEFAULTS_CONF.to_string());
    }
    let mut dropins: Vec<PathBuf> = std::fs::read_dir(DROPIN_DIR)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().map(|e| e == "conf").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    dropins.sort();
    files.extend(dropins.iter().map(|p| p.to_string_lossy().to_string()));
    if Path::new(TEMPLATE_CONF).exists() {
        files.push(TEMPLATE_CONF.to_string());
    }
    files
}

fn unavailable(reason: String) -> (bool, Option<String>) {
    (false, Some(reason))
}

#[tauri::command]
pub fn tlp_schema() -> ApiResponse<TlpSchema> {
    let defaults_text = match read_required(DEFAULTS_CONF) {
        Ok(text) => text,
        Err(error) => {
            let (available, reason) = unavailable(error);
            return ApiResponse {
                success: true,
                data: Some(TlpSchema {
                    available,
                    reason,
                    entries: Vec::new(),
                    renamed: BTreeMap::new(),
                }),
                error: None,
            };
        }
    };

    let defaults = parse_defaults(&defaults_text);
    let template = read_optional(TEMPLATE_CONF)
        .map(|text| parse_template(&text))
        .unwrap_or_default();
    let renamed = read_optional(RENAME_CONF)
        .map(|text| parse_rename(&text))
        .unwrap_or_default();
    let deprecated = read_optional(DEPRECATED_CONF)
        .map(|text| parse_deprecated(&text))
        .unwrap_or_default();

    let entries = build_schema(&defaults, &template, &deprecated);
    ApiResponse {
        success: true,
        data: Some(TlpSchema {
            available: true,
            reason: if template.is_empty() {
                Some(format!(
                    "{} is missing, so descriptions and enum values are unavailable",
                    TEMPLATE_CONF
                ))
            } else {
                None
            },
            entries,
            renamed,
        }),
        error: None,
    }
}

#[tauri::command]
pub fn tlp_config() -> ApiResponse<TlpConfig> {
    let files = config_files();
    if files.is_empty() {
        let (available, reason) = unavailable(format!(
            "no TLP configuration found: {} is absent",
            DEFAULTS_CONF
        ));
        return ApiResponse {
            success: true,
            data: Some(TlpConfig {
                available,
                reason,
                values: BTreeMap::new(),
                provenance: BTreeMap::new(),
                files,
                warnings: Vec::new(),
            }),
            error: None,
        };
    }

    let mut loaded: Vec<(String, String)> = Vec::with_capacity(files.len());
    for path in &files {
        match read_required(path) {
            Ok(text) => loaded.push((path.clone(), text)),
            Err(error) => {
                let (available, reason) = unavailable(error);
                return ApiResponse {
                    success: true,
                    data: Some(TlpConfig {
                        available,
                        reason,
                        values: BTreeMap::new(),
                        provenance: BTreeMap::new(),
                        files,
                        warnings: Vec::new(),
                    }),
                    error: None,
                };
            }
        }
    }

    let defaults_text = loaded
        .iter()
        .find(|(path, _)| path == DEFAULTS_CONF)
        .map(|(_, text)| text.clone())
        .unwrap_or_default();
    let known: Vec<String> = parse_defaults(&defaults_text).keys().cloned().collect();
    let renamed = read_optional(RENAME_CONF)
        .map(|text| parse_rename(&text))
        .unwrap_or_default();
    let deprecated = read_optional(DEPRECATED_CONF)
        .map(|text| parse_deprecated(&text))
        .unwrap_or_default();

    let (values, provenance, warnings) = merge_config(&loaded, &known, &renamed, &deprecated);
    ApiResponse {
        success: true,
        data: Some(TlpConfig {
            available: true,
            reason: None,
            values,
            provenance,
            files,
            warnings,
        }),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The fixture text below is written here rather than copied from TLP. The
    // parser has to cope with whatever the installed package contains, so what
    // matters is that these are the shapes TLP emits, not TLP's exact words.
    const DEFAULTS: &str = r#"# /usr/share/tlp/defaults.conf - TLP intrinsic defaults
TLP_ENABLE=1
TLP_WARN_LEVEL=3
TLP_MSG_COLORS="91 93 1 92"
DISK_IDLE_SECS_ON_AC=0
DISK_IDLE_SECS_ON_BAT=2
CPU_ENERGY_PERF_POLICY_ON_AC=balance_performance
CPU_ENERGY_PERF_POLICY_ON_BAT=balance_power
CPU_ENERGY_PERF_POLICY_ON_SAV=power
CPU_SCALING_GOVERNOR_ON_AC=performance
PLATFORM_PROFILE_ON_SAV=low-power
USB_AUTOSUSPEND=1
WIFI_PWR_ON_AC=off
RADEON_DPM_PERF_LEVEL_ON_AC=auto
"#;

    const TEMPLATE: &str = r#"# ------------------------------------------------------------------------------
# General
# ------------------------------------------------------------------------------

# Enable TLP (Y/N).
# Default: Y

TLP_ENABLE=1

# Set CPU energy/performance policies EPP and EPB:
#   performance, balance_performance, default, balance_power, power.
# Values are given in order of increasing power saving.
# Requires:
# * Intel CPU
#   EPP: Intel Core i 6th gen. or newer CPU with intel_pstate driver
# * AMD Zen 2 or newer CPU
#   EPP: amd-pstate driver in active mode ('amd-pstate-epp') as of kernel 6.3
# Default: balance_performance (AC), balance_power (BAT), power (SAV)

#CPU_ENERGY_PERF_POLICY_ON_AC=balance_performance

# Disk idle timeout, seconds.
# Default: 0

#DISK_IDLE_SECS_ON_AC=0

# Allow USB autosuspend (Y/N).
# Default: Y

#USB_AUTOSUSPEND=1
"#;

    const RENAME: &str = r#"CPU_HWP_ON_AC                   CPU_ENERGY_PERF_POLICY_ON_AC
USB_BLACKLIST                   USB_DENYLIST
"#;

    const DEPRECATED: &str =
        "RESTORE_DEVICE_STATE_ON_STARTUP # Parameter is deprecated: feature will be removed\n";

    fn defaults() -> BTreeMap<String, String> {
        parse_defaults(DEFAULTS)
    }

    fn fixture_schema() -> Vec<SchemaEntry> {
        build_schema(
            &defaults(),
            &parse_template(TEMPLATE),
            &parse_deprecated(DEPRECATED),
        )
    }

    fn entry<'a>(schema: &'a [SchemaEntry], key: &str) -> &'a SchemaEntry {
        schema
            .iter()
            .find(|entry| entry.key == key)
            .unwrap_or_else(|| panic!("{} is in the schema", key))
    }

    #[test]
    fn defaults_keep_quoted_values_intact_and_skip_comments() {
        let parsed = defaults();
        assert_eq!(parsed.get("TLP_ENABLE").map(String::as_str), Some("1"));
        assert_eq!(
            parsed.get("TLP_MSG_COLORS").map(String::as_str),
            Some("91 93 1 92")
        );
        assert_eq!(parsed.len(), 13, "only key=value lines are kept");
    }

    #[test]
    fn template_yields_description_default_and_enum_candidates() {
        let docs = parse_template(TEMPLATE);
        let doc = docs
            .get("CPU_ENERGY_PERF_POLICY_ON_AC")
            .expect("documented key");
        assert!(doc
            .description
            .starts_with("Set CPU energy/performance policies"));
        assert_eq!(
            doc.enum_values,
            vec![
                "performance",
                "balance_performance",
                "default",
                "balance_power",
                "power"
            ]
        );
        assert!(
            doc.requires.as_deref().unwrap_or("").contains("amd-pstate"),
            "requires text is captured: {:?}",
            doc.requires
        );
        assert!(doc
            .default_note
            .as_deref()
            .unwrap_or("")
            .contains("balance_performance"));
    }

    #[test]
    fn enum_candidates_need_a_comma_and_bare_words() {
        assert!(is_enum_candidate("#   active, passive.").is_some());
        assert!(is_enum_candidate("#   active, passive, guided(*).").is_some());
        assert_eq!(
            is_enum_candidate("#   active, passive, guided(*).")
                .expect("candidates")
                .1,
            Some("guided".to_string())
        );
        // Prose must not be mistaken for a value list.
        assert!(is_enum_candidate("# Unless one of them is set, TLP does nothing.").is_none());
        assert!(
            is_enum_candidate("# Values are given in order of increasing power saving.").is_none()
        );
    }

    #[test]
    fn a_y_n_description_marks_the_key_as_boolean() {
        let docs = parse_template(TEMPLATE);
        assert!(docs.get("TLP_ENABLE").expect("doc").bool_hint);
        assert!(!docs.get("DISK_IDLE_SECS_ON_AC").expect("doc").bool_hint);
    }

    #[test]
    fn source_suffix_splits_into_rows() {
        assert_eq!(
            split_source("CPU_ENERGY_PERF_POLICY_ON_AC"),
            ("CPU_ENERGY_PERF_POLICY".to_string(), Some(Source::Ac))
        );
        assert_eq!(
            split_source("PLATFORM_PROFILE_ON_SAV"),
            ("PLATFORM_PROFILE".to_string(), Some(Source::Sav))
        );
        assert_eq!(
            split_source("USB_AUTOSUSPEND"),
            ("USB_AUTOSUSPEND".to_string(), None)
        );
    }

    #[test]
    fn groups_come_from_the_key_prefix() {
        assert_eq!(group_of("CPU_BOOST_ON_AC"), Group::Cpu);
        assert_eq!(group_of("RADEON_DPM_PERF_LEVEL_ON_BAT"), Group::Graphics);
        assert_eq!(group_of("USB_AUTOSUSPEND"), Group::Usb);
        assert_eq!(group_of("RESTORE_THRESHOLDS_ON_BAT"), Group::Battery);
        assert_eq!(group_of("TLP_ENABLE"), Group::General);
        assert_eq!(group_of("SOMETHING_NEW"), Group::Other);
    }

    #[test]
    fn kind_is_inferred_from_defaults_enums_and_prose() {
        let schema = fixture_schema();
        assert_eq!(entry(&schema, "USB_AUTOSUSPEND").kind, ValueKind::Bool);
        assert_eq!(entry(&schema, "TLP_WARN_LEVEL").kind, ValueKind::Int);
        assert_eq!(entry(&schema, "DISK_IDLE_SECS_ON_BAT").kind, ValueKind::Int);
        assert_eq!(entry(&schema, "TLP_ENABLE").kind, ValueKind::Bool);
        assert!(matches!(
            entry(&schema, "CPU_ENERGY_PERF_POLICY_ON_AC").kind,
            ValueKind::Enum { .. }
        ));
        // A key with no documentation and a non-numeric default stays text.
        assert_eq!(entry(&schema, "WIFI_PWR_ON_AC").kind, ValueKind::Text);
    }

    #[test]
    fn schema_covers_every_key_from_defaults_and_the_template() {
        let schema = fixture_schema();
        for expected in [
            "TLP_ENABLE",
            "CPU_ENERGY_PERF_POLICY_ON_AC",
            "CPU_ENERGY_PERF_POLICY_ON_BAT",
            "CPU_ENERGY_PERF_POLICY_ON_SAV",
            "DISK_IDLE_SECS_ON_BAT",
            "USB_AUTOSUSPEND",
            "PLATFORM_PROFILE_ON_SAV",
        ] {
            entry(&schema, expected);
        }
        // The three source variants share a row, which is what the editor groups on.
        let rows = schema
            .iter()
            .filter(|e| e.row == "CPU_ENERGY_PERF_POLICY")
            .count();
        assert_eq!(rows, 3);
        assert_eq!(
            entry(&schema, "PLATFORM_PROFILE_ON_SAV").source,
            Some(Source::Sav)
        );
        assert_eq!(entry(&schema, "USB_AUTOSUSPEND").source, None);
    }

    #[test]
    fn deprecated_keys_carry_the_reason_from_tlps_own_table() {
        let deprecated = parse_deprecated(DEPRECATED);
        assert!(deprecated
            .get("RESTORE_DEVICE_STATE_ON_STARTUP")
            .expect("entry")
            .contains("deprecated"));

        // A deprecated key that is also a default gets flagged on its entry.
        let marked = build_schema(
            &parse_defaults("RESTORE_DEVICE_STATE_ON_STARTUP=0\n"),
            &BTreeMap::new(),
            &deprecated,
        );
        assert!(marked[0].deprecated.is_some());
    }

    #[test]
    fn merge_follows_tlps_read_order_where_tlp_conf_wins() {
        let files = vec![
            (DEFAULTS_CONF.to_string(), DEFAULTS.to_string()),
            (
                format!("{}/10-work.conf", DROPIN_DIR),
                "TLP_WARN_LEVEL=1\nUSB_AUTOSUSPEND=0\n".to_string(),
            ),
            (TEMPLATE_CONF.to_string(), "TLP_WARN_LEVEL=2\n".to_string()),
        ];
        let known: Vec<String> = defaults().keys().cloned().collect();
        let (values, provenance, warnings) =
            merge_config(&files, &known, &BTreeMap::new(), &BTreeMap::new());

        assert_eq!(values.get("TLP_WARN_LEVEL").map(String::as_str), Some("2"));
        assert_eq!(
            provenance.get("TLP_WARN_LEVEL").map(|p| p.origin.as_str()),
            Some("user")
        );
        assert_eq!(values.get("USB_AUTOSUSPEND").map(String::as_str), Some("0"));
        assert_eq!(
            provenance.get("USB_AUTOSUSPEND").map(|p| p.origin.as_str()),
            Some("dropin")
        );
        assert!(warnings.is_empty(), "{:?}", warnings);
    }

    #[test]
    fn append_assignments_add_to_the_list_instead_of_replacing_it() {
        let files = vec![(
            TEMPLATE_CONF.to_string(),
            "USB_DENYLIST=\"a b\"\nUSB_DENYLIST+=\"c\"\n".to_string(),
        )];
        let known = vec!["USB_DENYLIST".to_string()];
        let (values, _, warnings) =
            merge_config(&files, &known, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(
            values.get("USB_DENYLIST").map(String::as_str),
            Some("a b c")
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn renamed_keys_are_followed_and_reported() {
        let files = vec![(
            TEMPLATE_CONF.to_string(),
            "CPU_HWP_ON_AC=power\n".to_string(),
        )];
        let known: Vec<String> = defaults().keys().cloned().collect();
        let renamed = parse_rename(RENAME);
        let (values, _, warnings) = merge_config(&files, &known, &renamed, &BTreeMap::new());

        assert!(
            values.contains_key("CPU_ENERGY_PERF_POLICY_ON_AC"),
            "the value lands on the current key"
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].replacement.as_deref(),
            Some("CPU_ENERGY_PERF_POLICY_ON_AC")
        );
    }

    #[test]
    fn typos_and_deprecated_keys_are_reported_with_their_location() {
        let files = vec![(
            TEMPLATE_CONF.to_string(),
            "TLP_WARN_LEVEL=1\nUSB_AUTOSUSPND=1\nRESTORE_DEVICE_STATE_ON_STARTUP=0\n".to_string(),
        )];
        let known: Vec<String> = defaults().keys().cloned().collect();
        let (_, _, warnings) = merge_config(
            &files,
            &known,
            &BTreeMap::new(),
            &parse_deprecated(DEPRECATED),
        );

        let typo = warnings
            .iter()
            .find(|w| w.key == "USB_AUTOSUSPND")
            .expect("typo reported");
        assert_eq!(typo.line, 2);
        assert!(typo.message.contains("not a parameter"));
        assert_eq!(typo.path, TEMPLATE_CONF);
        let deprecated = warnings
            .iter()
            .find(|w| w.key == "RESTORE_DEVICE_STATE_ON_STARTUP")
            .expect("deprecated reported");
        assert!(deprecated.message.contains("deprecated"));
    }

    #[test]
    fn config_parsing_ignores_comments_and_counts_lines() {
        let assignments = parse_config("# a comment\n\nTLP_ENABLE=1\n  USB_DENYLIST=\"x y\"  \n");
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].key, "TLP_ENABLE");
        assert_eq!(assignments[0].line, 3);
        assert_eq!(assignments[1].value, "x y");
        assert!(!assignments[1].append);
    }

    #[test]
    fn config_files_match_what_this_machine_has() {
        // Asserts the shape, not the host: empty when TLP is absent, and
        // otherwise defaults.conf first and tlp.conf last, which is the order
        // the merge depends on.
        let files = config_files();
        if files.is_empty() {
            assert!(!Path::new(DEFAULTS_CONF).exists());
        } else {
            assert_eq!(files[0], DEFAULTS_CONF);
            assert_eq!(files.last().map(String::as_str), Some(TEMPLATE_CONF));
        }
    }

    #[test]
    fn group_order_covers_every_group() {
        let mut labels: Vec<&str> = Group::ORDER.iter().map(|g| g.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), Group::ORDER.len());
    }
}
