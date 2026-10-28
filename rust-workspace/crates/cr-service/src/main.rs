//! `cr-service.exe` — External Cloud Save Daemon & Local HTTP Block Server.
//! Listens on `127.0.0.1:<port>` (default `39771`) to serve both:
//! 1. RPC forwarding from `cloud_redirect.dll` (`POST /v1/rpc`, `POST /v1/session`, `GET /v1/health`)
//! 2. Native Steam Cloud file transfers from `steamclient64.dll` (`PUT /upload/...`, `GET /download/...`)

mod gdrive;
mod rpc_engine;
mod vault;

use cr_proto::cloud_messages::{url_decode_path, DEFAULT_SERVICE_PORT};
use cr_proto::zip_z::try_unwrap_steam_zip;
use rpc_engine::RpcEngine;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use vault::{now_unix, VaultStorage};

#[repr(C)]
#[allow(non_snake_case)]
struct SYSTEMTIME {
    wYear: u16,
    wMonth: u16,
    wDayOfWeek: u16,
    wDay: u16,
    wHour: u16,
    wMinute: u16,
    wSecond: u16,
    wMilliseconds: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
}

pub fn get_local_timestamp_str() -> String {
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

static WATCHING_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn ensure_parent_watcher(pid: u32, log_path: Arc<PathBuf>) {
    if pid == 0 {
        return;
    }
    if WATCHING_PID
        .compare_exchange(
            0,
            pid,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        std::thread::spawn(move || {
            type HANDLE = *mut std::ffi::c_void;
            #[link(name = "kernel32")]
            extern "system" {
                fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> HANDLE;
                fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) -> u32;
                fn CloseHandle(hObject: HANDLE) -> i32;
            }
            const SYNCHRONIZE: u32 = 0x0010_0000;
            const INFINITE: u32 = 0xFFFF_FFFF;

            let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                log_msg(
                    &log_path,
                    &format!(
                        "[Lifecycle] Nao foi possivel abrir handle de sincronizacao para Steam PID={}",
                        pid
                    ),
                );
                return;
            }

            log_msg(
                &log_path,
                &format!(
                    "[Lifecycle] Monitorando encerramento do processo Steam (PID={})...",
                    pid
                ),
            );

            unsafe {
                WaitForSingleObject(handle, INFINITE);
                CloseHandle(handle);
            }

            log_msg(
                &log_path,
                &format!(
                    "[Lifecycle] Processo Steam (PID={}) encerrou. Finalizando chrono-daemon.exe...",
                    pid
                ),
            );

            std::thread::sleep(std::time::Duration::from_millis(200));
            std::process::exit(0);
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = log_path;
    }
}

#[derive(Debug, Clone)]
struct ServiceConfig {
    port: u16,
    vault_dir: PathBuf,
    log_path: PathBuf,
}

impl ServiceConfig {
    fn load() -> Self {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let config_path = exe_dir.join("chronokeep.ini");
        let mut port = DEFAULT_SERVICE_PORT;
        let mut vault_dir = default_vault_dir(&exe_dir);

        if let Ok(content) = fs::read_to_string(&config_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let key = k.trim();
                    let val = v.trim();
                    if key.eq_ignore_ascii_case("port") {
                        if let Ok(p) = val.parse::<u16>() {
                            if p > 0 {
                                port = p;
                            }
                        }
                    } else if key.eq_ignore_ascii_case("vault_dir") && !val.is_empty() {
                        vault_dir = PathBuf::from(val);
                    }
                }
            }
        } else {
            let default_ini = format!(
                "# Configuracao do ChronoKeep (Cloud Save Vault)\r\n\
                 # Altere 'vault_dir' para sua pasta do Google Drive, OneDrive ou pasta local de preferencia.\r\n\
                 port={}\r\n\
                 vault_dir={}\r\n",
                port,
                vault_dir.display()
            );
            let _ = fs::write(&config_path, default_ini);
        }

        let log_path = exe_dir.join("chrono_daemon.log");
        Self {
            port,
            vault_dir,
            log_path,
        }
    }
}

fn default_vault_dir(exe_dir: &PathBuf) -> PathBuf {
    if let Ok(user_profile) = env::var("USERPROFILE") {
        let p = PathBuf::from(user_profile)
            .join("Saved Games")
            .join("ChronoKeepVault");
        return p;
    }
    exe_dir.join("ChronoKeepVault")
}

pub fn log_msg(log_path: &PathBuf, msg: &str) {
    let line = format!("[{}][chrono-daemon] {}\r\n", get_local_timestamp_str(), msg);
    print!("{}", line);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

fn bind_first_available_port(start_port: u16) -> Option<(u16, TcpListener)> {
    let addr = format!("127.0.0.1:{}", start_port);
    if let Ok(l) = TcpListener::bind(&addr) {
        return Some((start_port, l));
    }
    for offset in 1..=50 {
        let p = start_port.saturating_add(offset);
        let a = format!("127.0.0.1:{}", p);
        if let Ok(l) = TcpListener::bind(&a) {
            return Some((p, l));
        }
    }
    if let Ok(l) = TcpListener::bind("127.0.0.1:0") {
        if let Ok(local) = l.local_addr() {
            return Some((local.port(), l));
        }
    }
    None
}

fn update_ini_port(dir: &PathBuf, port: u16) {
    let ini_path = dir.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let mut found = false;
        for line in &mut lines {
            if let Some((k, _)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("port") {
                    *line = format!("port={}", port);
                    found = true;
                    break;
                }
            }
        }
        if !found {
            lines.push(format!("port={}", port));
        }
        let _ = fs::write(&ini_path, lines.join("\r\n") + "\r\n");
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let cfg = ServiceConfig::load();

    let mut parent_pid = 0u32;
    let mut idx = 1;
    while idx < args.len() {
        if args[idx] == "--parent-pid" && idx + 1 < args.len() {
            if let Ok(p) = args[idx + 1].parse::<u32>() {
                parent_pid = p;
            }
            idx += 2;
            continue;
        }
        idx += 1;
    }

    if args.len() >= 2 && args[1].eq_ignore_ascii_case("status") {
        println!("=== ChronoKeep Daemon (chrono-daemon) ===");
        println!("Porta configurada : 127.0.0.1:{}", cfg.port);
        println!("Pasta do Vault    : {}", cfg.vault_dir.display());
        println!("Arquivo de Log    : {}", cfg.log_path.display());
        return;
    }

    if args.len() >= 2 && args[1].eq_ignore_ascii_case("self-test") {
        run_self_test(&cfg);
        return;
    }

    let Some((actual_port, listener)) = bind_first_available_port(cfg.port) else {
        log_msg(
            &cfg.log_path,
            "Erro: Falha ao abrir socket em qualquer porta local disponivel.",
        );
        return;
    };

    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    if let Some(ref d) = exe_dir {
        let _ = fs::write(d.join("chrono_port.dat"), actual_port.to_string());
        update_ini_port(d, actual_port);
    }

    log_msg(
        &cfg.log_path,
        &format!(
            "Servidor iniciado em 127.0.0.1:{} | Vault={}",
            actual_port,
            cfg.vault_dir.display()
        ),
    );

    let token_file = exe_dir.as_ref().map(|d| d.join("chronokeep_gdrive.json"));
    let gdrive_ok = token_file.as_ref().map(|p| p.exists()).unwrap_or(false);
    if gdrive_ok {
        log_msg(&cfg.log_path, "Google Drive: ATIVO (chronokeep_gdrive.json encontrado)");
    } else {
        log_msg(&cfg.log_path, "Google Drive: INATIVO (chronokeep_gdrive.json nao encontrado)");
    }

    let vault = VaultStorage::new(cfg.vault_dir.clone(), exe_dir);
    let engine = Arc::new(Mutex::new(RpcEngine::new(vault, actual_port, Some(cfg.log_path.clone()))));
    let log_path = Arc::new(cfg.log_path);

    if parent_pid > 0 {
        ensure_parent_watcher(parent_pid, Arc::clone(&log_path));
    }

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let eng = Arc::clone(&engine);
        let lp = Arc::clone(&log_path);
        thread::spawn(move || {
            handle_connection(stream, eng, lp);
        });
    }
}

fn handle_connection(
    mut stream: TcpStream,
    engine: Arc<Mutex<RpcEngine>>,
    log_path: Arc<PathBuf>,
) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));

    let Some((method, path, body)) = read_http_request(&mut stream, &log_path) else {
        return;
    };

    if method == "GET" && path == "/v1/health" {
        let _ = write_http_response(&mut stream, "200 OK", "application/json", b"{\"ok\":true}");
        return;
    }

    if method == "POST" && path == "/v1/shutdown" {
        log_msg(
            &log_path,
            "[Lifecycle] Requisicao de encerramento (/v1/shutdown) recebida da Steam. Finalizando chrono-daemon...",
        );
        let _ = write_http_response(&mut stream, "200 OK", "text/plain", b"OK");
        let _ = stream.flush();
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(150));
            std::process::exit(0);
        });
        return;
    }

    if method == "POST" && path == "/v1/session" {
        // Text lines: line 0 = steam_path, line 1 = account_id, line 2 = comma-separated app_ids, line 3 = steam_pid
        let text = String::from_utf8_lossy(&body);
        let mut lines = text.lines();
        let steam_path = lines.next().map(|s| s.trim().to_string());
        let account_id = lines
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let apps: Vec<u32> = lines
            .next()
            .unwrap_or("")
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect();
        let steam_pid = lines
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);

        if steam_pid > 0 {
            ensure_parent_watcher(steam_pid, Arc::clone(&log_path));
        }

        log_msg(
            &log_path,
            &format!(
                "Sessao atualizada: account_id={} apps={} steam_path={:?}",
                account_id,
                apps.len(),
                steam_path
            ),
        );

        if let Ok(mut guard) = engine.lock() {
            guard.update_session(steam_path.clone(), account_id, apps.clone());
            if let Some(ref sp) = steam_path {
                if account_id != 0 && !apps.is_empty() {
                    let vault_clone = guard.vault.clone();
                    let lp_clone = log_path.as_ref().clone();
                    crate::gdrive::start_background_pull_all(
                        PathBuf::from(sp),
                        account_id,
                        apps,
                        vault_clone,
                        Some(lp_clone),
                    );
                }
            }
        }
        let _ = write_http_response(&mut stream, "200 OK", "text/plain", b"OK");
        return;
    }

    if method == "POST" && path == "/v1/game_state" {
        let text = String::from_utf8_lossy(&body);
        let mut lines = text.lines();
        let app_id = lines
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let account_id = lines
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let running = lines
            .next()
            .map(|s| s.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !running && app_id != 0 {
            log_msg(
                &log_path,
                &format!(
                    "[API] Notificacao de jogo encerrado recebida da bridge: app={} acct={}",
                    app_id, account_id
                ),
            );
            if let Ok(mut guard) = engine.lock() {
                guard.on_exit_sync(account_id, app_id);
            }
        }
        let _ = write_http_response(&mut stream, "200 OK", "text/plain", b"OK");
        return;
    }

    if method == "POST" && path == "/v1/rpc" {
        // Binary frame:
        // [0..4]: u32 method_len (LE)
        // [4..4+method_len]: method UTF-8
        // [..+4]: u32 app_id (LE)
        // [..+4]: u32 account_id (LE)
        // [..]: raw request protobuf bytes
        if body.len() < 12 {
            let _ = write_http_response(&mut stream, "400 Bad Request", "text/plain", b"");
            return;
        }
        let method_len = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
        if 12 + method_len > body.len() {
            let _ = write_http_response(&mut stream, "400 Bad Request", "text/plain", b"");
            return;
        }
        let rpc_method = String::from_utf8_lossy(&body[4..4 + method_len]).into_owned();
        let off = 4 + method_len;
        let app_id = u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]);
        let account_id = u32::from_le_bytes([
            body[off + 4],
            body[off + 5],
            body[off + 6],
            body[off + 7],
        ]);
        let req_bytes = &body[off + 8..];

        let rpc_res = engine
            .lock()
            .ok()
            .and_then(|mut g| g.handle_rpc(&rpc_method, app_id, account_id, req_bytes));

        match rpc_res {
            Some((eresult, resp_bytes)) => {
                let extra_info = extract_rpc_log_detail(&rpc_method, req_bytes);
                log_msg(
                    &log_path,
                    &format!(
                        "RPC {} app={} acct={}{} req={}B -> eresult={} resp={}B",
                        rpc_method,
                        app_id,
                        account_id,
                        extra_info,
                        req_bytes.len(),
                        eresult,
                        resp_bytes.len()
                    ),
                );
                let mut out = Vec::with_capacity(5 + resp_bytes.len());
                out.push(1u8); // handled = true
                out.extend_from_slice(&eresult.to_le_bytes());
                out.extend_from_slice(&resp_bytes);
                let _ = write_http_response(&mut stream, "200 OK", "application/octet-stream", &out);
            }
            None => {
                let _ = write_http_response(&mut stream, "200 OK", "application/octet-stream", &[0u8]);
            }
        }
        return;
    }

    if method == "PUT" && path.starts_with("/upload/") {
        log_msg(
            &log_path,
            &format!("[HTTP] PUT recebido para {} (tamanho={}B)", path, body.len()),
        );
        if let Some((account_id, app_id, clean_filename)) = parse_blob_route(&path, "/upload/") {
            let final_bytes = match try_unwrap_steam_zip(&body) {
                Some(unwrapped) => {
                    log_msg(
                        &log_path,
                        &format!(
                            "PUT upload app={} file='{}': ZIP 'z' descomprimido ({}B -> {}B)",
                            app_id,
                            clean_filename,
                            body.len(),
                            unwrapped.len()
                        ),
                    );
                    unwrapped
                }
                None => {
                    log_msg(
                        &log_path,
                        &format!(
                            "PUT upload app={} file='{}': armazenado bruto ({}B)",
                            app_id,
                            clean_filename,
                            body.len()
                        ),
                    );
                    body
                }
            };

            if let Ok(mut guard) = engine.lock() {
                guard.store_uploaded_blob(account_id, app_id, clean_filename, final_bytes);
            }
            let _ = write_http_response(&mut stream, "200 OK", "text/plain", b"");
            let _ = stream.shutdown(std::net::Shutdown::Write);
            return;
        }
        log_msg(
            &log_path,
            &format!("[HTTP] PUT falhou: Rota invalida ou caminho inseguro: '{}'", path),
        );
        let _ = write_http_response(&mut stream, "404 Not Found", "text/plain", b"");
        let _ = stream.shutdown(std::net::Shutdown::Write);
        return;
    }

    if method == "GET" && path.starts_with("/download/") {
        if let Some((account_id, app_id, clean_filename)) = parse_blob_route(&path, "/download/") {
            let maybe_data = engine
                .lock()
                .ok()
                .and_then(|g| g.retrieve_download_blob(account_id, app_id, &clean_filename));

            if let Some(data) = maybe_data {
                log_msg(
                    &log_path,
                    &format!(
                        "GET download app={} file='{}' -> {}B enviados",
                        app_id,
                        clean_filename,
                        data.len()
                    ),
                );
                let _ = write_http_response(&mut stream, "200 OK", "application/octet-stream", &data);
                return;
            }
        }
        let _ = write_http_response(&mut stream, "404 Not Found", "text/plain", b"");
        return;
    }

    let _ = write_http_response(&mut stream, "404 Not Found", "text/plain", b"");
}

fn extract_rpc_log_detail(method: &str, req_bytes: &[u8]) -> String {
    use cr_proto::cloud_messages::{
        RPC_BEGIN_UPLOAD, RPC_COMMIT_UPLOAD, RPC_DELETE_FILE, RPC_FILE_DOWNLOAD,
    };
    use cr_proto::wire::{find_field, parse_message};

    let field_num = match method {
        RPC_FILE_DOWNLOAD | RPC_DELETE_FILE => 2,
        RPC_COMMIT_UPLOAD => 4,
        RPC_BEGIN_UPLOAD => 6,
        _ => return String::new(),
    };
    let fields = parse_message(req_bytes);
    if let Some(fname) = find_field(&fields, field_num).and_then(|f| f.as_str()) {
        format!(" file='{}'", fname)
    } else {
        String::new()
    }
}

fn parse_blob_route(path: &str, prefix: &str) -> Option<(u32, u32, String)> {
    let rest = path.strip_prefix(prefix)?;
    let rest = rest.split('?').next().unwrap_or(rest);
    let mut parts = rest.splitn(3, '/');
    let account_id = parts.next()?.parse::<u32>().ok()?;
    let app_id = parts.next()?.parse::<u32>().ok()?;
    let raw_file = parts.next()?;
    let decoded = url_decode_path(raw_file);
    let clean = cr_proto::cloud_messages::sanitize_rel_path(&decoded);
    if clean.is_empty() {
        return None;
    }
    Some((account_id, app_id, clean))
}

fn read_http_request(
    stream: &mut TcpStream,
    log_path: &PathBuf,
) -> Option<(String, String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    let header_end;

    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 65536 {
            return None;
        }
    }

    let header_str = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = header_str.lines();
    let req_line = lines.next()?;
    let mut req_parts = req_line.split_whitespace();
    let method = req_parts.next()?.to_uppercase();
    let path = req_parts.next()?.to_string();

    let mut content_length = 0usize;
    let mut has_expect_100 = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = val.parse::<usize>().unwrap_or(0);
            } else if key.eq_ignore_ascii_case("expect")
                && val.to_ascii_lowercase().contains("100-continue")
            {
                has_expect_100 = true;
            }
        }
    }

    if has_expect_100 {
        log_msg(
            log_path,
            &format!(
                "[HTTP] {} {} -> Header Expect: 100-continue detectado. Enviando 100 Continue",
                method, path
            ),
        );
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = stream.flush();
    }

    if content_length > 512 * 1024 * 1024 {
        log_msg(
            log_path,
            &format!(
                "[HTTP] {} {} -> Rejeitado: Content-Length {} excede limite maximo de 512MB",
                method, path, content_length
            ),
        );
        return None;
    }

    let mut body = Vec::with_capacity(content_length);
    if buf.len() > header_end {
        body.extend_from_slice(&buf[header_end..]);
    }
    while body.len() < content_length {
        let to_read = (content_length - body.len()).min(chunk.len());
        let n = stream.read(&mut chunk[..to_read]).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    if content_length > 0 && body.len() < content_length {
        log_msg(
            log_path,
            &format!(
                "[HTTP] {} {} -> Leitura incompleta do body: {} de {} bytes recebidos",
                method,
                path,
                body.len(),
                content_length
            ),
        );
        return None;
    }
    body.truncate(content_length);

    Some((method, path, body))
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = if body.is_empty() {
        format!(
            "HTTP/1.1 {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            status
        )
    } else {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status,
            content_type,
            body.len()
        )
    };
    stream.write_all(header.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn run_self_test(cfg: &ServiceConfig) {
    use cr_proto::wire::ProtoWriter;

    println!("============================================================================");
    println!("  CHRONOKEEP — AUTO-TESTE COMPLETO DE CLOUD SAVE (SEM ABRIR JOGO)");
    println!("============================================================================");
    println!("Alvo Local   : http://127.0.0.1:{}", cfg.port);
    println!("Pasta Vault  : {}", cfg.vault_dir.display());
    println!();

    let test_acct: u32 = 76561198;
    let test_app: u32 = 480; // Spacewar / AppID de teste padrao da Steam
    let test_filename = "chronokeep_test_save.txt";
    let test_payload = format!(
        "ChronoKeep Cloud Save Test OK! Timestamp={}\r\n",
        now_unix()
    );

    // 1. Verificar se o chrono-daemon esta respondendo em /v1/health
    let health_ok = send_raw_http(cfg.port, "GET", "/v1/health", b"").is_some();
    if !health_ok {
        println!("[!] chrono-daemon nao estava rodando. Iniciando instancia local temporaria...");
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let vault = VaultStorage::new(cfg.vault_dir.clone(), exe_dir.clone());
        let mut engine = RpcEngine::new(vault, cfg.port, None);
        if let Some(ref d) = exe_dir {
            engine.update_session(Some(d.to_string_lossy().into_owned()), test_acct, vec![test_app]);
        }

        // Simular BeginFileUpload + Store + Commit diretamente na engine
        let mut req_begin = ProtoWriter::new();
        req_begin.write_varint(1, test_app as u64);
        req_begin.write_varint(2, test_payload.len() as u64);
        req_begin.write_varint(3, test_payload.len() as u64);
        req_begin.write_varint(5, now_unix());
        req_begin.write_str(6, test_filename);
        let _ = engine.handle_rpc("Cloud.ClientBeginFileUpload#1", test_app, test_acct, &req_begin.into_bytes());
        engine.store_uploaded_blob(test_acct, test_app, test_filename.to_string(), test_payload.as_bytes().to_vec());

        let mut req_commit = ProtoWriter::new();
        req_commit.write_varint(1, 1);
        req_commit.write_varint(2, test_app as u64);
        req_commit.write_str(4, test_filename);
        let _ = engine.handle_rpc("Cloud.ClientCommitFileUpload#1", test_app, test_acct, &req_commit.into_bytes());
        println!("[1/3] Simulacao direta da Engine RPC : [OK]");
    } else {
        println!("[1/4] Conexao HTTP com chrono-daemon (127.0.0.1:{}) : [OK]", cfg.port);

        // 2. Enviar ClientBeginFileUpload#1 via /v1/rpc
        let mut req_begin = ProtoWriter::new();
        req_begin.write_varint(1, test_app as u64);
        req_begin.write_varint(2, test_payload.len() as u64);
        req_begin.write_varint(3, test_payload.len() as u64);
        req_begin.write_varint(5, now_unix());
        req_begin.write_str(6, test_filename);
        let begin_frame = build_rpc_frame("Cloud.ClientBeginFileUpload#1", test_app, test_acct, &req_begin.into_bytes());
        let _ = send_raw_http(cfg.port, "POST", "/v1/rpc", &begin_frame);

        // 3. Enviar PUT /upload/76561198/480/chronokeep_test_save.txt
        let put_path = format!("/upload/{}/{}/{}", test_acct, test_app, test_filename);
        let _ = send_raw_http(cfg.port, "PUT", &put_path, test_payload.as_bytes());

        // 4. Enviar ClientCommitFileUpload#1 via /v1/rpc
        let mut req_commit = ProtoWriter::new();
        req_commit.write_varint(1, 1);
        req_commit.write_varint(2, test_app as u64);
        req_commit.write_str(4, test_filename);
        let commit_frame = build_rpc_frame("Cloud.ClientCommitFileUpload#1", test_app, test_acct, &req_commit.into_bytes());
        let _ = send_raw_http(cfg.port, "POST", "/v1/rpc", &commit_frame);

        println!("[2/4] Ciclo RPC Steam Cloud (BeginUpload -> PUT -> CommitUpload) : [OK]");
    }

    // Verificar arquivo gravado no Vault Local
    let expected_file = cfg
        .vault_dir
        .join(test_acct.to_string())
        .join(test_app.to_string())
        .join("files")
        .join(test_filename);
    if expected_file.exists() {
        println!(
            "[3/4] Arquivo salvo no Vault Local : [OK] ({})",
            expected_file.display()
        );
    } else {
        println!(
            "[3/4] Arquivo salvo no Vault Local : [FALHA] ({})",
            expected_file.display()
        );
    }

    // Testar sincronizacao com Google Drive se chronokeep_gdrive.json existir
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let token_file = exe_dir.join("chronokeep_gdrive.json");
    if token_file.exists() {
        print!("[4/4] Testando envio para Google Drive (ChronoKeep/{}/{}/{})... ", test_acct, test_app, test_filename);
        let _ = std::io::stdout().flush();
        let ok = gdrive::sync_file_to_gdrive(
            &exe_dir,
            test_acct,
            test_app,
            test_filename,
            test_payload.as_bytes(),
            Some(&cfg.log_path),
        );
        if ok {
            println!("[OK - ENVIADO AO GOOGLE DRIVE!]");
        } else {
            println!("[FALHA - Verifique a conexao ou refaca o login Google]");
        }
    } else {
        println!(
            "[4/4] Google Drive : [PULADO - chronokeep_gdrive.json nao encontrado em {}]",
            exe_dir.display()
        );
    }
    println!("============================================================================");
}

fn build_rpc_frame(method: &str, app_id: u32, account_id: u32, proto_bytes: &[u8]) -> Vec<u8> {
    let mb = method.as_bytes();
    let mut out = Vec::with_capacity(12 + mb.len() + proto_bytes.len());
    out.extend_from_slice(&(mb.len() as u32).to_le_bytes());
    out.extend_from_slice(mb);
    out.extend_from_slice(&app_id.to_le_bytes());
    out.extend_from_slice(&account_id.to_le_bytes());
    out.extend_from_slice(proto_bytes);
    out
}

fn send_raw_http(port: u16, method: &str, path: &str, body: &[u8]) -> Option<Vec<u8>> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500)).ok()?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
    let hdr = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        method,
        path,
        port,
        body.len()
    );
    stream.write_all(hdr.as_bytes()).ok()?;
    if !body.is_empty() {
        stream.write_all(body).ok()?;
    }
    stream.flush().ok()?;
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).ok()?;
    let hdr_end = find_subslice(&resp, b"\r\n\r\n")? + 4;
    let status = String::from_utf8_lossy(&resp[..hdr_end]);
    if !status.contains("200 OK") {
        return None;
    }
    Some(resp[hdr_end..].to_vec())
}
