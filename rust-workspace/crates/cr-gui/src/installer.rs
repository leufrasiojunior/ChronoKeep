//! Installation, deployment of embedded binaries, and configuration persistence.

use crate::service_mgr::{check_service_online, is_process_running, kill_process_and_wait, launch_steam, start_service_from_gui};
use crate::win32::*;
use cr_common::config::{detect_default_steam_dir, detect_default_vault_dir, find_companion_binary, is_gdrive_connected, resolve_service_port, DEFAULT_SERVICE_PORT};
use cr_common::http::to_wide;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub fn deploy_bytes_safe(data: &[u8], dst: &Path) -> bool {
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::write(dst, data).is_ok() {
        return true;
    }
    let old_path = dst.with_extension("old");
    let _ = fs::remove_file(&old_path);
    let _ = fs::rename(dst, &old_path);
    fs::write(dst, data).is_ok()
}

pub fn deploy_component(target: &Path, companion_name: &str, embedded: &[u8]) -> bool {
    let bytes = if let Some(local_path) = find_companion_binary(companion_name) {
        fs::read(local_path).unwrap_or_else(|_| embedded.to_vec())
    } else {
        embedded.to_vec()
    };
    deploy_bytes_safe(&bytes, target)
}

pub fn ensure_opensteamtool_cloud_enabled(steam_dir: &Path) {
    let toml_path = steam_dir.join("opensteamtool.toml");
    if !toml_path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(&toml_path) else {
        return;
    };

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut in_cloud_section = false;
    let mut cloud_found = false;
    let mut enabled_found = false;

    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_cloud_section = trimmed.eq_ignore_ascii_case("[cloud]");
            if in_cloud_section {
                cloud_found = true;
            }
        } else if in_cloud_section && trimmed.to_ascii_lowercase().starts_with("enabled") {
            *line = "enabled = true".to_string();
            enabled_found = true;
        }
    }

    if !cloud_found {
        lines.push(String::new());
        lines.push("[cloud]".to_string());
        lines.push("enabled = true".to_string());
    } else if !enabled_found {
        if let Some(pos) = lines.iter().position(|l| l.trim().eq_ignore_ascii_case("[cloud]")) {
            lines.insert(pos + 1, "enabled = true".to_string());
        }
    }

    let updated = lines.join("\r\n") + "\r\n";
    let _ = fs::write(&toml_path, updated);
}

pub fn is_opensteamtool_cloud_disabled(steam_dir: &Path) -> bool {
    let toml_path = steam_dir.join("opensteamtool.toml");
    if !toml_path.exists() {
        return false;
    }
    let Ok(content) = fs::read_to_string(&toml_path) else {
        return false;
    };
    let mut in_cloud = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_cloud = t.eq_ignore_ascii_case("[cloud]");
        } else if in_cloud && t.to_ascii_lowercase().starts_with("enabled") {
            if let Some((_, v)) = t.split_once('=') {
                return !v.trim().eq_ignore_ascii_case("true");
            }
        }
    }
    true
}

pub unsafe fn prompt_enable_cloud_if_needed(hwnd: HWND, steam_dir: &Path) -> bool {
    let toml_path = steam_dir.join("opensteamtool.toml");
    if !toml_path.exists() {
        return true;
    }
    if is_opensteamtool_cloud_disabled(steam_dir) {
        let msg = to_wide(
            "AVISO IMPORTANTE:\r\n\r\n\
             O recurso [cloud] do OpenSteamTool está DESATIVADO no arquivo 'opensteamtool.toml'.\r\n\r\n\
             Se você não ativar o [cloud], o redirecionamento de saves da Steam pelo ChronoKeep NÃO irá funcionar!\r\n\r\n\
             Deseja que o ChronoKeep ative automaticamente o recurso [cloud] no seu opensteamtool.toml agora?",
        );
        let cap = to_wide("ChronoKeep — Recurso Cloud do OpenSteamTool");
        let resp = MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_YESNO | MB_ICONQUESTION);
        if resp == IDYES {
            ensure_opensteamtool_cloud_enabled(steam_dir);
            let ok_msg = to_wide("Recurso [cloud] ativado com sucesso em opensteamtool.toml!");
            let ok_cap = to_wide("Configuracao Atualizada");
            MessageBoxW(hwnd, ok_msg.as_ptr(), ok_cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
            return true;
        } else {
            let warn = to_wide(
                "Atenção: O recurso [cloud] permaneceu desativado.\r\n\
                 Lembre-se de ativá-lo no opensteamtool.toml para que os saves da nuvem sejam interceptados.",
            );
            let warn_cap = to_wide("Aviso");
            MessageBoxW(hwnd, warn.as_ptr(), warn_cap.as_ptr(), MB_OK | MB_ICONWARNING);
            return false;
        }
    }
    true
}

pub unsafe fn refresh_status_label(hwnd: HWND) {
    let steam_dir = get_control_text(hwnd, ID_EDIT_STEAM);
    let port = resolve_service_port(&steam_dir);

    let dll_exists = Path::new(&steam_dir).join("chronokeep.dll").exists()
        || Path::new(&steam_dir).join("cloud_redirect.dll").exists();
    let svc_online = check_service_online(port);
    let gdrive_ok = is_gdrive_connected(Path::new(&steam_dir));

    let gdrive_text = if gdrive_ok {
        "3. Conta Google Drive: [CONECTADA — Sincronizacao em nuvem ativa]"
    } else {
        "3. Conta Google Drive: [NAO CONECTADA — Clique abaixo para logar no navegador]"
    };
    set_control_text(hwnd, ID_LBL_GDRIVE_STATUS, gdrive_text);

    let status_text = format!(
        "Status: DLL na Steam: [{}]   |   Daemon (127.0.0.1:{}): [{}]   |   GDrive: [{}]",
        if dll_exists { "INSTALADA" } else { "NAO INSTALADA" },
        port,
        if svc_online { "ONLINE" } else { "OFFLINE" },
        if gdrive_ok { "OK" } else { "OFF" }
    );
    set_control_text(hwnd, ID_LBL_STATUS, &status_text);
}

pub unsafe fn populate_existing_config(hwnd: HWND) {
    let steam_dir = detect_default_steam_dir();
    let mut vault_dir = detect_default_vault_dir(None)
        .to_string_lossy()
        .into_owned();

    let ini_path = Path::new(&steam_dir).join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                if k.eq_ignore_ascii_case("vault_dir") && !v.is_empty() {
                    vault_dir = v.to_string();
                }
            }
        }
    }

    set_control_text(hwnd, ID_EDIT_STEAM, &steam_dir);
    set_control_text(hwnd, ID_EDIT_VAULT, &vault_dir);

    prompt_enable_cloud_if_needed(hwnd, Path::new(&steam_dir));
    refresh_status_label(hwnd);
}

pub fn run_silent_install() -> i32 {
    let steam_dir_str = detect_default_steam_dir();
    let vault_dir_str = detect_default_vault_dir(None)
        .to_string_lossy()
        .into_owned();
    let port = DEFAULT_SERVICE_PORT;
    let steam_dir = Path::new(&steam_dir_str);

    if !steam_dir.exists() {
        return 1;
    }

    let _ = fs::create_dir_all(&vault_dir_str);

    let ini_content = format!(
        "# Configuracao do ChronoKeep (Modo Silencioso)\r\n\
         port={}\r\n\
         vault_dir={}\r\n\
         dll_name=chronokeep.dll\r\n",
        port, vault_dir_str
    );
    let ini_path = steam_dir.join("chronokeep.ini");
    let _ = fs::write(&ini_path, ini_content);

    let steam_was_running = is_process_running("steam.exe");
    if steam_was_running {
        kill_process_and_wait("steam.exe", 5000);
    }
    kill_process_and_wait("chrono-daemon.exe", 2500);

    deploy_component(&steam_dir.join("chronokeep.dll"), "chronokeep.dll", EMBEDDED_CHRONOKEEP_DLL);
    deploy_component(&steam_dir.join("cloud_redirect.dll"), "cloud_redirect.dll", EMBEDDED_CHRONOKEEP_DLL);
    deploy_component(&steam_dir.join("chrono-daemon.exe"), "chrono-daemon.exe", EMBEDDED_CHRONO_DAEMON);

    ensure_opensteamtool_cloud_enabled(steam_dir);
    start_service_from_gui(steam_dir, port);

    if steam_was_running {
        std::thread::sleep(Duration::from_millis(600));
        launch_steam(steam_dir);
    }

    0
}

pub unsafe fn on_install_and_save(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);

    let steam_dir = Path::new(&steam_dir_str);
    if !steam_dir.exists() {
        let msg = to_wide("A pasta da Steam informada nao existe. Verifique o caminho.");
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    let _ = fs::create_dir_all(&vault_dir_str);

    let port: u16 = DEFAULT_SERVICE_PORT;
    let ini_content = format!(
        "# Configuracao do ChronoKeep gerada pelo Instalador GUI\r\n\
         port={}\r\n\
         vault_dir={}\r\n\
         dll_name=chronokeep.dll\r\n",
        port, vault_dir_str
    );
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Err(e) = fs::write(&ini_path, ini_content) {
        let msg = to_wide(&format!("Falha ao gravar chronokeep.ini: {}", e));
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    let steam_was_running = is_process_running("steam.exe");
    if steam_was_running {
        kill_process_and_wait("steam.exe", 5000);
    }
    kill_process_and_wait("chrono-daemon.exe", 2500);

    let mut deployed_files = Vec::new();

    let target_dll = steam_dir.join("chronokeep.dll");
    if deploy_component(&target_dll, "chronokeep.dll", EMBEDDED_CHRONOKEEP_DLL) {
        deployed_files.push("chronokeep.dll".to_string());
    }

    let target_compat = steam_dir.join("cloud_redirect.dll");
    if deploy_component(&target_compat, "cloud_redirect.dll", EMBEDDED_CHRONOKEEP_DLL) {
        deployed_files.push("cloud_redirect.dll (compatibilidade OpenSteamTool/Hubcap)".to_string());
    }

    let target_svc = steam_dir.join("chrono-daemon.exe");
    if deploy_component(&target_svc, "chrono-daemon.exe", EMBEDDED_CHRONO_DAEMON) {
        deployed_files.push("chrono-daemon.exe".to_string());
    }

    prompt_enable_cloud_if_needed(hwnd, steam_dir);
    start_service_from_gui(steam_dir, port);
    refresh_status_label(hwnd);

    if steam_was_running {
        std::thread::sleep(Duration::from_millis(600));
        launch_steam(steam_dir);
    }

    let restart_notice = if steam_was_running {
        "\r\n\r\n[OK] A Steam estava aberta e foi reaberta automaticamente com o ChronoKeep carregado!"
    } else {
        "\r\n\r\nAgora voce ja pode abrir a sua Steam normalmente para jogar!"
    };

    let summary = format!(
        "Instalacao do ChronoKeep concluida com sucesso!\r\n\r\n\
         Pasta da Steam: {}\r\n\
         Pasta de Saves (Vault): {}\r\n\
         Arquivos implantados: {}{}",
        steam_dir_str,
        vault_dir_str,
        deployed_files.join(", "),
        restart_notice
    );
    let msg = to_wide(&summary);
    let cap = to_wide("ChronoKeep — Instalacao Concluida");
    MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
}
