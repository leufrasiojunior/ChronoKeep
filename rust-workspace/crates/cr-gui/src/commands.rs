//! Tauri IPC commands connecting the modern Fluent UI with the ChronoKeep Rust backend.

use crate::installer::{
    deploy_component, ensure_opensteamtool_cloud_enabled, is_opensteamtool_cloud_disabled,
};
use crate::service_mgr::{
    kill_process_and_wait, start_service_from_gui, stop_service_from_gui,
};
use cr_common::config::{
    check_daemon_health, detect_default_steam_dir, detect_default_vault_dir, is_gdrive_connected,
    resolve_service_port, DEFAULT_SERVICE_PORT,
};
use cr_common::fs_utils::{collect_files_recursive, format_size};
use cr_common::gdrive::{
    exchange_code_for_tokens, DEFAULT_GDRIVE_CLIENT_ID as GDRIVE_CLIENT_ID,
    DEFAULT_GDRIVE_SCOPE as GDRIVE_SCOPE,
};
use cr_common::http::{extract_json_str, url_decode, url_encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const EMBEDDED_CHRONO_DAEMON: &[u8] = include_bytes!("../embedded/chrono-daemon.exe");
pub const EMBEDDED_CHRONOKEEP_DLL: &[u8] = include_bytes!("../embedded/chronokeep.dll");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SystemInfoDto {
    pub steam_dir: String,
    pub vault_dir: String,
    pub opensteamtool_active: bool,
    pub is_installed: bool,
    pub version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigDto {
    pub steam_dir: String,
    pub vault_dir: String,
    pub port: u16,
    pub gdrive_connected: bool,
    pub max_upload_mb: u64,
    pub theme: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceStatusDto {
    pub online: bool,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameSaveItemDto {
    pub account_id: u32,
    pub app_id: u32,
    pub name: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub total_size_formatted: String,
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub fn get_system_info() -> SystemInfoDto {
    let steam_dir = detect_default_steam_dir();
    let s_path = Path::new(&steam_dir);
    let vault_dir = detect_default_vault_dir(None).to_string_lossy().into_owned();

    let ost_disabled = is_opensteamtool_cloud_disabled(s_path);
    let opensteamtool_active = s_path.join("opensteamtool.toml").exists() && !ost_disabled;

    let is_installed = (s_path.join("chronokeep.dll").exists()
        || s_path.join("cloud_redirect.dll").exists())
        && s_path.join("chrono-daemon.exe").exists();

    SystemInfoDto {
        steam_dir,
        vault_dir,
        opensteamtool_active,
        is_installed,
        version: "0.1.2".to_string(),
    }
}

#[tauri::command]
pub fn get_config() -> ConfigDto {
    let steam_dir = detect_default_steam_dir();
    let s_path = Path::new(&steam_dir);
    let mut vault_dir = detect_default_vault_dir(None).to_string_lossy().into_owned();
    let mut port = DEFAULT_SERVICE_PORT;
    let mut max_upload_mb = 512;
    let mut theme = "system".to_string();

    let ini_path = s_path.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = t.split_once('=') {
                let k = k.trim().to_ascii_lowercase();
                let v = v.trim();
                match k.as_str() {
                    "vault_dir" if !v.is_empty() => vault_dir = v.to_string(),
                    "port" => port = v.parse().unwrap_or(DEFAULT_SERVICE_PORT),
                    "max_upload_mb" => max_upload_mb = v.parse().unwrap_or(512),
                    "theme" => theme = v.to_string(),
                    _ => {}
                }
            }
        }
    }

    let gdrive_connected = is_gdrive_connected(s_path);

    ConfigDto {
        steam_dir,
        vault_dir,
        port,
        gdrive_connected,
        max_upload_mb,
        theme,
    }
}

#[tauri::command]
pub fn save_config(cfg: ConfigDto) -> OperationResult {
    let s_path = Path::new(&cfg.steam_dir);
    if !s_path.exists() {
        return OperationResult {
            success: false,
            message: format!("A pasta da Steam não existe: {}", cfg.steam_dir),
        };
    }

    let _ = fs::create_dir_all(&cfg.vault_dir);

    let ini_path = s_path.join("chronokeep.ini");
    let mut lines = Vec::new();

    // Preserva tokens existentes se houver
    let mut refresh_token = String::new();
    let mut gdrive_folder_id = String::new();
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            if let Some((k, v)) = line.trim().split_once('=') {
                let k = k.trim().to_ascii_lowercase();
                let v = v.trim();
                if k == "gdrive_refresh_token" {
                    refresh_token = v.to_string();
                } else if k == "gdrive_folder_id" {
                    gdrive_folder_id = v.to_string();
                }
            }
        }
    }

    lines.push("# Configuracao do ChronoKeep".to_string());
    lines.push(format!("port={}", cfg.port));
    lines.push(format!("vault_dir={}", cfg.vault_dir));
    lines.push("dll_name=chronokeep.dll".to_string());
    lines.push(format!("max_upload_mb={}", cfg.max_upload_mb));
    lines.push(format!("theme={}", cfg.theme));

    if !refresh_token.is_empty() {
        lines.push(format!("gdrive_refresh_token={}", refresh_token));
    }
    if !gdrive_folder_id.is_empty() {
        lines.push(format!("gdrive_folder_id={}", gdrive_folder_id));
    }

    let content = lines.join("\r\n") + "\r\n";
    if fs::write(&ini_path, content).is_ok() {
        // Se o daemon estiver ativo, reinicia para carregar imediatamente as novas configurações (limite de upload, vault, etc.)
        if check_daemon_health(cfg.port) {
            stop_service_from_gui(cfg.port);
            start_service_from_gui(s_path, cfg.port);
        }
        OperationResult {
            success: true,
            message: "Configurações salvas e aplicadas com sucesso!".to_string(),
        }
    } else {
        OperationResult {
            success: false,
            message: "Falha ao gravar arquivo chronokeep.ini".to_string(),
        }
    }
}

#[tauri::command]
pub fn get_service_status(steam_dir: Option<String>) -> ServiceStatusDto {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let port = resolve_service_port(&s_dir);
    let online = check_daemon_health(port);
    ServiceStatusDto { online, port }
}

#[tauri::command]
pub fn start_service(steam_dir: Option<String>) -> OperationResult {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let s_path = Path::new(&s_dir);
    let port = resolve_service_port(&s_dir);

    if check_daemon_health(port) {
        return OperationResult {
            success: true,
            message: format!("O serviço já está ativo na porta {}", port),
        };
    }

    let started = start_service_from_gui(s_path, port);
    if started {
        OperationResult {
            success: true,
            message: format!("ChronoKeep Daemon iniciado com sucesso na porta {}!", port),
        }
    } else {
        OperationResult {
            success: false,
            message: "Falha ao iniciar chrono-daemon.exe. Verifique se o executável existe.".to_string(),
        }
    }
}

#[tauri::command]
pub fn stop_service(steam_dir: Option<String>) -> OperationResult {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let port = resolve_service_port(&s_dir);
    let stopped = stop_service_from_gui(port);
    if stopped {
        OperationResult {
            success: true,
            message: "ChronoKeep Daemon encerrado.".to_string(),
        }
    } else {
        OperationResult {
            success: false,
            message: "O serviço já estava desligado ou não respondeu ao comando de parada.".to_string(),
        }
    }
}

#[tauri::command]
pub fn install_chronokeep(steam_dir: String, vault_dir: String) -> OperationResult {
    let s_path = Path::new(&steam_dir);
    if !s_path.exists() {
        return OperationResult {
            success: false,
            message: "A pasta da Steam informada não foi encontrada.".to_string(),
        };
    }

    let _ = fs::create_dir_all(&vault_dir);

    // Encerra daemon se ativo
    kill_process_and_wait("chrono-daemon.exe", 2000);

    // Deploy dos binários
    let dll_ok = deploy_component(
        &s_path.join("chronokeep.dll"),
        "chronokeep.dll",
        EMBEDDED_CHRONOKEEP_DLL,
    );
    let _ = deploy_component(
        &s_path.join("cloud_redirect.dll"),
        "cloud_redirect.dll",
        EMBEDDED_CHRONOKEEP_DLL,
    );
    let daemon_ok = deploy_component(
        &s_path.join("chrono-daemon.exe"),
        "chrono-daemon.exe",
        EMBEDDED_CHRONO_DAEMON,
    );

    if !dll_ok || !daemon_ok {
        return OperationResult {
            success: false,
            message: "Falha ao copiar arquivos para a pasta da Steam. Verifique permissões de gravação.".to_string(),
        };
    }

    // Configuração inicial
    let port = resolve_service_port(&steam_dir);
    let ini_path = s_path.join("chronokeep.ini");
    if !ini_path.exists() {
        let ini_content = format!(
            "# Configuracao gerada pelo ChronoKeep Setup\r\n\
             port={}\r\n\
             vault_dir={}\r\n\
             dll_name=chronokeep.dll\r\n",
            port, vault_dir
        );
        let _ = fs::write(&ini_path, ini_content);
    }

    // Ativa [cloud] no opensteamtool.toml se existir
    ensure_opensteamtool_cloud_enabled(s_path);

    // Inicia serviço em segundo plano
    start_service_from_gui(s_path, port);

    OperationResult {
        success: true,
        message: "ChronoKeep instalado na Steam com sucesso! O serviço daemon foi iniciado.".to_string(),
    }
}

#[tauri::command]
pub fn start_google_login(steam_dir: Option<String>) -> Result<OperationResult, String> {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let s_path = Path::new(&s_dir);

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Falha ao abrir porta local: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Falha ao ler porta: {}", e))?
        .port();

    let redirect_uri = format!("http://localhost:{}/callback", port);
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        url_encode(GDRIVE_CLIENT_ID),
        url_encode(&redirect_uri),
        url_encode(GDRIVE_SCOPE)
    );

    // Abre URL no navegador padrão do sistema
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = OsStr::new(&auth_url).encode_wide().chain(std::iter::once(0)).collect();
        let op: Vec<u16> = OsStr::new("open").encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            crate::win32::ShellExecuteW(0, op.as_ptr(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        }
    }

    // Escuta callback
    let mut auth_code = None;
    for stream in listener.incoming().flatten() {
        let mut s = stream;
        let _ = s.set_read_timeout(Some(Duration::from_secs(45)));
        let mut buf = [0u8; 4096];
        let Ok(n) = s.read(&mut buf) else { break };
        let req_str = String::from_utf8_lossy(&buf[..n]);
        let first_line = req_str.lines().next().unwrap_or("");
        if !first_line.contains("/callback") {
            let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            continue;
        }

        if let Some(q_idx) = first_line.find('?') {
            let query_part = first_line[q_idx + 1..].split_whitespace().next().unwrap_or("");
            for pair in query_part.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    if k == "code" {
                        auth_code = Some(url_decode(v));
                    }
                }
            }
        }

        let html = "<html><head><meta charset='utf-8'><title>ChronoKeep</title></head>\
            <body style='background:#18181B;color:#F4F4F5;font-family:Segoe UI,sans-serif;text-align:center;padding-top:80px;'>\
            <h1 style='color:#60CDFF;'>ChronoKeep Conectado!</h1>\
            <p>Sua conta do Google Drive foi vinculada com sucesso. Você já pode fechar esta aba e voltar ao aplicativo.</p>\
            </body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = s.write_all(resp.as_bytes());
        let _ = s.flush();
        break;
    }

    let code = auth_code.ok_or_else(|| "Nenhum código de autorização recebido do navegador.".to_string())?;

    let json_resp = exchange_code_for_tokens(&code, &redirect_uri)
        .ok_or_else(|| "Falha ao trocar código de autorização por tokens no Google.".to_string())?;

    // 1. Persistir o token JSON completo onde o daemon e is_gdrive_connected buscam
    let token_json_path = s_path.join("chronokeep_gdrive.json");
    let _ = fs::write(&token_json_path, &json_resp);
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            if parent != s_path {
                let _ = fs::write(parent.join("chronokeep_gdrive.json"), &json_resp);
            }
        }
    }

    // 2. Extrair e manter refresh_token em chronokeep.ini para retrocompatibilidade
    let refresh_token = extract_json_str(&json_resp, "refresh_token");
    let ini_path = s_path.join("chronokeep.ini");
    let mut lines = Vec::new();
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for l in content.lines() {
            let t = l.trim();
            if !t.to_ascii_lowercase().starts_with("gdrive_refresh_token") {
                lines.push(t.to_string());
            }
        }
    }
    if let Some(rt) = refresh_token {
        lines.push(format!("gdrive_refresh_token={}", rt));
    }
    let _ = fs::write(&ini_path, lines.join("\r\n") + "\r\n");

    // 3. Reiniciar daemon se ativo para reconhecer o novo token imediatamente
    let port = resolve_service_port(&s_dir);
    if check_daemon_health(port) {
        stop_service_from_gui(port);
        start_service_from_gui(s_path, port);
    }

    Ok(OperationResult {
        success: true,
        message: "Google Drive vinculado com sucesso!".to_string(),
    })
}

#[tauri::command]
pub fn disconnect_google(steam_dir: Option<String>) -> OperationResult {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let s_path = Path::new(&s_dir);

    // 1. Remover entradas do chronokeep.ini
    let ini_path = s_path.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        let lines: Vec<String> = content
            .lines()
            .filter(|l| {
                let t = l.trim().to_ascii_lowercase();
                !t.starts_with("gdrive_refresh_token") && !t.starts_with("gdrive_folder_id")
            })
            .map(|s| s.to_string())
            .collect();
        let _ = fs::write(&ini_path, lines.join("\r\n") + "\r\n");
    }

    // 2. Apagar chronokeep_gdrive.json na pasta da Steam
    let token_json_path = s_path.join("chronokeep_gdrive.json");
    if token_json_path.exists() {
        let _ = fs::remove_file(&token_json_path);
    }

    // 3. Apagar chronokeep_gdrive.json na pasta do executável atual, se existir
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            let p_token = parent.join("chronokeep_gdrive.json");
            if p_token.exists() {
                let _ = fs::remove_file(&p_token);
            }
        }
    }

    // 4. Limpar cache em memória de pastas e token do Google Drive
    cr_common::gdrive::clear_folder_cache();
    cr_common::gdrive::clear_token_cache();

    // 5. Reiniciar o daemon se ativo para invalidar qualquer TOKEN_CACHE de processo em memória
    let port = resolve_service_port(&s_dir);
    if check_daemon_health(port) {
        stop_service_from_gui(port);
        start_service_from_gui(s_path, port);
    }

    OperationResult {
        success: true,
        message: "Conta do Google Drive desconectada com sucesso.".to_string(),
    }
}

#[tauri::command]
pub fn upload_game_save(
    app_id: u32,
    account_id: u32,
    steam_dir: Option<String>,
    vault_dir: Option<String>,
) -> OperationResult {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let v_dir = vault_dir.unwrap_or_else(|| detect_default_vault_dir(None).to_string_lossy().into_owned());
    let steam_path = Path::new(&s_dir);
    let vault_path = Path::new(&v_dir);

    match crate::orphans::upload_single_game_save(steam_path, vault_path, app_id, account_id) {
        Ok(count) => OperationResult {
            success: true,
            message: format!("Sucesso! {} arquivo(s) do AppID {} sincronizados com o Google Drive.", count, app_id),
        },
        Err(err) => OperationResult {
            success: false,
            message: err,
        },
    }
}

#[tauri::command]
pub fn scan_saves(steam_dir: Option<String>, vault_dir: Option<String>) -> Vec<GameSaveItemDto> {
    let s_dir = steam_dir.unwrap_or_else(detect_default_steam_dir);
    let v_dir = vault_dir.unwrap_or_else(|| detect_default_vault_dir(None).to_string_lossy().into_owned());
    let steam_path = Path::new(&s_dir);
    let vault_path = Path::new(&v_dir);

    let mut map: BTreeMap<(u32, u32), (usize, u64, String)> = BTreeMap::new();

    // 1. Escanear Steam userdata
    let userdata_dir = steam_path.join("userdata");
    if userdata_dir.exists() {
        if let Ok(entries) = fs::read_dir(&userdata_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_dir() { continue; }
                let Some(acct_str) = p.file_name().and_then(|n| n.to_str()) else { continue };
                let Ok(account_id) = acct_str.parse::<u32>() else { continue };
                if account_id == 0 { continue; }

                if let Ok(apps) = fs::read_dir(&p) {
                    for app_entry in apps.flatten() {
                        let app_path = app_entry.path();
                        if !app_path.is_dir() { continue; }
                        let Some(app_str) = app_path.file_name().and_then(|n| n.to_str()) else { continue };
                        let Ok(app_id) = app_str.parse::<u32>() else { continue };
                        if app_id == 7 || app_id == 760 || app_id == 241100 || app_id == 241300 {
                            continue;
                        }

                        let remote_dir = app_path.join("remote");
                        let mut files: Vec<(String, PathBuf)> = Vec::new();
                        if remote_dir.exists() {
                            collect_files_recursive(&remote_dir, &remote_dir, &mut files);
                        }
                        if !files.is_empty() {
                            let total_bytes: u64 = files.iter().map(|(_, p)| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
                            map.insert((account_id, app_id), (files.len(), total_bytes, "Steam Userdata".to_string()));
                        }
                    }
                }
            }
        }
    }

    // 2. Escanear Vault
    if vault_path.exists() {
        if let Ok(entries) = fs::read_dir(vault_path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_dir() { continue; }
                let Some(acct_str) = p.file_name().and_then(|n| n.to_str()) else { continue };
                let Ok(account_id) = acct_str.parse::<u32>() else { continue };

                if let Ok(apps) = fs::read_dir(&p) {
                    for app_entry in apps.flatten() {
                        let app_path = app_entry.path();
                        if !app_path.is_dir() { continue; }
                        let Some(app_str) = app_path.file_name().and_then(|n| n.to_str()) else { continue };
                        let Ok(app_id) = app_str.parse::<u32>() else { continue };
                        if app_id == 7 || app_id == 760 || app_id == 241100 || app_id == 241300 {
                            continue;
                        }

                        let mut files: Vec<(String, PathBuf)> = Vec::new();
                        collect_files_recursive(&app_path, &app_path, &mut files);
                        files.retain(|(rel, _)| rel != "manifest.tsv");

                        if !files.is_empty() {
                            let total_bytes: u64 = files.iter().map(|(_, p)| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
                            map.entry((account_id, app_id))
                                .and_modify(|e| {
                                    e.0 = e.0.max(files.len());
                                    e.1 = e.1.max(total_bytes);
                                    e.2 = "Steam + Vault".to_string();
                                })
                                .or_insert((files.len(), total_bytes, "Vault Local".to_string()));
                        }
                    }
                }
            }
        }
    }

    map.into_iter()
        .map(|((account_id, app_id), (file_count, total_bytes, source))| GameSaveItemDto {
            account_id,
            app_id,
            name: format!("AppID {}", app_id),
            file_count,
            total_bytes,
            total_size_formatted: format_size(total_bytes),
            source,
        })
        .collect()
}

#[tauri::command]
pub fn get_logs(log_name: String, max_lines: Option<usize>) -> String {
    let steam_dir = detect_default_steam_dir();
    let s_path = Path::new(&steam_dir);
    let log_file = match log_name.as_str() {
        "bridge" => s_path.join("chronokeep_bridge.log"),
        _ => s_path.join("chrono_daemon.log"),
    };

    if !log_file.exists() {
        return format!("[Log '{}' vazio ou ainda não criado em {}]", log_name, log_file.display());
    }

    let limit = max_lines.unwrap_or(300);
    if let Ok(content) = fs::read_to_string(&log_file) {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() <= limit {
            content
        } else {
            lines[lines.len() - limit..].join("\n")
        }
    } else {
        "[Erro ao ler arquivo de log]".to_string()
    }
}

#[tauri::command]
pub fn clear_log(log_name: String) -> OperationResult {
    let steam_dir = detect_default_steam_dir();
    let s_path = Path::new(&steam_dir);
    let log_file = match log_name.as_str() {
        "bridge" => s_path.join("chronokeep_bridge.log"),
        _ => s_path.join("chrono_daemon.log"),
    };

    if fs::write(&log_file, "").is_ok() {
        OperationResult {
            success: true,
            message: "Arquivo de log limpo com sucesso.".to_string(),
        }
    } else {
        OperationResult {
            success: false,
            message: "Falha ao limpar arquivo de log.".to_string(),
        }
    }
}

#[tauri::command]
pub fn open_directory(path: String) -> OperationResult {
    let p = Path::new(&path);
    if !p.exists() {
        return OperationResult {
            success: false,
            message: format!("Caminho não encontrado: {}", path),
        };
    }

    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = OsStr::new(&path).encode_wide().chain(std::iter::once(0)).collect();
        let op: Vec<u16> = OsStr::new("explore").encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            crate::win32::ShellExecuteW(0, op.as_ptr(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        }
    }

    OperationResult {
        success: true,
        message: "Pasta aberta no Explorer.".to_string(),
    }
}
