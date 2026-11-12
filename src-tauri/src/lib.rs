mod auth;
mod autostart;
mod battery;
pub mod biometrics;
pub mod environment;
pub mod fan_control;
mod fan_curve;
pub mod gnome_extension;
pub mod hardware_root;
mod mcp;
mod monitor;
pub mod overview;
pub mod pam;
mod performance;
mod permissions;
pub mod power;
mod privileged;
mod security;
mod settings;
mod system_info;
mod terminal;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

#[tauri::command]
async fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("Failed to open URL: {}", e))
}

#[tauri::command]
async fn minimize_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window
            .minimize()
            .map_err(|e| format!("Failed to minimize: {}", e))
    } else {
        Err("Window not found".to_string())
    }
}

#[tauri::command]
async fn toggle_maximize(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let is_maximized = window
            .is_maximized()
            .map_err(|e| format!("Failed to check maximize state: {}", e))?;
        if is_maximized {
            window
                .unmaximize()
                .map_err(|e| format!("Failed to unmaximize: {}", e))
        } else {
            window
                .maximize()
                .map_err(|e| format!("Failed to maximize: {}", e))
        }
    } else {
        Err("Window not found".to_string())
    }
}

#[tauri::command]
async fn close_window(app: AppHandle) -> Result<(), String> {
    // Hide window instead of closing to keep app running in background
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| format!("Failed to hide: {}", e))
    } else {
        Err("Window not found".to_string())
    }
}

#[tauri::command]
async fn start_drag(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window
            .start_dragging()
            .map_err(|e| format!("Failed to start drag: {}", e))
    } else {
        Err("Window not found".to_string())
    }
}

/// Called by the frontend once every template is injected and every view has
/// initialised.
///
/// Its absence in a log means the JS never finished booting, which no pixel
/// check can prove on its own — a window can be fully painted by a frontend
/// that died halfway through init.
///
/// `failed` names the view setups that threw. Each is now isolated, so reaching
/// this point no longer implies they all succeeded — a boot that wires eight
/// views out of nine still gets here, and would otherwise look identical to a
/// clean one.
#[tauri::command]
fn report_frontend_ready(templates: usize, views: usize, failed: Vec<String>) {
    println!("[thinkpro-vantage] frontend ready: templates={templates} views={views}");
    if !failed.is_empty() {
        // Deliberately loud: the launch test greps this output, and a partially
        // wired app is the failure mode most likely to pass a screenshot check.
        eprintln!(
            "[thinkpro-vantage] frontend init had {} failing step(s): {}",
            failed.len(),
            failed.join("; ")
        );
    }
}

/// Any uncaught frontend exception or rejection.
///
/// This is the check that catches a view dying on an absent sysfs path: the
/// sidebar still paints, the process still lives, and without this line nothing
/// would ever notice.
#[tauri::command]
fn report_frontend_error(msg: String) {
    // Truncated because an unhandled rejection can carry an entire stack.
    let msg: String = msg.chars().take(500).collect();
    eprintln!("[thinkpro-vantage] frontend error: {msg}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(fan_idx) = args.iter().position(|a| a == "--fan" || a == "--set-fan") {
                if let Some(target_speed) = args.get(fan_idx + 1) {
                    let speed = target_speed.clone();
                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::fan_control::apply_fan_speed(&app_handle, &speed).await;
                    });
                    return;
                }
            }

            // When a second instance is launched, focus the existing window
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.unminimize();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            // --- Diagnostic instrumentation ---
            //
            // These lines are the contract scripts/test-gui-packages-docker.sh
            // asserts against. In a headless container they are the only evidence
            // that the app started correctly, so do not remove or reword them
            // without updating that script. Format: "[thinkpro-vantage] <key>: <value>".
            if let Some(w) = app.get_webview_window("main") {
                match w.url() {
                    // An http(s) URL here means the build points at devUrl, which
                    // renders an empty window on any machine without a dev server.
                    Ok(url) => println!("[thinkpro-vantage] webview url: {url}"),
                    Err(e) => {
                        eprintln!("[thinkpro-vantage] warning: could not read webview url: {e}")
                    }
                }
            }

            // Probe the hardware surfaces this app depends on, and say so. In a
            // container every one is absent, and that is a legitimate supported
            // state -- not a failure. Printing it is what lets a test tell
            // "started fine, no ThinkPad here" from "broken", which are otherwise
            // indistinguishable from the outside.
            let present = |p: &str| {
                if std::path::Path::new(p).exists() {
                    "present"
                } else {
                    "absent"
                }
            };
            let ibm_fan = present("/proc/acpi/ibm/fan");
            let bat0 = present("/sys/class/power_supply/BAT0");
            let cpufreq = present("/sys/devices/system/cpu/cpu0/cpufreq");
            println!(
                "[thinkpro-vantage] hw probe: ibm_fan={ibm_fan} bat0={bat0} cpufreq={cpufreq}"
            );

            // Only the thinkpad_acpi fan interface distinguishes a supported
            // machine. A battery and cpufreq exist on every Linux laptop, and a
            // container inherits the host's /sys -- so keying the mode off those
            // reported full ThinkPad support from inside a container that had no
            // fan interface at all. The first run of the launch test caught it.
            println!(
                "[thinkpro-vantage] hw mode: {}",
                if ibm_fan == "present" {
                    "full"
                } else {
                    "degraded"
                }
            );

            // Load fan curve config from persistent storage
            let saved_config = fan_curve::load_config_from_store(app.handle());
            let initial_mode = if saved_config.enabled {
                fan_curve::FanTargetMode::Curve
            } else {
                fan_curve::FanTargetMode::Auto
            };
            let fan_curve_state = fan_curve::FanCurveState::new(std::sync::Mutex::new(
                fan_curve::FanSupervisorState {
                    mode: initial_mode,
                    curve_config: saved_config,
                },
            ));
            app.manage(fan_curve_state);

            // Handle standalone CLI fan speed argument if started headlessly
            let cli_args: Vec<String> = std::env::args().collect();
            if let Some(fan_idx) = cli_args
                .iter()
                .position(|a| a == "--fan" || a == "--set-fan")
            {
                if let Some(target_speed) = cli_args.get(fan_idx + 1) {
                    let speed = target_speed.clone();
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::block_on(async move {
                        let _ = crate::fan_control::apply_fan_speed(&app_handle, &speed).await;
                    });
                    std::process::exit(0);
                }
            }

            // Initialize MCP server state (off by default)
            let mcp_state =
                mcp::McpState::new(tokio::sync::Mutex::new(mcp::McpServerState::default()));
            app.manage(mcp_state);

            // Start fan curve background task
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                fan_curve::fan_curve_background_task(app_handle).await;
            });

            // Follow the active power profile, which TLP changes on its own when
            // the power cable moves — see `performance::power_profile_watcher`.
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                performance::power_profile_watcher(app_handle).await;
            });

            // Hold the saved GPU clock ceiling. lactd re-applies its own level
            // from its config whenever it starts, so a ceiling written to sysfs
            // has to be written again — see `performance::gpu_clock_watcher`.
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                performance::gpu_clock_watcher(app_handle).await;
            });
            // Create tray menu
            let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide Window", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

            // Load tray icon - dedicated tray artwork, distinct from the app icon
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))
                .expect("bundled tray icon should decode");

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .icon(tray_icon)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // Prevent window from closing, hide it instead
            if let Some(window) = app.get_webview_window("main") {
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // Prevent the window from closing
                        api.prevent_close();
                        // Hide the window instead
                        let _ = window_clone.hide();
                    }
                });
            }

            // The window is configured invisible so a login launch never paints
            // and then disappears. A manual launch shows it here; an autostart
            // launch stays in the tray, which is the only way "start at login"
            // does not become "a window in your face at login". When
            // minimize_to_tray is off the operator has asked for the window
            // itself, so a login launch shows it too.
            let launched_at_login = autostart::launched_from_autostart();
            let start_hidden = launched_at_login
                && settings::load_raw_app_settings(app.handle()).minimize_to_tray;
            if let Some(window) = app.get_webview_window("main") {
                if !start_hidden {
                    let _ = window.show();
                }
            }
            println!(
                "[thinkpro-vantage] window start: launched_at_login={launched_at_login} hidden={start_hidden}"
            );

            // Reconcile the autostart entry with the saved setting before
            // anything else can read either. A fresh install has no store, so
            // auto_start defaults to true and this creates the entry on the
            // very first run; afterwards it restores a hand-deleted entry and
            // removes one left behind by a version that no longer wanted it.
            autostart::reconcile(app.handle());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Utilities
            open_url,
            minimize_window,
            toggle_maximize,
            close_window,
            start_drag,
            // Authentication
            auth::authenticate_once,
            // Permissions
            permissions::check_permissions_status,
            permissions::setup_permissions,
            // Power ownership
            power::arbiter::power_owners,
            // TLP (reads the installed package; gated by the arbiter)
            power::tlp::tlp_schema,
            power::tlp::tlp_config,
            // System overview
            overview::get_system_overview,
            overview::get_device_serial,
            overview::get_memory_speed,
            overview::get_storage_health,
            overview::get_package_watts,
            power::doctor::power_doctor,
            power::writer::power_preview_changes,
            power::writer::power_apply_changes,
            power::writer::power_rollback_dropin,
            terminal::open_terminal_with,
            // Autostart at login
            autostart::get_autostart_status,
            autostart::set_autostart,
            // Settings & Import/Export
            settings::save_app_settings,
            settings::load_app_settings,
            settings::update_setting,
            settings::export_settings,
            settings::import_settings,
            settings::reset_settings,
            // Fan Control
            fan_control::get_sensor_data,
            fan_control::get_fan_capability,
            environment::get_system_report,
            report_frontend_ready,
            report_frontend_error,
            fan_control::enable_fan_control,
            fan_control::set_fan_speed,
            fan_control::check_permissions,
            // Fan Curve
            fan_curve::set_fan_curve,
            fan_curve::get_fan_curve,
            fan_curve::enable_fan_curve,
            // System Info
            system_info::get_system_info,
            // Battery
            battery::get_battery_info,
            battery::get_battery_thresholds,
            battery::set_battery_thresholds,
            battery::get_power_consumption,
            // Performance
            performance::get_cpu_info,
            performance::set_cpu_governor,
            performance::get_power_profile,
            performance::set_power_profile,
            performance::get_turbo_boost_status,
            performance::set_turbo_boost,
            // Performance — clock limits and the GPU clock level
            performance::get_clock_limits,
            performance::set_gpu_clock_level,
            performance::clear_gpu_clock_cap,
            performance::reattach_gpu,
            // Monitor
            monitor::get_system_monitor,
            // Security
            security::get_security_status,
            security::update_virus_definitions,
            security::scan_path,
            security::quick_scan,
            security::install_clamav,
            // MCP
            mcp::get_mcp_status,
            mcp::start_mcp_server,
            mcp::stop_mcp_server,
            // GNOME Top Panel Extension
            gnome_extension::get_gnome_extension_status,
            gnome_extension::enable_gnome_extension,
            gnome_extension::install_gnome_extension,
            gnome_extension::uninstall_gnome_extension,
            // Biometrics & Fingerprint
            biometrics::get_biometric_overview,
            biometrics::get_fingerprint_status,
            biometrics::enroll_fingerprint,
            biometrics::cancel_fingerprint_action,
            biometrics::verify_fingerprint,
            biometrics::delete_fingerprint,
            biometrics::install_fingerprint_packages,
            biometrics::configure_biometric_pam,
            biometrics::open_keyring_manager,
            // PAM Safe Configuration & Recovery
            pam::check_pam_health,
            pam::execute_pam_repair,
            pam::execute_pam_rollback,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            // Never exit leaving the fan under manual control. The firmware
            // watchdog is the backstop for a hard kill, but on a clean exit we
            // hand the fan back explicitly and immediately.
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                fan_curve::restore_fan_to_auto_blocking();
            }
        });
}
