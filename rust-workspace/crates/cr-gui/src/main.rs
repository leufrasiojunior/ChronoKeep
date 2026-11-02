#![windows_subsystem = "windows"]

mod commands;
pub mod embedded;
mod i18n;
mod installer;
mod oauth_flow;
mod orphans;
mod service_mgr;
mod updater;
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

    // Limpa eventuais binarios temporarios de atualizacoes anteriores (.new.exe)
    if let Ok(cur_exe) = std::env::current_exe() {
        let new_exe = cur_exe.with_extension("new.exe");
        if new_exe.exists() {
            let _ = std::fs::remove_file(&new_exe);
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
            set_language,
            get_service_status,
            start_service,
            stop_service,
            check_system_dependencies,
            install_chronokeep,
            start_google_login,
            disconnect_google,
            scan_saves,
            upload_game_save,
            get_logs,
            clear_log,
            set_debug_mode,
            open_directory,
            open_url,
            is_steam_running,
            launch_steam,
            check_for_updates,
            update_component,
            get_launch_mode,
            uninstall_chronokeep,
            get_games_catalog,
            get_game_achievements,
            get_steam_api_key,
            set_steam_api_key,
            open_steam_store,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChronoKeep GUI");
}
