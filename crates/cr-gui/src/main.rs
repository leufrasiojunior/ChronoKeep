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
use std::path::Path;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut start_minimized = false;
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
        } else if a == "--minimized" || a == "-m" || a == "--tray" {
            start_minimized = true;
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
        .setup(move |app| {
            #[cfg(target_os = "windows")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window_vibrancy::apply_mica(&window, None);
                    if start_minimized {
                        let _ = window.hide();
                    }
                }
            }

            // Configura o ícone na Bandeja do Sistema (System Tray)
            let s_dir_init = load_preferred_steam_dir();
            let lang = crate::i18n::current_lang(Some(Path::new(&s_dir_init)));
            let is_pt = lang.is_pt();

            let open_label = if is_pt {
                "Abrir ChronoKeep"
            } else {
                "Open ChronoKeep"
            };
            let heal_label = if is_pt {
                "Verificar / Restaurar Proteção Steam"
            } else {
                "Verify / Restore Steam Protection"
            };
            let quit_label = if is_pt {
                "Sair Completamente"
            } else {
                "Quit Completely"
            };

            if let (Ok(open_i), Ok(heal_i), Ok(quit_i)) = (
                MenuItem::with_id(app, "open", open_label, true, None::<&str>),
                MenuItem::with_id(app, "heal", heal_label, true, None::<&str>),
                MenuItem::with_id(app, "quit", quit_label, true, None::<&str>),
            ) {
                if let Ok(menu) = Menu::with_items(app, &[&open_i, &heal_i, &quit_i]) {
                    let mut tray_builder = TrayIconBuilder::new()
                        .tooltip("ChronoKeep — Steam Cloud Guardian")
                        .menu(&menu)
                        .show_menu_on_left_click(false)
                        .on_menu_event(|app_handle, event| match event.id.as_ref() {
                            "open" => {
                                if let Some(window) = app_handle.get_webview_window("main") {
                                    let _ = window.show();
                                    let _ = window.unminimize();
                                    let _ = window.set_focus();
                                }
                            }
                            "heal" => {
                                let s_dir = load_preferred_steam_dir();
                                let s_path = Path::new(&s_dir);
                                let lang = crate::i18n::current_lang(Some(s_path));
                                let (th, dh) = cr_common::config::auto_heal_steam_dir(s_path);
                                let title = if lang.is_pt() {
                                    "ChronoKeep — Proteção Steam Cloud"
                                } else {
                                    "ChronoKeep — Steam Cloud Protection"
                                };
                                let msg = if th || dh {
                                    if lang.is_pt() {
                                        "A configuração do OpenSteamTool e a DLL oficial foram restauradas com sucesso!"
                                    } else {
                                        "OpenSteamTool configuration and official DLL were successfully restored!"
                                    }
                                } else if lang.is_pt() {
                                    "Proteção verificada: chronokeep.dll e opensteamtool.toml estão íntegros e ativos."
                                } else {
                                    "Protection verified: chronokeep.dll and opensteamtool.toml are intact and active."
                                };
                                cr_common::notify::show_balloon_notification(title, msg, false);
                            }
                            "quit" => {
                                app_handle.exit(0);
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
                                let app_handle = tray.app_handle();
                                if let Some(window) = app_handle.get_webview_window("main") {
                                    let _ = window.show();
                                    let _ = window.unminimize();
                                    let _ = window.set_focus();
                                }
                            }
                        });

                    if let Some(icon) = app.default_window_icon() {
                        tray_builder = tray_builder.icon(icon.clone());
                    }
                    let _ = tray_builder.build(app);
                }
            }

            // Thread Guardião em segundo plano (monitora e auto-cura opensteamtool.toml e cloud_redirect.dll)
            std::thread::spawn(move || {
                let mut legacy_locked_notified = false;
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let s_dir = load_preferred_steam_dir();
                    let s_path = Path::new(&s_dir);
                    if !s_path.exists() {
                        continue;
                    }
                    if !cr_common::config::load_guardian_enabled(Some(s_path)) {
                        continue;
                    }

                    let (toml_healed, dll_healed) = cr_common::config::auto_heal_steam_dir(s_path);
                    let lang = crate::i18n::current_lang(Some(s_path));
                    let is_pt = lang.is_pt();

                    if toml_healed || dll_healed {
                        legacy_locked_notified = false;
                        if cr_common::config::load_notifications_enabled(Some(s_path)) {
                            let title = if is_pt {
                                "ChronoKeep — Guardião Steam Cloud"
                            } else {
                                "ChronoKeep — Steam Cloud Guardian"
                            };
                            let msg = if is_pt {
                                "Alteração externa (ex: LuaTools) detectada e corrigida automaticamente na pasta da Steam."
                            } else {
                                "External change (e.g. LuaTools) detected and automatically healed in the Steam folder."
                            };
                            cr_common::notify::show_balloon_notification(title, msg, false);
                        }
                    }

                    // Se chronokeep.dll existe mas cloud_redirect.dll continua sendo a legada mesmo após auto_heal_steam_dir,
                    // significa que a Steam está rodando com a DLL legada bloqueada em memória.
                    let ck_dll = s_path.join("chronokeep.dll");
                    let redir_dll = s_path.join("cloud_redirect.dll");
                    if ck_dll.exists()
                        && redir_dll.exists()
                        && !cr_common::config::is_chronokeep_bridge(&redir_dll)
                    {
                        if !legacy_locked_notified {
                            legacy_locked_notified = true;
                            let title = if is_pt {
                                "ChronoKeep — Reinicie a Steam"
                            } else {
                                "ChronoKeep — Restart Steam"
                            };
                            let msg = if is_pt {
                                "A DLL legada está bloqueada em memória pela Steam! O opensteamtool.toml já foi restaurado — reinicie a Steam para reativar o ChronoKeep."
                            } else {
                                "The legacy DLL is locked in memory by Steam! opensteamtool.toml has been restored — restart Steam to re-enable ChronoKeep."
                            };
                            cr_common::notify::show_balloon_notification(title, msg, true);
                        }
                    } else {
                        legacy_locked_notified = false;
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let is_uninstall = get_launch_mode() == "uninstall";
                let s_dir = load_preferred_steam_dir();
                if !is_uninstall
                    && cr_common::config::load_guardian_enabled(Some(Path::new(&s_dir)))
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
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
            enable_opensteamtool_cloud,
            start_google_login,
            disconnect_google,
            scan_saves,
            upload_game_save,
            sync_game_stats,
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
            repair_bridge,
            get_games_catalog,
            get_game_achievements,
            get_steam_api_key,
            set_steam_api_key,
            test_steam_api_key,
            resolve_game_cover_url,
            open_steam_store,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ChronoKeep GUI");
}
