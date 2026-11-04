//! Lightweight HTTP/TCP client in `cloud_redirect.dll` that communicates with `cr-service.exe`
//! over `127.0.0.1:<port>` and auto-spawns `cr-service.exe` in the background if not running.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
}

#[cfg(not(target_os = "windows"))]
unsafe fn GetCurrentProcessId() -> u32 {
    0
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub use cr_common::config::{check_daemon_health as check_health, resolve_service_port};

pub fn ensure_service_running(steam_path: &str, port: u16) -> bool {
    if check_health(port) {
        crate::log_debug(&format!("[IPC] Daemon ja ativo e saudavel na porta {}", port));
        return true;
    }

    crate::log_debug(&format!(
        "[IPC] Daemon inativo na porta {}. Procurando executaveis em '{}'...",
        port, steam_path
    ));

    // Try spawning <steam_path>\chrono-daemon.exe in background without console window
    let candidates = ["chrono-daemon.exe", "cr-service.exe"];
    for bin_name in candidates {
        let exe_path: PathBuf = Path::new(steam_path).join(bin_name);
        if exe_path.exists() {
            let mut cmd = std::process::Command::new(&exe_path);
            cmd.current_dir(steam_path);
            let my_pid = unsafe { GetCurrentProcessId() };
            if my_pid != 0 {
                cmd.arg("--parent-pid").arg(my_pid.to_string());
            }
            #[cfg(target_os = "windows")]
            {
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            crate::log_debug(&format!(
                "[IPC] Iniciando processo: {} (parent_pid={})",
                exe_path.display(),
                my_pid
            ));
            if cmd.spawn().is_ok() {
                for i in 0..25 {
                    std::thread::sleep(Duration::from_millis(60));
                    let current_port = resolve_service_port(steam_path);
                    if check_health(current_port) || check_health(port) {
                        crate::log_debug(&format!(
                            "[IPC] Daemon inicializado com sucesso apos {}ms na porta {}",
                            (i + 1) * 60,
                            if check_health(current_port) { current_port } else { port }
                        ));
                        return true;
                    }
                }
            }
        }
    }

    let current_port = resolve_service_port(steam_path);
    let ok = check_health(current_port) || check_health(port);
    crate::log_debug(&format!(
        "[IPC] Checagem final de saude do daemon (porta={}, resolvida={}): {}",
        port, current_port, if ok { "SUCESSO" } else { "FALHA" }
    ));
    ok
}

pub fn push_session_update(
    port: u16,
    steam_path: &str,
    account_id: u32,
    apps: &[u32],
) -> bool {
    let my_pid = unsafe { GetCurrentProcessId() };
    let apps_csv = apps
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let payload = format!("{}\n{}\n{}\n{}\n", steam_path, account_id, apps_csv, my_pid);
    crate::log_debug(&format!(
        "[IPC] push_session_update: porta={}, acct={}, apps=[{}], pid={}",
        port, account_id, apps_csv, my_pid
    ));
    http_post(port, "/v1/session", payload.as_bytes(), Duration::from_secs(2)).is_some()
}

pub fn notify_shutdown(port: u16) {
    crate::log_debug(&format!("[IPC] Enviando /v1/shutdown na porta {}", port));
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(150)) {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
        let req = format!(
            "POST /v1/shutdown HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            port
        );
        let _ = stream.write_all(req.as_bytes());
        let _ = stream.flush();
    }
}

pub fn push_game_state(port: u16, app_id: u32, account_id: u32, running: bool) -> bool {
    crate::log_debug(&format!(
        "[IPC] push_game_state: app={}, acct={}, running={}",
        app_id, account_id, running
    ));
    let payload = format!("{}\n{}\n{}\n", app_id, account_id, running);
    http_post(port, "/v1/game_state", payload.as_bytes(), Duration::from_secs(2)).is_some()
}

pub enum RpcForwardResult {
    Handled(i32, Vec<u8>),
    UnhandledPassthrough,
    TransportError,
}

/// Forwards a `Cloud.*` RPC call to `cr-service.exe` and distinguishes handled responses,
/// explicit passthrough (`0u8`), and daemon transport failures.
pub fn forward_rpc(
    port: u16,
    steam_path: &str,
    method: &str,
    app_id: u32,
    account_id: u32,
    req_bytes: &[u8],
) -> RpcForwardResult {
    let method_bytes = method.as_bytes();
    let mut payload = Vec::with_capacity(12 + method_bytes.len() + req_bytes.len());
    payload.extend_from_slice(&(method_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(method_bytes);
    payload.extend_from_slice(&app_id.to_le_bytes());
    payload.extend_from_slice(&account_id.to_le_bytes());
    payload.extend_from_slice(req_bytes);

    let timeout = if method == cr_proto::cloud_messages::RPC_COMPLETE_BATCH {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(14)
    };

    crate::log_debug(&format!(
        "[IPC] Enviando RPC {} para app={} acct={} (porta={}, req={}B, timeout={:?})",
        method, app_id, account_id, port, req_bytes.len(), timeout
    ));

    let start_t = std::time::Instant::now();
    let resp_body = match http_post(port, "/v1/rpc", &payload, timeout) {
        Some(b) => b,
        None => {
            crate::log_debug(&format!(
                "[IPC] Daemon em 127.0.0.1:{} nao respondeu; tentando restabelecer servico...",
                port
            ));
            // If cr-service was closed mid-session, auto-respawn once and retry
            if ensure_service_running(steam_path, port) {
                let resolved_port = resolve_service_port(steam_path);
                crate::log_debug(&format!(
                    "[IPC] Servico recuperado. Reenviando RPC {} para 127.0.0.1:{}",
                    method, resolved_port
                ));
                match http_post(resolved_port, "/v1/rpc", &payload, timeout) {
                    Some(b) => b,
                    None => {
                        crate::log_debug(&format!(
                            "[IPC] ERRO: Reenvio de RPC {} para porta {} tambem falhou",
                            method, resolved_port
                        ));
                        return RpcForwardResult::TransportError;
                    }
                }
            } else {
                crate::log_debug(&format!(
                    "[IPC] ERRO: Falha ao restabelecer daemon para RPC {}",
                    method
                ));
                return RpcForwardResult::TransportError;
            }
        }
    };

    let elapsed = start_t.elapsed();
    if resp_body.is_empty() {
        crate::log_debug(&format!(
            "[IPC] Resposta da RPC {} vazia (0B) apos {:?}",
            method, elapsed
        ));
        return RpcForwardResult::TransportError;
    }
    if resp_body[0] == 0 {
        crate::log_debug(&format!(
            "[IPC] Resposta da RPC {}: daemon indicou passthrough explicito (handled=0) em {:?}",
            method, elapsed
        ));
        return RpcForwardResult::UnhandledPassthrough;
    }
    if resp_body.len() < 5 {
        crate::log_debug(&format!(
            "[IPC] Resposta da RPC {} invalida (tamanho {}B < 5B)",
            method, resp_body.len()
        ));
        return RpcForwardResult::TransportError;
    }
    let eresult = i32::from_le_bytes([resp_body[1], resp_body[2], resp_body[3], resp_body[4]]);
    let proto_out = resp_body[5..].to_vec();
    crate::log_debug(&format!(
        "[IPC] RPC {} concluida com sucesso em {:?}: eresult={}, proto_bytes={}B",
        method, elapsed, eresult, proto_out.len()
    ));
    RpcForwardResult::Handled(eresult, proto_out)
}

fn http_post(port: u16, path: &str, body: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let header = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        path,
        port,
        body.len()
    );
    stream.write_all(header.as_bytes()).ok()?;
    if !body.is_empty() {
        stream.write_all(body).ok()?;
    }
    stream.flush().ok()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let status_str = String::from_utf8_lossy(&raw[..header_end]);
    if !status_str.contains("200 OK") {
        return None;
    }
    Some(raw[header_end..].to_vec())
}

