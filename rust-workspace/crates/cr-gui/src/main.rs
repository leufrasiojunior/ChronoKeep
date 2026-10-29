#![windows_subsystem = "windows"]

mod commands;
mod installer;
mod oauth_flow;
mod orphans;
mod service_mgr;
mod win32;

use commands::*;
use installer::run_silent_install;
use std::env;
use tauri::Manager;

fn main() {
    let args: Vec<String> = env::args().collect();
    for arg in &args[1..] {
        let a = arg.to_lowercase();
        if a == "/s"
            || a == "-s"
            || a == "/silent"
            || a == "--silent"
            || a == "/quiet"
            || a == "-q"
            || a == "/install"
        {
            let exit_code = run_silent_install();
            std::process::exit(exit_code);
        }
    }

    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window_vibrancy::apply_mica(&window, None);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_system_info,
            get_config,
            save_config,
            get_service_status,
            start_service,
            stop_service,
            install_chronokeep,
            start_google_login,
            disconnect_google,
            scan_saves,
            upload_game_save,
            get_logs,
            clear_log,
            open_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChronoKeep GUI");
}
