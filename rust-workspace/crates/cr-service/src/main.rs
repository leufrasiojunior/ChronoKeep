//! `chrono-daemon.exe` — External Cloud Save Daemon & Local HTTP Block Server.
//! Listens on `127.0.0.1:<port>` (default `39771`) to serve both:
//! 1. RPC forwarding from `chronokeep.dll` (`POST /v1/rpc`, `POST /v1/session`, `GET /v1/health`)
//! 2. Native Steam Cloud file transfers from `steamclient64.dll` (`PUT /upload/...`, `GET /download/...`)

mod config;
mod gdrive;
mod lifecycle;
mod rpc_engine;
mod server;
mod vault;

use config::{bind_first_available_port, update_ini_port, ServiceConfig};
use lifecycle::{ensure_parent_watcher, log_msg};
use rpc_engine::RpcEngine;
use server::{handle_connection, run_self_test};
use std::env;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use vault::VaultStorage;

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
