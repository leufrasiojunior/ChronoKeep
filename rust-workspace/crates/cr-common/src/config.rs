//! System configuration, port discovery, and directory detection utilities.

pub use cr_proto::cloud_messages::DEFAULT_SERVICE_PORT;
use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_service_port(steam_path: &str) -> u16 {
    let base = Path::new(steam_path);

    // 1. Tenta chrono_port.dat (gravado dinamicamente pelo daemon com a porta real)
    let port_dat = base.join("chrono_port.dat");
    if let Ok(content) = fs::read_to_string(&port_dat) {
        if let Ok(p) = content.trim().parse::<u16>() {
            if p > 0 {
                return p;
            }
        }
    }

    // 2. Tenta chronokeep.ini ou cr_clone_config.ini
    let candidates = ["chronokeep.ini", "cr_clone_config.ini"];
    for name in candidates {
        let ini_path = base.join(name);
        if let Ok(content) = fs::read_to_string(ini_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim().eq_ignore_ascii_case("port") {
                        if let Ok(p) = v.trim().parse::<u16>() {
                            if p > 0 {
                                return p;
                            }
                        }
                    }
                }
            }
        }
    }

    DEFAULT_SERVICE_PORT
}

pub fn detect_default_steam_dir() -> String {
    let candidates = [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
        r"D:\Steam",
        r"E:\Steam",
    ];
    for p in candidates {
        if Path::new(p).exists() {
            return p.to_string();
        }
    }
    r"C:\Program Files (x86)\Steam".to_string()
}

pub fn detect_default_vault_dir(exe_dir: Option<&Path>) -> PathBuf {
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(user_profile)
            .join("Saved Games")
            .join("ChronoKeepVault");
        return p;
    }
    exe_dir
        .map(|d| d.join("ChronoKeepVault"))
        .unwrap_or_else(|| PathBuf::from("ChronoKeepVault"))
}

pub fn is_gdrive_connected(steam_dir: &Path) -> bool {
    let token_file = steam_dir.join("chronokeep_gdrive.json");
    if token_file.exists() {
        return true;
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            return parent.join("chronokeep_gdrive.json").exists();
        }
    }
    false
}
