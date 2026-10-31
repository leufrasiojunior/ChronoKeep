//! Tauri IPC commands connecting the modern Fluent UI with the ChronoKeep Rust backend.

use crate::installer::{
    deploy_component, ensure_opensteamtool_cloud_enabled, is_opensteamtool_cloud_disabled,
};
use crate::service_mgr::{
    is_process_running, kill_process_and_wait, launch_steam, start_service_from_gui,
    stop_service_from_gui,
};
use cr_common::config::{
    check_daemon_health, detect_default_steam_dir, detect_default_vault_dir, is_gdrive_connected,
    resolve_service_port, DEFAULT_SERVICE_PORT,
};
use cr_common::fs_utils::{collect_files_recursive, format_size};
use cr_common::manifest::parse_manifest;
use cr_common::gdrive::{
    exchange_code_for_tokens, DEFAULT_GDRIVE_CLIENT_ID as GDRIVE_CLIENT_ID,
    DEFAULT_GDRIVE_SCOPE as GDRIVE_SCOPE,
};
use cr_common::http::{extract_json_str, url_decode, url_encode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
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
    pub in_local: bool,
    pub in_cloud: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
}

fn get_gui_prefs_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("ChronoKeep").join("gui_prefs.ini");
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            return parent.join("gui_prefs.ini");
        }
    }
    PathBuf::from("gui_prefs.ini")
}

pub fn load_preferred_steam_dir() -> String {
    let prefs_file = get_gui_prefs_path();
    if let Ok(content) = fs::read_to_string(&prefs_file) {
        for line in content.lines() {
            if let Some((k, v)) = line.trim().split_once('=') {
                if k.trim().eq_ignore_ascii_case("last_steam_dir") {
                    let path = v.trim();
                    if !path.is_empty() && Path::new(path).exists() {
                        return path.to_string();
                    }
                }
            }
        }
    }
    detect_default_steam_dir()
}

pub fn save_preferred_steam_dir(steam_dir: &str) {
    let prefs_file = get_gui_prefs_path();
    if let Some(parent) = prefs_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut lines = Vec::new();
    let mut updated = false;
    if let Ok(content) = fs::read_to_string(&prefs_file) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case("last_steam_dir") {
                    lines.push(format!("last_steam_dir={}", steam_dir));
                    updated = true;
                    continue;
                }
            }
            if !trimmed.is_empty() {
                lines.push(trimmed.to_string());
            }
        }
    }
    if !updated {
        lines.push(format!("last_steam_dir={}", steam_dir));
    }
    let content = lines.join("\r\n") + "\r\n";
    let _ = fs::write(prefs_file, content);
}

#[tauri::command]
pub fn get_system_info() -> SystemInfoDto {
    let steam_dir = load_preferred_steam_dir();
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
    let steam_dir = load_preferred_steam_dir();
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

    let current_port = resolve_service_port(&cfg.steam_dir);
    let content = lines.join("\r\n") + "\r\n";
    if fs::write(&ini_path, content).is_ok() {
        save_preferred_steam_dir(&cfg.steam_dir);
        // Se o daemon estiver ativo (na porta anterior ou na nova), reinicia para aplicar as novas configurações
        let was_running = check_daemon_health(current_port) || (current_port != cfg.port && check_daemon_health(cfg.port));
        if was_running {
            stop_service_from_gui(current_port);
            if current_port != cfg.port {
                stop_service_from_gui(cfg.port);
            }
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
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
    let port = resolve_service_port(&s_dir);
    let online = check_daemon_health(port);
    ServiceStatusDto { online, port }
}

#[tauri::command]
pub fn start_service(steam_dir: Option<String>) -> OperationResult {
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
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
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
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

// TODO: Possibilidade de atualizar individualmente o daemon, a dll e o executavel:
// - Atualizar somente o daemon (chrono-daemon.exe) sem reiniciar o Steam se a DLL nao mudou.
// - Atualizar somente a DLL (chronokeep.dll / cloud_redirect.dll).
// - Auto-atualizacao do proprio executavel do painel (ChronoKeepSetup.exe).
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

    // Encerra Steam se em execução para evitar bloqueio ao sobrescrever a DLL carregada
    let steam_was_running = is_process_running("steam.exe");
    if steam_was_running {
        kill_process_and_wait("steam.exe", 5000);
    }

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
        // TODO: Relaunch steam if it was running before returning error (steam_was_running)
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

    // Salva o diretório da Steam como preferência da GUI
    save_preferred_steam_dir(&steam_dir);

    // Inicia serviço em segundo plano
    start_service_from_gui(s_path, port);

    // Relança a Steam se estava aberta antes da instalação
    if steam_was_running {
        std::thread::sleep(Duration::from_millis(600));
        launch_steam(s_path);
    }

    OperationResult {
        success: true,
        message: "ChronoKeep instalado na Steam com sucesso! O serviço daemon foi iniciado.".to_string(),
    }
}

#[tauri::command]
pub fn start_google_login(steam_dir: Option<String>) -> Result<OperationResult, String> {
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
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

    // Escuta callback e valida resultado antes de responder ao navegador
    let mut exchange_result = None;
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

        let mut auth_code = None;
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

        if let Some(code) = auth_code {
            if let Some(json_resp) = exchange_code_for_tokens(&code, &redirect_uri) {
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
                exchange_result = Some(Ok(json_resp));
                break;
            } else {
                let html = "<html><head><meta charset='utf-8'><title>ChronoKeep - Erro</title></head>\
                    <body style='background:#18181B;color:#F4F4F5;font-family:Segoe UI,sans-serif;text-align:center;padding-top:80px;'>\
                    <h1 style='color:#F87171;'>Falha na Vinculação</h1>\
                    <p>Não foi possível trocar o código de autorização pelos tokens de acesso do Google. Você já pode fechar esta aba e tentar novamente no aplicativo.</p>\
                    </body></html>";
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.flush();
                exchange_result = Some(Err("Falha ao trocar código de autorização por tokens no Google.".to_string()));
                break;
            }
        } else {
            let html = "<html><head><meta charset='utf-8'><title>ChronoKeep - Cancelado</title></head>\
                <body style='background:#18181B;color:#F4F4F5;font-family:Segoe UI,sans-serif;text-align:center;padding-top:80px;'>\
                <h1 style='color:#F87171;'>Autorização Cancelada ou Negada</h1>\
                <p>Nenhum código de autorização foi fornecido pelo Google. Você já pode fechar esta aba e tentar novamente no aplicativo.</p>\
                </body></html>";
            let resp = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
            exchange_result = Some(Err("Nenhum código de autorização recebido do navegador.".to_string()));
            break;
        }
    }

    let json_resp = match exchange_result {
        Some(Ok(resp)) => resp,
        Some(Err(err)) => return Err(err),
        None => return Err("Tempo limite esgotado aguardando autorização no navegador.".to_string()),
    };

    // 1. Persistir o token JSON completo onde o daemon e is_gdrive_connected buscam
    let token_json_path = s_path.join("chronokeep_gdrive.json");
    fs::write(&token_json_path, &json_resp).map_err(|e| {
        format!(
            "Falha de permissão ao gravar token em '{}': {}. Execute o ChronoKeep como Administrador ou conceda permissão de escrita.",
            token_json_path.display(),
            e
        )
    })?;
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            if parent != s_path && parent.join("chrono-daemon.exe").exists() {
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
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
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
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
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

fn get_app_names_cache_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(appdata).join("ChronoKeep");
        let _ = fs::create_dir_all(&p);
        return p.join("app_names_cache.json");
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            return parent.join("app_names_cache.json");
        }
    }
    PathBuf::from("app_names_cache.json")
}

fn load_app_names_cache() -> HashMap<u32, String> {
    let p = get_app_names_cache_path();
    if let Ok(content) = fs::read_to_string(&p) {
        if let Ok(map) = serde_json::from_str::<HashMap<u32, String>>(&content) {
            return map;
        }
    }
    HashMap::new()
}

// TODO: Filtrar placeholders temporarios ("AppID <id>") antes de persistir o cache em disco,
// permitindo tentar buscar novamente o nome real na Steam Store em varreduras futuras.
fn save_app_names_cache(cache: &HashMap<u32, String>) {
    let p = get_app_names_cache_path();
    if let Ok(content) = serde_json::to_string_pretty(cache) {
        let _ = fs::write(p, content);
    }
}

fn read_game_name_from_manifest(acf_path: &Path) -> Option<String> {
    if let Ok(content) = fs::read_to_string(acf_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.to_ascii_lowercase().starts_with("\"name\"") {
                let parts: Vec<&str> = line.split('"').filter(|s| !s.trim().is_empty()).collect();
                if parts.len() >= 2 {
                    let name = parts[1].trim();
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

fn get_steam_library_folders(steam_path: &Path) -> Vec<PathBuf> {
    let mut folders = vec![steam_path.to_path_buf()];
    let vdf = steam_path.join("steamapps").join("libraryfolders.vdf");
    if let Ok(content) = fs::read_to_string(&vdf) {
        for line in content.lines() {
            let line = line.trim();
            if line.to_ascii_lowercase().starts_with("\"path\"") {
                let parts: Vec<&str> = line.split('"').filter(|s| !s.trim().is_empty()).collect();
                if parts.len() >= 2 {
                    let p = PathBuf::from(parts[1].trim());
                    if p.exists() && !folders.contains(&p) {
                        folders.push(p);
                    }
                }
            }
        }
    }
    folders
}

fn resolve_game_names_batch(
    app_ids: &[u32],
    libraries: &[PathBuf],
    cache: &mut HashMap<u32, String>,
) {
    let mut missing = Vec::new();

    // 1. Verificar cache e arquivos .acf locais das bibliotecas Steam (0ms offline)
    for &app_id in app_ids {
        if let Some(cached) = cache.get(&app_id) {
            if !cached.trim().is_empty() {
                continue;
            }
        }

        let mut found_local = None;
        for lib in libraries {
            let acf = lib.join("steamapps").join(format!("appmanifest_{}.acf", app_id));
            if let Some(name) = read_game_name_from_manifest(&acf) {
                found_local = Some(name);
                break;
            }
        }

        if let Some(name) = found_local {
            cache.insert(app_id, name);
        } else {
            missing.push(app_id);
        }
    }

    if missing.is_empty() {
        return;
    }

    // 2. Para jogos sem manifesto local e ainda não no cache:
    // Consulta a Steam Store API com verificação de conectividade. Se a API estiver fora do ar
    // ou a primeira chamada der timeout/erro, interrompe imediatamente novas requisições nesta sessão
    // e registra o fallback "AppID <id>" em cache (miss caching), impedindo travamentos seriais.
    #[cfg(target_os = "windows")]
    {
        let mut network_available = true;
        for app_id in missing {
            if !network_available {
                cache.insert(app_id, format!("AppID {}", app_id));
                continue;
            }
            let path = format!("/api/appdetails?appids={}&filters=basic", app_id);
            if let Some((200, resp)) = cr_common::http::https_request("GET", "store.steampowered.com", &path, &[], &[]) {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp) {
                    if let Some(name) = val.get(&app_id.to_string())
                        .and_then(|app| app.get("data"))
                        .and_then(|data| data.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        cache.insert(app_id, name.to_string());
                        continue;
                    }
                }
                cache.insert(app_id, format!("AppID {}", app_id));
            } else {
                network_available = false;
                cache.insert(app_id, format!("AppID {}", app_id));
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        for app_id in missing {
            cache.insert(app_id, format!("AppID {}", app_id));
        }
    }
}

#[tauri::command]
pub fn scan_saves(steam_dir: Option<String>, vault_dir: Option<String>) -> Vec<GameSaveItemDto> {
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
    let v_dir = vault_dir.unwrap_or_else(|| detect_default_vault_dir(None).to_string_lossy().into_owned());
    let steam_path = Path::new(&s_dir);
    let vault_path = Path::new(&v_dir);

    // Map: (account_id, app_id) -> (file_count, total_bytes, source, in_cloud)
    let mut map: BTreeMap<(u32, u32), (usize, u64, String, bool)> = BTreeMap::new();

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
                            map.insert((account_id, app_id), (files.len(), total_bytes, "Steam Userdata".to_string(), false));
                        }
                    }
                }
            }
        }
    }

    // 2. Escanear Cofre (Vault)
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

                        let manifest_p = app_path.join("manifest.tsv");
                        let in_cloud = if manifest_p.exists() {
                            fs::read_to_string(&manifest_p)
                                .map(|c| {
                                    let m = parse_manifest(&c);
                                    m.change_number > 0 && !m.files.is_empty()
                                })
                                .unwrap_or(false)
                        } else {
                            false
                        };

                        let files_dir = app_path.join("files");
                        let scan_target = if files_dir.exists() { &files_dir } else { &app_path };
                        let mut files: Vec<(String, PathBuf)> = Vec::new();
                        collect_files_recursive(scan_target, scan_target, &mut files);
                        files.retain(|(rel, _)| rel != "manifest.tsv");

                        if !files.is_empty() {
                            let total_bytes: u64 = files.iter().map(|(_, p)| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
                            map.entry((account_id, app_id))
                                .and_modify(|e| {
                                    e.0 = e.0.max(files.len());
                                    e.1 = e.1.max(total_bytes);
                                    e.2 = "Steam + Cofre".to_string();
                                    e.3 = e.3 || in_cloud;
                                })
                                .or_insert((files.len(), total_bytes, "Cofre Local".to_string(), in_cloud));
                        }
                    }
                }
            }
        }
    }

    let libraries = get_steam_library_folders(steam_path);
    let mut cache = load_app_names_cache();

    let all_app_ids: Vec<u32> = map.keys().map(|(_, app_id)| *app_id).collect();
    resolve_game_names_batch(&all_app_ids, &libraries, &mut cache);

    let items: Vec<GameSaveItemDto> = map.into_iter()
        .map(|((account_id, app_id), (file_count, total_bytes, source, in_cloud))| {
            let name = cache.get(&app_id).cloned().unwrap_or_else(|| format!("AppID {}", app_id));
            GameSaveItemDto {
                account_id,
                app_id,
                name,
                file_count,
                total_bytes,
                total_size_formatted: format_size(total_bytes),
                source,
                in_local: true,
                in_cloud,
            }
        })
        .collect();

    save_app_names_cache(&cache);
    items
}

#[tauri::command]
pub fn get_logs(log_name: String, max_lines: Option<usize>, steam_dir: Option<String>) -> String {
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
    let s_path = Path::new(&s_dir);
    let log_file = match log_name.as_str() {
        "bridge" => s_path.join("chronokeep_bridge.log"),
        _ => s_path.join("chrono_daemon.log"),
    };

    if !log_file.exists() {
        return format!("[Log '{}' vazio ou ainda não criado em {}]", log_name, log_file.display());
    }

    let limit = max_lines.unwrap_or(300);
    let content = if let Ok(meta) = fs::metadata(&log_file) {
        if meta.len() > 1024 * 1024 {
            if let Ok(mut f) = fs::File::open(&log_file) {
                use std::io::{Read, Seek, SeekFrom};
                let offset = meta.len().saturating_sub(512 * 1024);
                let _ = f.seek(SeekFrom::Start(offset));
                let mut buf = Vec::new();
                let _ = f.read_to_end(&mut buf);
                String::from_utf8_lossy(&buf).to_string()
            } else {
                fs::read_to_string(&log_file).unwrap_or_default()
            }
        } else {
            fs::read_to_string(&log_file).unwrap_or_default()
        }
    } else {
        fs::read_to_string(&log_file).unwrap_or_default()
    };

    if content.is_empty() {
        return "[Arquivo de log vazio]".to_string();
    }

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= limit {
        lines.join("\n")
    } else {
        lines[lines.len() - limit..].join("\n")
    }
}

#[tauri::command]
pub fn clear_log(log_name: String, steam_dir: Option<String>) -> OperationResult {
    let s_dir = steam_dir.unwrap_or_else(load_preferred_steam_dir);
    let s_path = Path::new(&s_dir);
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

#[tauri::command]
pub fn open_url(url: String) -> OperationResult {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = OsStr::new(&url).encode_wide().chain(std::iter::once(0)).collect();
        let op: Vec<u16> = OsStr::new("open").encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            crate::win32::ShellExecuteW(0, op.as_ptr(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }

    OperationResult {
        success: true,
        message: "Link aberto no navegador.".to_string(),
    }
}
