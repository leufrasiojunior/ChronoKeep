//! Configuration management and local port allocation for `chrono-daemon`.

use cr_common::config::{detect_default_vault_dir, DEFAULT_SERVICE_PORT};
use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub port: u16,
    pub vault_dir: PathBuf,
    pub log_path: PathBuf,
    pub max_upload_mb: u64,
    pub debug_mode: bool,
}

impl ServiceConfig {
    pub fn load() -> Self {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let config_path = exe_dir.join("chronokeep.ini");
        let mut port = DEFAULT_SERVICE_PORT;
        let mut vault_dir = detect_default_vault_dir(Some(&exe_dir));
        let mut max_upload_mb = 512u64;
        let mut debug_mode = false;

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
                    } else if key.eq_ignore_ascii_case("max_upload_mb") {
                        if let Ok(mb) = val.parse::<u64>() {
                            if mb > 0 {
                                max_upload_mb = mb;
                            }
                        }
                    } else if key.eq_ignore_ascii_case("debug") {
                        debug_mode = val.eq_ignore_ascii_case("true") || val == "1";
                    }
                }
            }
        } else {
            let default_ini = format!(
                "# Configuracao do ChronoKeep (Cloud Save Vault)\r\n\
                 # Altere 'vault_dir' para sua pasta do Google Drive, OneDrive ou pasta local de preferencia.\r\n\
                 port={}\r\n\
                 vault_dir={}\r\n\
                 max_upload_mb={}\r\n\
                 debug=false\r\n",
                port,
                vault_dir.display(),
                max_upload_mb
            );
            let _ = fs::write(&config_path, default_ini);
        }

        crate::lifecycle::set_debug_mode(debug_mode);

        let log_path = exe_dir.join("chrono_daemon.log");
        Self {
            port,
            vault_dir,
            log_path,
            max_upload_mb,
            debug_mode,
        }
    }
}

pub fn bind_first_available_port(start_port: u16) -> Option<(u16, TcpListener)> {
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

pub fn update_ini_port(dir: &Path, port: u16) {
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
