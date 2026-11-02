//! Linux-PAM Configuration, Validation, Sanitization, and Safe Management.
//!
//! This module provides:
//! 1. Strict syntax validation for Linux-PAM configuration files according to pam.conf(5).
//! 2. Detection and rejection of invalid actions (such as `end`, which caused the Debian 13 login lockout).
//! 3. Safe, idempotent sanitization and repair of affected PAM configuration files.
//! 4. Automatic backup creation before any modification, with instant rollback capabilities.
//! 5. Preservation of standard distribution authentication stacks (`pam_unix.so`).

use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;

/// Standard historical PAM control flags.
pub const VALID_HISTORICAL_CONTROLS: &[&str] = &[
    "required",
    "requisite",
    "sufficient",
    "optional",
    "include",
    "substack",
];

/// Valid PAM return value tokens for extended control syntax `[value=action ...]`.
pub const VALID_PAM_VALUES: &[&str] = &[
    "success",
    "open_err",
    "symbol_err",
    "service_err",
    "system_err",
    "buf_err",
    "perm_denied",
    "auth_err",
    "cred_insufficient",
    "authinfo_unavail",
    "user_unknown",
    "maxtries",
    "new_authtok_reqd",
    "acct_expired",
    "session_err",
    "cred_unavail",
    "cred_expired",
    "cred_err",
    "no_module_data",
    "conv_err",
    "authtok_err",
    "authtok_recover_err",
    "authtok_lock_busy",
    "authtok_disable_aging",
    "try_again",
    "ignore",
    "abort",
    "authtok_expired",
    "module_unknown",
    "bad_item",
    "conv_again",
    "incomplete",
    "default",
];

/// Valid action keywords for extended control syntax.
/// Note: Linux-PAM actions are ONLY these keywords or an unsigned integer jump count ($N$).
/// `end` is explicitly NOT a valid Linux-PAM action!
pub const VALID_PAM_ACTIONS: &[&str] = &["ignore", "bad", "die", "ok", "done", "reset"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PamHealthReport {
    pub is_healthy: bool,
    pub malformed_rules_found: Vec<String>,
    pub common_auth_has_unix: bool,
    pub gdm_fingerprint_clean: bool,
    pub backups_available: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PamRepairResult {
    pub success: bool,
    pub message: String,
    pub cleaned_files: Vec<String>,
    pub backup_paths: Vec<String>,
    pub errors: Vec<String>,
}

/// Validates whether a PAM control specification string is valid per pam.conf(5).
pub fn is_valid_pam_control(control: &str) -> bool {
    let trimmed = control.trim();
    if trimmed.is_empty() {
        return false;
    }

    if VALID_HISTORICAL_CONTROLS.contains(&trimmed) {
        return true;
    }

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1].trim();
        if inner.is_empty() {
            return false;
        }

        for pair in inner.split_whitespace() {
            let parts: Vec<&str> = pair.split('=').collect();
            if parts.len() != 2 {
                return false;
            }
            let val = parts[0].trim();
            let act = parts[1].trim();

            if !VALID_PAM_VALUES.contains(&val) {
                return false;
            }

            let is_valid_action_keyword = VALID_PAM_ACTIONS.contains(&act);
            let is_jump_number = !act.is_empty() && act.chars().all(|c| c.is_ascii_digit());

            if !is_valid_action_keyword && !is_jump_number {
                // Reject invalid actions such as "end"
                return false;
            }
        }

        return true;
    }

    false
}

/// Validates a single non-comment line in a PAM file.
pub fn validate_pam_line(line: &str) -> Result<(), String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(());
    }

    // Directives like @include or include
    if trimmed.starts_with("@include") || trimmed.starts_with("include") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(format!("Malformed include directive: '{}'", trimmed));
        }
        return Ok(());
    }

    // Split tokens respecting square brackets in control
    let tokens = tokenize_pam_line(trimmed);
    if tokens.len() < 3 {
        return Err(format!(
            "PAM rule must have at least type, control, and module path: '{}'",
            trimmed
        ));
    }

    let mod_type = tokens[0].trim_start_matches('-');
    let valid_types = ["auth", "account", "session", "password"];
    if !valid_types.contains(&mod_type) {
        return Err(format!("Invalid PAM module type: '{}'", tokens[0]));
    }

    let control = &tokens[1];
    if !is_valid_pam_control(control) {
        return Err(format!(
            "Invalid PAM control syntax: '{}'. (Note: 'end' is not a valid action; expected jump number or keyword like 'done')",
            control
        ));
    }

    Ok(())
}

/// Helper to tokenize PAM line taking `[...]` into account as a single token.
pub fn tokenize_pam_line(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_bracket = false;

    for ch in line.chars() {
        match ch {
            '[' => {
                in_bracket = true;
                current.push(ch);
            }
            ']' => {
                in_bracket = false;
                current.push(ch);
            }
            ' ' | '\t' if !in_bracket => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Validates an entire PAM file content.
/// Returns Ok(()) if all active rules have valid PAM syntax and normal auth fallbacks exist.
pub fn validate_pam_content(content: &str, is_auth_stack: bool) -> Result<(), String> {
    let mut has_unix = false;

    for (line_no, line) in content.lines().enumerate() {
        if let Err(err) = validate_pam_line(line) {
            return Err(format!("Line {}: {}", line_no + 1, err));
        }

        if line.contains("pam_unix.so") {
            has_unix = true;
        }
    }

    if is_auth_stack && !has_unix {
        return Err(
            "Authentication stack must include pam_unix.so as fallback password provider"
                .to_string(),
        );
    }

    Ok(())
}

/// Sanitizes PAM file content by stripping malformed rules, particularly rules containing
/// `success=end`, invalid control syntax, or corrupted duplicates.
pub fn sanitize_pam_content(content: &str) -> (String, Vec<String>) {
    let mut sanitized_lines = Vec::new();
    let mut removed_lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect defective 'success=end' rules or invalid actions
        if trimmed.contains("success=end") || (trimmed.contains("howdy") && trimmed.contains("end"))
        {
            removed_lines.push(line.to_string());
            continue;
        }

        // Validate active rules
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Err(e) = validate_pam_line(trimmed) {
                removed_lines.push(format!("{} (Reason: {})", line, e));
                continue;
            }
        }

        sanitized_lines.push(line.to_string());
    }

    let mut result = sanitized_lines.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }

    (result, removed_lines)
}

/// Generates a bash script to safely repair all PAM configuration files,
/// create timestamped backups, clean up malformed `success=end` rules,
/// update Debian pam-auth-update cleanly, and restore `gdm-fingerprint` to stock.
pub fn build_pam_repair_bash_script() -> String {
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    format!(
        r#"#!/bin/bash
set -e

BACKUP_DIR="/var/backups/thinkutils_pam_{ts}"
mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"

echo "=== [1/5] Backing up current PAM configuration to $BACKUP_DIR ==="
for f in /etc/pam.d/common-* /etc/pam.d/gdm-* /etc/pam.d/sudo; do
  if [ -f "$f" ]; then
    cp -a "$f" "$BACKUP_DIR/"
  fi
done

echo "=== [2/5] Sanitizing corrupted 'success=end' rules from PAM files ==="
for f in /etc/pam.d/common-auth /etc/pam.d/gdm-fingerprint /etc/pam.d/gdm-password; do
  if [ -f "$f" ]; then
    # Strip any lines containing malformed success=end or TEMP-DISABLED howdy
    sed -i '/success=end/d' "$f" 2>/dev/null || true
    sed -i '/TEMP-DISABLED.*howdy/d' "$f" 2>/dev/null || true
    sed -i '/howdy\/pam\.py/d' "$f" 2>/dev/null || true
  fi
done

echo "=== [3/5] Restoring clean gdm-fingerprint stack ==="
if [ -f /etc/pam.d/gdm-fingerprint ]; then
  # Ensure gdm-fingerprint contains only the stock fprintd rules
  cat << 'EOF' > /etc/pam.d/gdm-fingerprint
#%PAM-1.0
auth    requisite       pam_nologin.so
auth	required	pam_succeed_if.so user != root quiet_success
auth	required	pam_fprintd.so
auth    optional        pam_gnome_keyring.so
@include common-account
session [success=ok ignore=ignore module_unknown=ignore default=bad]        pam_selinux.so close
session required        pam_loginuid.so
session [success=ok ignore=ignore module_unknown=ignore default=bad]        pam_selinux.so open
session optional        pam_keyinit.so force revoke
session required        pam_limits.so
session required        pam_env.so readenv=1
session required        pam_env.so readenv=1 envfile=/etc/default/locale
@include common-session
session optional        pam_gnome_keyring.so auto_start
password required       pam_fprintd.so
EOF
  chmod 644 /etc/pam.d/gdm-fingerprint
fi

echo "=== [4/5] Ensuring Howdy and face recognition rules are completely purged ==="
for f in /etc/pam.d/common-* /etc/pam.d/gdm-* /etc/pam.d/sudo; do
  if [ -f "$f" ]; then
    sed -i '/howdy/d' "$f" 2>/dev/null || true
    sed -i '/pam_python.*pam\.py/d' "$f" 2>/dev/null || true
  fi
done
rm -rf /lib/security/howdy /etc/howdy /usr/share/pam-configs/howdy 2>/dev/null || true

echo "=== [5/5] Re-synchronizing Debian PAM configuration via pam-auth-update ==="
if [ -x /usr/sbin/pam-auth-update ]; then
  # On Debian, pam-auth-update cleanly regenerates /etc/pam.d/common-auth with valid numeric jump counts
  DEBIAN_FRONTEND=noninteractive /usr/sbin/pam-auth-update --package --force 2>/dev/null || true
fi

# Validation: ensure pam_unix.so is present and no success=end exists
if grep -q 'success=end' /etc/pam.d/common-auth 2>/dev/null; then
  echo "ERROR: Corrupted rule still found in /etc/pam.d/common-auth! Reverting..."
  cp -a "$BACKUP_DIR/common-auth" /etc/pam.d/common-auth
  exit 1
fi

if ! grep -q 'pam_unix\.so' /etc/pam.d/common-auth 2>/dev/null; then
  echo "ERROR: pam_unix.so missing from common-auth! Reverting..."
  cp -a "$BACKUP_DIR/common-auth" /etc/pam.d/common-auth
  exit 1
fi

echo "PAM_REPAIR_COMPLETE"
"#,
        ts = ts
    )
}

/// Generates a bash script to rollback PAM configuration to the most recent backup.
pub fn build_pam_rollback_bash_script() -> String {
    r#"#!/bin/bash
set -e

LATEST_BACKUP=$(ls -td /var/backups/thinkutils_pam_* 2>/dev/null | head -n 1)
if [ -z "$LATEST_BACKUP" ] || [ ! -d "$LATEST_BACKUP" ]; then
  echo "ERROR: No ThinkUtils PAM backup found in /var/backups/"
  exit 1
fi

echo "Restoring PAM configuration from $LATEST_BACKUP..."
for f in "$LATEST_BACKUP"/*; do
  if [ -f "$f" ]; then
    fname=$(basename "$f")
    cp -a "$f" "/etc/pam.d/$fname"
    echo "Restored /etc/pam.d/$fname"
  fi
done

# Strip any residual malformed success=end
for f in /etc/pam.d/common-auth /etc/pam.d/gdm-fingerprint; do
  if [ -f "$f" ]; then
    sed -i '/success=end/d' "$f" 2>/dev/null || true
  fi
done

echo "PAM_ROLLBACK_COMPLETE"
"#
    .to_string()
}

/// Checks the health of PAM authentication files on the host system.
#[tauri::command]
pub fn check_pam_health() -> PamHealthReport {
    let mut malformed_rules = Vec::new();
    let mut common_auth_has_unix = false;
    let mut gdm_fingerprint_clean = true;

    if let Ok(content) = fs::read_to_string("/etc/pam.d/common-auth") {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.contains("success=end") {
                malformed_rules.push(format!("/etc/pam.d/common-auth: {}", trimmed));
            }
            if trimmed.contains("pam_unix.so") {
                common_auth_has_unix = true;
            }
            if trimmed.contains("howdy/pam.py") || trimmed.contains("pam_howdy.so") {
                malformed_rules.push(format!(
                    "/etc/pam.d/common-auth: residual face rule '{}'",
                    trimmed
                ));
            }
        }
    }

    if let Ok(content) = fs::read_to_string("/etc/pam.d/gdm-fingerprint") {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.contains("success=end")
                || (trimmed.contains("howdy") && !trimmed.starts_with('#'))
            {
                malformed_rules.push(format!("/etc/pam.d/gdm-fingerprint: {}", trimmed));
                gdm_fingerprint_clean = false;
            }
        }
    }

    let mut backups = Vec::new();
    if let Ok(entries) = fs::read_dir("/var/backups") {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with("thinkutils_pam_") {
                backups.push(fname);
            }
        }
    }
    backups.sort();
    backups.reverse();

    let is_healthy = malformed_rules.is_empty() && common_auth_has_unix && gdm_fingerprint_clean;

    PamHealthReport {
        is_healthy,
        malformed_rules_found: malformed_rules,
        common_auth_has_unix,
        gdm_fingerprint_clean,
        backups_available: backups,
    }
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "0")
        .unwrap_or(false)
}

/// Executes PAM repair via pkexec (or direct bash if running as root in CLI/recovery).
#[tauri::command]
pub fn execute_pam_repair() -> PamRepairResult {
    let script = build_pam_repair_bash_script();
    let output = if is_root() {
        Command::new("bash").args(["-c", &script]).output()
    } else {
        Command::new("pkexec")
            .args(["bash", "-c", &script])
            .output()
    };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let success = out.status.success() && stdout.contains("PAM_REPAIR_COMPLETE");

            PamRepairResult {
                success,
                message: if success {
                    "PAM configuration repaired successfully. Standard password authentication and clean biometric stacks verified.".to_string()
                } else {
                    format!("PAM repair failed: {}", stderr.trim())
                },
                cleaned_files: vec![
                    "/etc/pam.d/common-auth".to_string(),
                    "/etc/pam.d/gdm-fingerprint".to_string(),
                ],
                backup_paths: vec!["/var/backups/thinkutils_pam_*".to_string()],
                errors: if success { vec![] } else { vec![stderr] },
            }
        }
        Err(e) => PamRepairResult {
            success: false,
            message: format!("Failed to launch execution for PAM repair: {}", e),
            cleaned_files: vec![],
            backup_paths: vec![],
            errors: vec![e.to_string()],
        },
    }
}

/// Executes PAM rollback via pkexec (or direct bash if running as root in CLI/recovery).
#[tauri::command]
pub fn execute_pam_rollback() -> PamRepairResult {
    let script = build_pam_rollback_bash_script();
    let output = if is_root() {
        Command::new("bash").args(["-c", &script]).output()
    } else {
        Command::new("pkexec")
            .args(["bash", "-c", &script])
            .output()
    };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let success = out.status.success() && stdout.contains("PAM_ROLLBACK_COMPLETE");

            PamRepairResult {
                success,
                message: if success {
                    "PAM configuration rolled back successfully to previous backup.".to_string()
                } else {
                    format!("PAM rollback failed: {}", stderr.trim())
                },
                cleaned_files: vec!["/etc/pam.d/common-auth".to_string()],
                backup_paths: vec![],
                errors: if success { vec![] } else { vec![stderr] },
            }
        }
        Err(e) => PamRepairResult {
            success: false,
            message: format!("Failed to launch execution for PAM rollback: {}", e),
            cleaned_files: vec![],
            backup_paths: vec![],
            errors: vec![e.to_string()],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validates_standard_historical_controls() {
        assert!(is_valid_pam_control("required"));
        assert!(is_valid_pam_control("requisite"));
        assert!(is_valid_pam_control("sufficient"));
        assert!(is_valid_pam_control("optional"));
        assert!(is_valid_pam_control("include"));
        assert!(is_valid_pam_control("substack"));
        assert!(!is_valid_pam_control("invalid_control"));
    }

    #[test]
    fn test_validates_extended_controls_with_valid_actions() {
        // Valid jump numbers
        assert!(is_valid_pam_control("[success=1 default=ignore]"));
        assert!(is_valid_pam_control("[success=2 default=ignore]"));
        assert!(is_valid_pam_control("[success=3 default=ignore]"));

        // Valid keywords
        assert!(is_valid_pam_control("[success=ok default=ignore]"));
        assert!(is_valid_pam_control("[success=done default=ignore]"));
        assert!(is_valid_pam_control(
            "[success=done new_authtok_reqd=done default=ignore]"
        ));
        assert!(is_valid_pam_control(
            "[success=ok ignore=ignore module_unknown=ignore default=bad]"
        ));
    }

    #[test]
    fn test_strictly_rejects_malformed_success_end() {
        // 'end' is not a valid PAM action keyword or jump number
        assert!(!is_valid_pam_control("[success=end default=ignore]"));
        assert!(!is_valid_pam_control("[success=end]"));
        assert!(!is_valid_pam_control("[default=end]"));
        assert!(!is_valid_pam_control("[success=foo default=ignore]"));
    }

    #[test]
    fn test_validates_full_pam_lines() {
        assert!(validate_pam_line("auth required pam_unix.so nullok").is_ok());
        assert!(validate_pam_line("auth [success=1 default=ignore] pam_unix.so nullok").is_ok());
        assert!(validate_pam_line(
            "auth [success=2 default=ignore] pam_python.so /lib/security/howdy/pam.py"
        )
        .is_ok());
        assert!(
            validate_pam_line("auth sufficient pam_python.so /lib/security/howdy/pam.py").is_ok()
        );
        assert!(validate_pam_line("@include common-auth").is_ok());
        assert!(validate_pam_line("# comment line").is_ok());
        assert!(validate_pam_line("   ").is_ok());

        // Malformed lines must fail
        assert!(validate_pam_line(
            "auth [success=end default=ignore] pam_python.so /lib/security/howdy/pam.py"
        )
        .is_err());
        assert!(validate_pam_line("bad_type required pam_unix.so").is_err());
    }

    #[test]
    fn test_sanitizes_malformed_success_end_content() {
        let input = "\
# here are the per-package modules (the \"Primary\" block)
auth	[success=end default=ignore]	pam_python.so /lib/security/howdy/pam.py
auth	[success=1 default=ignore]	pam_unix.so nullok
# here's the fallback if no module succeeds
auth	requisite			pam_deny.so
auth	required			pam_permit.so
";

        let (sanitized, removed) = sanitize_pam_content(input);
        assert_eq!(removed.len(), 1);
        assert!(removed[0].contains("success=end"));
        assert!(!sanitized.contains("success=end"));
        assert!(sanitized.contains("pam_unix.so nullok"));
        assert!(sanitized.contains("pam_deny.so"));
        assert!(sanitized.contains("pam_permit.so"));

        // Sanitized content must pass validation
        assert!(validate_pam_content(&sanitized, true).is_ok());
    }

    #[test]
    fn test_auth_stack_requires_pam_unix() {
        let no_unix = "\
auth required pam_deny.so
auth required pam_permit.so
";
        assert!(validate_pam_content(no_unix, true).is_err());
    }

    #[test]
    fn test_sanitization_is_idempotent() {
        let input = "\
# Clean stack
auth	[success=1 default=ignore]	pam_unix.so nullok
auth	requisite			pam_deny.so
auth	required			pam_permit.so
";
        let (first_pass, removed1) = sanitize_pam_content(input);
        assert_eq!(removed1.len(), 0);
        assert_eq!(first_pass, input);

        let (second_pass, removed2) = sanitize_pam_content(&first_pass);
        assert_eq!(removed2.len(), 0);
        assert_eq!(second_pass, input);
    }

    #[test]
    fn test_repair_script_contains_critical_safety_guards() {
        let script = build_pam_repair_bash_script();
        // Must back up first
        assert!(script.contains("/var/backups/thinkutils_pam_"));
        // Must sanitize success=end
        assert!(script.contains("sed -i '/success=end/d'"));
        // Must ensure pam_unix is retained
        assert!(script.contains("grep -q 'pam_unix\\.so'"));
        // Must revert if validation fails
        assert!(script.contains("cp -a \"$BACKUP_DIR/common-auth\" /etc/pam.d/common-auth"));
        // Must never inject success=end
        assert!(!script.contains("success=end default=ignore"));
    }

    #[test]
    fn test_rollback_script_locates_latest_backup() {
        let script = build_pam_rollback_bash_script();
        assert!(script.contains("ls -td /var/backups/thinkutils_pam_*"));
        assert!(script.contains("cp -a \"$f\" \"/etc/pam.d/$fname\""));
        assert!(script.contains("PAM_ROLLBACK_COMPLETE"));
    }
}
