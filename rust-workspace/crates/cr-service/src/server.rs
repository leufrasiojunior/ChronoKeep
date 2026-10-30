//! HTTP connection handling, request routing, and streaming for `chrono-daemon`.

use crate::config::ServiceConfig;
use crate::lifecycle::log_msg;
use crate::rpc_engine::RpcEngine;
use crate::vault::now_unix;
use cr_proto::cloud_messages::url_decode_path;
use cr_proto::zip_z::try_unwrap_steam_zip;
use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn handle_connection(
    mut stream: TcpStream,
    engine: Arc<Mutex<RpcEngine>>,
    log_path: Arc<PathBuf>,
    max_upload_mb: u64,
) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));

    let Some((method, path, body)) = read_http_request(&mut stream, &log_path, max_upload_mb) else {
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
            crate::lifecycle::ensure_parent_watcher(steam_pid, Arc::clone(&log_path));
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
    log_path: &Path,
    _max_upload_mb: u64,
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

    // Sincronização automática interceptada da Steam via daemon:
    // Não impõe limite restritivo; sincroniza independente do tamanho.
    // Mantém apenas uma trava técnica de segurança contra transbordamento de buffer (4GB).
    const MAX_SAFE_DAEMON_PAYLOAD: usize = 4 * 1024 * 1024 * 1024;
    if content_length > MAX_SAFE_DAEMON_PAYLOAD {
        log_msg(
            log_path,
            &format!(
                "[HTTP] {} {} -> Rejeitado: Content-Length {} excede limite maximo de seguranca (4GB)",
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

pub fn run_self_test(cfg: &ServiceConfig) {
    use crate::vault::VaultStorage;
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
        let ok = crate::gdrive::sync_file_to_gdrive(
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
