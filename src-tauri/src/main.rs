// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--repair-pam" => {
                println!("Running ThinkUtils PAM Repair...");
                let result = thinkutils_lib::pam::execute_pam_repair();
                if result.success {
                    println!("SUCCESS: {}", result.message);
                    std::process::exit(0);
                } else {
                    eprintln!("FAILURE: {}", result.message);
                    for err in result.errors {
                        eprintln!("  {}", err);
                    }
                    std::process::exit(1);
                }
            }
            "--rollback-pam" => {
                println!("Rolling back ThinkUtils PAM Configuration...");
                let result = thinkutils_lib::pam::execute_pam_rollback();
                if result.success {
                    println!("SUCCESS: {}", result.message);
                    std::process::exit(0);
                } else {
                    eprintln!("FAILURE: {}", result.message);
                    for err in result.errors {
                        eprintln!("  {}", err);
                    }
                    std::process::exit(1);
                }
            }
            "--check-pam" => {
                let report = thinkutils_lib::pam::check_pam_health();
                println!("=== ThinkUtils PAM Health Check ===");
                println!(
                    "Status: {}",
                    if report.is_healthy {
                        "HEALTHY"
                    } else {
                        "ATTENTION NEEDED"
                    }
                );
                println!("Common auth has unix: {}", report.common_auth_has_unix);
                println!("GDM fingerprint clean: {}", report.gdm_fingerprint_clean);
                if !report.malformed_rules_found.is_empty() {
                    println!("Malformed rules found:");
                    for r in &report.malformed_rules_found {
                        println!("  - {}", r);
                    }
                }
                if !report.backups_available.is_empty() {
                    println!("Available backups in /var/backups:");
                    for b in &report.backups_available {
                        println!("  - {}", b);
                    }
                }
                std::process::exit(if report.is_healthy { 0 } else { 1 });
            }
            "--help" | "-h" => {
                println!("ThinkUtils - Linux ThinkPad Utilities");
                println!("Usage: thinkutils [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --check-pam     Check PAM authentication configuration health");
                println!(
                    "  --repair-pam    Safely sanitize, repair, and validate PAM configuration"
                );
                println!("  --rollback-pam  Rollback PAM configuration to the latest backup");
                println!("  --help, -h      Show this help message");
                println!();
                println!("Running without arguments launches the graphical interface.");
                std::process::exit(0);
            }
            _ => {}
        }
    }

    thinkutils_lib::run()
}
