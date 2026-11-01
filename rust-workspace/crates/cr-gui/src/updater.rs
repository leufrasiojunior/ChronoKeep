//! Automatic update verification and deployment for ChronoKeep components.
//! Supports decoupled updates of ChronoKeep (GUI), chrono-daemon, and chronokeep.dll.

use crate::i18n::Lang;
use crate::installer::deploy_bytes_safe;
use crate::service_mgr::{
    close_steam_and_wait, is_process_running, kill_process_and_wait, launch_steam,
    start_service_from_gui, stop_service_from_gui,
};
use cr_common::config::resolve_service_port;
use cr_common::http::https_download_url;
use cr_proto::sha256::{sha256_file, sha256_hex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[cfg(target_os = "windows")]
use crate::win32::CREATE_NO_WINDOW;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const PRIMARY_VERSIONS_URL: &str =
    "https://raw.githubusercontent.com/leufrasiojunior/ChronoKeep/main/versions.json";
const FALLBACK_VERSIONS_URL: &str =
    "https://raw.githubusercontent.com/leufrasiojunior/ChronoKeep/develop/versions.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ManifestComponent {
    pub filename: String,
    pub sha256: String,
    pub download_url: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RemoteManifest {
    pub release_version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    pub components: HashMap<String, ManifestComponent>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ComponentUpdateInfo {
    pub id: String,               // "gui", "daemon", "bridge"
    pub name: String,             // "ChronoKeep (Painel)", "chrono-daemon (Serviço)", "chronokeep.dll (Bridge Steam)"
    pub current_version: String,  // e.g. "v0.1.2", "Instalado", "Não Instalado"
    pub latest_version: String,   // e.g. "v0.1.3"
    pub current_hash: String,     // Local SHA-256
    pub latest_hash: String,      // Remote SHA-256
    pub status: String,           // "up_to_date", "update_available", "not_installed", "error"
    pub download_url: String,
    pub filename: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateCheckResult {
    pub success: bool,
    pub message: String,
    pub release_tag: String,
    pub has_updates: bool,
    pub components: Vec<ComponentUpdateInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateApplyResult {
    pub success: bool,
    pub message: String,
    pub component_id: String,
    pub restart_required: bool,
}

pub fn fetch_remote_manifest(lang: Lang) -> Result<RemoteManifest, String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let primary_url = format!("{}?t={}", PRIMARY_VERSIONS_URL, ts);
    let fallback_url = format!("{}?t={}", FALLBACK_VERSIONS_URL, ts);

    // Tenta URL primária do release com bypass de cache CDN
    let res = https_download_url(&primary_url, 15_000)
        .or_else(|| https_download_url(&fallback_url, 15_000));

    let (status, bytes) = res.ok_or_else(|| {
        if lang.is_pt() {
            "Não foi possível conectar ao servidor de versões do GitHub. Verifique sua conexão com a internet.".to_string()
        } else {
            "Could not connect to the GitHub version server. Please check your internet connection.".to_string()
        }
    })?;

    if status != 200 {
        return Err(if lang.is_pt() {
            format!("Servidor de atualização retornou HTTP {}.", status)
        } else {
            format!("Update server returned HTTP {}.", status)
        });
    }

    serde_json::from_slice::<RemoteManifest>(&bytes).map_err(|e| {
        if lang.is_pt() {
            format!("Falha ao decodificar manifesto de versões: {}", e)
        } else {
            format!("Failed to decode version manifest: {}", e)
        }
    })
}

pub fn sanitize_windows_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s
    }
}

fn extract_marker_version(path: &Path, prefix: &[u8]) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if let Some(pos) = bytes.windows(prefix.len()).position(|w| w == prefix) {
        let after = &bytes[pos + prefix.len()..];
        let mut ver_bytes = Vec::new();
        for &b in after {
            if b == 0 || b == b'\r' || b == b'\n' || b == b' ' {
                break;
            }
            if b.is_ascii_graphic() {
                ver_bytes.push(b);
            } else {
                break;
            }
        }
        if !ver_bytes.is_empty() {
            if let Ok(s) = String::from_utf8(ver_bytes) {
                let clean = s.trim();
                if !clean.is_empty() {
                    return Some(if clean.starts_with('v') || clean.starts_with('V') {
                        clean.to_string()
                    } else {
                        format!("v{}", clean)
                    });
                }
            }
        }
    }
    None
}

pub fn detect_daemon_version(daemon_path: &Path, remote_hash: &str, target_ver: &str, lang: Lang) -> String {
    if !daemon_path.exists() {
        return if lang.is_pt() {
            "Não instalado".to_string()
        } else {
            "Not installed".to_string()
        };
    }
    // 1. Tenta extrair a versão pelo marcador embutido no binário
    if let Some(v) = extract_marker_version(daemon_path, b"CK_DAEMON_VER:") {
        return v;
    }

    let cur_hash = sha256_file(daemon_path).unwrap_or_default().to_lowercase();
    if !cur_hash.is_empty() && !remote_hash.is_empty() && cur_hash == remote_hash {
        return target_ver.to_string();
    }
    let embedded_hash = sha256_hex(crate::commands::EMBEDDED_CHRONO_DAEMON).to_lowercase();
    if !cur_hash.is_empty() && cur_hash == embedded_hash {
        return format!("v{}", env!("CARGO_PKG_VERSION"));
    }
    match cur_hash.as_str() {
        "466d7d2b51be232ec37d38dbb96f8809925722940d37ae0c85d9e4ef28a01ab3" => "v0.1.8".to_string(),
        "737f94f094f83385d6cc51a6ccfda32432b989eef25d3c57f667facff0993328" => "v0.1.8".to_string(),
        "4952f719b17bb13a90322524a64d400748d999ba34c7abfe9bdfff22ea6e037a" => "v0.1.7".to_string(),
        "bd3e9c3e9e16dbd6207dbd1bf66ac13aa8fd2c6a28f155ffa4bf006d1288b2bc" => "v0.1.6".to_string(),
        "67adff9ae4589b8060a9cd17d0a6046e7a1907ab71b59f2527ed5f6427e5a850" => "v0.1.5".to_string(),
        "24de7fd18a865a3b2b3c6a3220d511ed12b16d2011139a88acd0454860461e48" => "v0.1.5".to_string(),
        "011328eac03fd39a6fa1500649552499c717d27692868c77969fd179d7adbeb3" => "v0.1.4".to_string(),
        "2e11a572a133710fb7a550072d1171a79f18efa96a46d1961bc1eb222fca4b94" => "v0.1.4".to_string(),
        "d7c6a8dd046aaf50d6ffa4cb4935c4b32e110c435eafabd4c318470fa462f57e" => "v0.1.3".to_string(),
        _ => {
            if !target_ver.is_empty() && target_ver.starts_with('v') {
                target_ver.to_string()
            } else if lang.is_pt() {
                "Instalado".to_string()
            } else {
                "Installed".to_string()
            }
        }
    }
}

pub fn detect_bridge_version(bridge_path: &Path, remote_hash: &str, target_ver: &str, lang: Lang) -> String {
    if !bridge_path.exists() {
        return if lang.is_pt() {
            "Não instalado".to_string()
        } else {
            "Not installed".to_string()
        };
    }
    // 1. Tenta extrair a versão pelo marcador embutido no binário
    if let Some(v) = extract_marker_version(bridge_path, b"CK_BRIDGE_VER:") {
        return v;
    }

    let cur_hash = sha256_file(bridge_path).unwrap_or_default().to_lowercase();
    if !cur_hash.is_empty() && !remote_hash.is_empty() && cur_hash == remote_hash {
        return target_ver.to_string();
    }
    let embedded_hash = sha256_hex(crate::commands::EMBEDDED_CHRONOKEEP_DLL).to_lowercase();
    if !cur_hash.is_empty() && cur_hash == embedded_hash {
        return format!("v{}", env!("CARGO_PKG_VERSION"));
    }
    match cur_hash.as_str() {
        "12831a16d2196a6d288606a28cdcad763f0add737382e62803bf56d6dbcdd94e" => "v0.1.10".to_string(),
        "57ac95df3db6a2ee315df9e9ceaac756b10664ccce53c29d6c91059d42426c47" => "v0.1.9".to_string(),
        "6f1e2585b9f3d9505879799929208bed3b7d98dabe4b10912354d596964f1f46" => "v0.1.8".to_string(),
        "a7afdbd1b5d20005387d82b4cf55795c4c728b932f8ea766e0bce8e00fa6872a" => "v0.1.7".to_string(),
        "cd56c1520b8f0326c67e154ed7ab7099ccd843edd9a4ef5a56bca278960f51ec" => "v0.1.6".to_string(),
        "1a5767db4161aace8be0f7be7c05518575bafa40914b4301cac106d5e7c14e21" => "v0.1.5".to_string(),
        "8433a409012ef36ac17e48f29b1e5591793b563ed5316f0724d7da724fbf56ff" => "v0.1.5".to_string(),
        "9a883c9e39592e4934dac08c7b5aa1099116369e7b686b2bbeb674c84609a988" => "v0.1.4".to_string(),
        "daf4c9e79183632d97adeae7ea432eddf600b91197c8a2c5c51c42a2a0cd56aa" => "v0.1.4".to_string(),
        "ec4fb0630a74624d782a211846cf070e06a0e48fa8ca6e7ca0d43c620931943f" => "v0.1.3".to_string(),
        _ => {
            if !target_ver.is_empty() && target_ver.starts_with('v') {
                target_ver.to_string()
            } else if lang.is_pt() {
                "Instalado".to_string()
            } else {
                "Installed".to_string()
            }
        }
    }
}

fn parse_version_tuple(v: &str) -> (u32, u32, u32) {
    let clean = v.trim().trim_start_matches('v').trim_start_matches('V');
    let mut parts = clean.split('.').filter_map(|s| s.parse::<u32>().ok());
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn is_remote_newer(current: &str, remote: &str) -> bool {
    let cur = parse_version_tuple(current);
    let rem = parse_version_tuple(remote);
    rem > cur
}

pub fn check_updates(steam_dir: &Path, lang: Lang) -> UpdateCheckResult {
    let manifest = match fetch_remote_manifest(lang) {
        Ok(m) => m,
        Err(e) => {
            return UpdateCheckResult {
                success: false,
                message: e,
                release_tag: String::new(),
                has_updates: false,
                components: Vec::new(),
            };
        }
    };

    let not_installed_str = if lang.is_pt() {
        "Não instalado"
    } else {
        "Not installed"
    };

    let mut components = Vec::new();

    // 1. ChronoKeep (GUI)
    let gui_manifest = manifest.components.get("gui");
    let gui_latest_ver = gui_manifest
        .and_then(|m| m.version.clone())
        .unwrap_or_else(|| manifest.release_version.clone());

    let (gui_hash, gui_status, gui_dl, gui_filename) = if let Some(m) = gui_manifest {
        let remote_hash = m.sha256.to_lowercase();
        let current_hash = std::env::current_exe()
            .ok()
            .and_then(|p| sha256_file(&p).ok())
            .unwrap_or_default()
            .to_lowercase();

        let cur_ver = format!("v{}", env!("CARGO_PKG_VERSION"));
        let status = if current_hash.is_empty() {
            "error".to_string()
        } else if current_hash == remote_hash {
            "up_to_date".to_string()
        } else if is_remote_newer(&cur_ver, &gui_latest_ver) {
            "update_available".to_string()
        } else {
            "up_to_date".to_string()
        };

        (current_hash, status, m.download_url.clone(), m.filename.clone())
    } else {
        (String::new(), "up_to_date".to_string(), String::new(), "ChronoKeep.exe".to_string())
    };

    components.push(ComponentUpdateInfo {
        id: "gui".to_string(),
        name: if lang.is_pt() {
            "ChronoKeep (Painel)".to_string()
        } else {
            "ChronoKeep (GUI Panel)".to_string()
        },
        current_version: format!("v{}", env!("CARGO_PKG_VERSION")),
        latest_version: gui_latest_ver,
        current_hash: gui_hash,
        latest_hash: gui_manifest.map(|m| m.sha256.to_lowercase()).unwrap_or_default(),
        status: gui_status,
        download_url: gui_dl,
        filename: gui_filename,
    });

    // 2. chrono-daemon (Serviço)
    let daemon_manifest = manifest.components.get("daemon");
    let daemon_path = steam_dir.join("chrono-daemon.exe");
    let daemon_latest_ver = daemon_manifest
        .and_then(|m| m.version.clone())
        .unwrap_or_else(|| manifest.release_version.clone());

    let (daemon_hash, daemon_status, daemon_ver, daemon_dl, daemon_filename) = if let Some(m) = daemon_manifest {
        let remote_hash = m.sha256.to_lowercase();
        if daemon_path.exists() {
            let current_hash = sha256_file(&daemon_path).unwrap_or_default().to_lowercase();
            let ver = detect_daemon_version(&daemon_path, &remote_hash, &daemon_latest_ver, lang);
            let status = if current_hash == remote_hash {
                "up_to_date".to_string()
            } else if is_remote_newer(&ver, &daemon_latest_ver) {
                "update_available".to_string()
            } else if ver == daemon_latest_ver {
                "up_to_date".to_string()
            } else if !is_remote_newer(&ver, &daemon_latest_ver) && ver.starts_with('v') {
                "up_to_date".to_string()
            } else {
                "update_available".to_string()
            };
            (current_hash, status, ver, m.download_url.clone(), m.filename.clone())
        } else {
            (String::new(), "not_installed".to_string(), not_installed_str.to_string(), m.download_url.clone(), m.filename.clone())
        }
    } else {
        (String::new(), "up_to_date".to_string(), "OK".to_string(), String::new(), "chrono-daemon.exe".to_string())
    };

    components.push(ComponentUpdateInfo {
        id: "daemon".to_string(),
        name: if lang.is_pt() {
            "chrono-daemon (Serviço)".to_string()
        } else {
            "chrono-daemon (Service)".to_string()
        },
        current_version: daemon_ver,
        latest_version: daemon_latest_ver,
        current_hash: daemon_hash,
        latest_hash: daemon_manifest.map(|m| m.sha256.to_lowercase()).unwrap_or_default(),
        status: daemon_status,
        download_url: daemon_dl,
        filename: daemon_filename,
    });

    // 3. chronokeep.dll (Bridge Steam Cloud)
    let bridge_manifest = manifest.components.get("bridge")
        .or_else(|| manifest.components.get("bridge_dll"));
    let bridge_path = if steam_dir.join("chronokeep.dll").exists() {
        steam_dir.join("chronokeep.dll")
    } else if steam_dir.join("cloud_redirect.dll").exists() {
        steam_dir.join("cloud_redirect.dll")
    } else {
        steam_dir.join("chronokeep.dll")
    };
    let bridge_latest_ver = bridge_manifest
        .and_then(|m| m.version.clone())
        .unwrap_or_else(|| manifest.release_version.clone());

    let (bridge_hash, bridge_status, bridge_ver, bridge_dl, bridge_filename) = if let Some(m) = bridge_manifest {
        let remote_hash = m.sha256.to_lowercase();
        if bridge_path.exists() {
            let current_hash = sha256_file(&bridge_path).unwrap_or_default().to_lowercase();
            let ver = detect_bridge_version(&bridge_path, &remote_hash, &bridge_latest_ver, lang);
            let status = if current_hash == remote_hash {
                "up_to_date".to_string()
            } else if is_remote_newer(&ver, &bridge_latest_ver) {
                "update_available".to_string()
            } else if ver == bridge_latest_ver {
                "up_to_date".to_string()
            } else if !is_remote_newer(&ver, &bridge_latest_ver) && ver.starts_with('v') {
                "up_to_date".to_string()
            } else {
                "update_available".to_string()
            };
            (current_hash, status, ver, m.download_url.clone(), m.filename.clone())
        } else {
            (String::new(), "not_installed".to_string(), not_installed_str.to_string(), m.download_url.clone(), m.filename.clone())
        }
    } else {
        (String::new(), "up_to_date".to_string(), "OK".to_string(), String::new(), "chronokeep.dll".to_string())
    };

    components.push(ComponentUpdateInfo {
        id: "bridge".to_string(),
        name: if lang.is_pt() {
            "chronokeep.dll (Bridge Steam Cloud)".to_string()
        } else {
            "chronokeep.dll (Steam Cloud Bridge)".to_string()
        },
        current_version: bridge_ver,
        latest_version: bridge_latest_ver,
        current_hash: bridge_hash,
        latest_hash: bridge_manifest.map(|m| m.sha256.to_lowercase()).unwrap_or_default(),
        status: bridge_status,
        download_url: bridge_dl,
        filename: bridge_filename,
    });

    let has_updates = components.iter().any(|c| c.status == "update_available");

    UpdateCheckResult {
        success: true,
        message: if has_updates {
            if lang.is_pt() {
                "Atualizações disponíveis!".to_string()
            } else {
                "Updates available!".to_string()
            }
        } else if lang.is_pt() {
            "Todos os componentes do ChronoKeep estão atualizados.".to_string()
        } else {
            "All ChronoKeep components are up to date.".to_string()
        },
        release_tag: manifest.release_version,
        has_updates,
        components,
    }
}

pub fn apply_update(
    component_id: &str,
    steam_dir: &Path,
    steam_was_running_override: Option<bool>,
    reopen_steam: bool,
    lang: Lang,
) -> Result<UpdateApplyResult, String> {
    let manifest = fetch_remote_manifest(lang)?;

    match component_id {
        "daemon" => {
            let comp = manifest.components.get("daemon").ok_or_else(|| {
                if lang.is_pt() {
                    "Componente 'daemon' não encontrado no manifesto remoto.".to_string()
                } else {
                    "Component 'daemon' not found in remote manifest.".to_string()
                }
            })?;

            let (status, bytes) = https_download_url(&comp.download_url, 60_000).ok_or_else(|| {
                if lang.is_pt() {
                    format!("Falha no download de {}", comp.filename)
                } else {
                    format!("Failed to download {}", comp.filename)
                }
            })?;

            if status != 200 {
                return Err(if lang.is_pt() {
                    format!("Download de {} falhou com status HTTP {}.", comp.filename, status)
                } else {
                    format!("Download of {} failed with HTTP status {}.", comp.filename, status)
                });
            }

            let hash = sha256_hex(&bytes).to_lowercase();
            if hash != comp.sha256.to_lowercase() {
                return Err(if lang.is_pt() {
                    format!("Falha de integridade SHA-256 no arquivo baixado: {}", comp.filename)
                } else {
                    format!("SHA-256 integrity check failed for downloaded file: {}", comp.filename)
                });
            }

            let steam_currently_running = is_process_running("steam.exe");
            let steam_was_running = steam_was_running_override.unwrap_or(steam_currently_running);

            if steam_currently_running {
                close_steam_and_wait(steam_dir, 5000);
            }

            let port = resolve_service_port(&steam_dir.to_string_lossy());
            stop_service_from_gui(port);
            kill_process_and_wait("chrono-daemon.exe", 2000);

            let dst = steam_dir.join("chrono-daemon.exe");
            if !deploy_bytes_safe(&bytes, &dst) {
                return Err(if lang.is_pt() {
                    "Falha ao gravar novo chrono-daemon.exe em disco.".to_string()
                } else {
                    "Failed to write new chrono-daemon.exe to disk.".to_string()
                });
            }

            let restarted = start_service_from_gui(steam_dir, port);

            if steam_was_running && reopen_steam {
                launch_steam(steam_dir);
            }

            let msg = if steam_was_running && reopen_steam {
                if lang.is_pt() {
                    "chrono-daemon.exe atualizado! A Steam foi reiniciada.".to_string()
                } else {
                    "chrono-daemon.exe updated! Steam has been restarted.".to_string()
                }
            } else if steam_was_running {
                if lang.is_pt() {
                    "chrono-daemon.exe atualizado com sucesso!".to_string()
                } else {
                    "chrono-daemon.exe updated successfully!".to_string()
                }
            } else if restarted {
                if lang.is_pt() {
                    "chrono-daemon.exe atualizado e reiniciado com sucesso!".to_string()
                } else {
                    "chrono-daemon.exe updated and restarted successfully!".to_string()
                }
            } else if lang.is_pt() {
                "chrono-daemon.exe atualizado com sucesso!".to_string()
            } else {
                "chrono-daemon.exe updated successfully!".to_string()
            };

            Ok(UpdateApplyResult {
                success: true,
                message: msg,
                component_id: "daemon".to_string(),
                restart_required: false,
            })
        }
        "bridge" => {
            let comp = manifest
                .components
                .get("bridge")
                .or_else(|| manifest.components.get("bridge_dll"))
                .ok_or_else(|| {
                    if lang.is_pt() {
                        "Componente 'bridge' não encontrado no manifesto remoto.".to_string()
                    } else {
                        "Component 'bridge' not found in remote manifest.".to_string()
                    }
                })?;

            let (status, bytes) = https_download_url(&comp.download_url, 60_000).ok_or_else(|| {
                if lang.is_pt() {
                    format!("Falha no download de {}", comp.filename)
                } else {
                    format!("Failed to download {}", comp.filename)
                }
            })?;

            if status != 200 {
                return Err(if lang.is_pt() {
                    format!("Download de {} falhou com status HTTP {}.", comp.filename, status)
                } else {
                    format!("Download of {} failed with HTTP status {}.", comp.filename, status)
                });
            }

            let hash = sha256_hex(&bytes).to_lowercase();
            if hash != comp.sha256.to_lowercase() {
                return Err(if lang.is_pt() {
                    format!("Falha de integridade SHA-256 no arquivo baixado: {}", comp.filename)
                } else {
                    format!("SHA-256 integrity check failed for downloaded file: {}", comp.filename)
                });
            }

            let steam_currently_running = is_process_running("steam.exe");
            let steam_was_running = steam_was_running_override.unwrap_or(steam_currently_running);

            if steam_currently_running {
                close_steam_and_wait(steam_dir, 5000);
            }

            let dst_chronokeep = steam_dir.join("chronokeep.dll");
            let dst_redirect = steam_dir.join("cloud_redirect.dll");

            let ok1 = deploy_bytes_safe(&bytes, &dst_chronokeep);
            let _ = deploy_bytes_safe(&bytes, &dst_redirect);

            if !ok1 {
                return Err(if lang.is_pt() {
                    "Falha ao gravar chronokeep.dll no diretório da Steam.".to_string()
                } else {
                    "Failed to write chronokeep.dll to Steam directory.".to_string()
                });
            }

            if steam_was_running && reopen_steam {
                launch_steam(steam_dir);
            }

            let msg = if steam_was_running && reopen_steam {
                if lang.is_pt() {
                    "chronokeep.dll atualizada! A Steam foi reiniciada.".to_string()
                } else {
                    "chronokeep.dll updated! Steam has been restarted.".to_string()
                }
            } else if lang.is_pt() {
                "chronokeep.dll atualizada com sucesso!".to_string()
            } else {
                "chronokeep.dll updated successfully!".to_string()
            };

            Ok(UpdateApplyResult {
                success: true,
                message: msg,
                component_id: "bridge".to_string(),
                restart_required: false,
            })
        }
        "gui" => {
            let comp = manifest.components.get("gui").ok_or_else(|| {
                if lang.is_pt() {
                    "Componente 'gui' não encontrado no manifesto remoto.".to_string()
                } else {
                    "Component 'gui' not found in remote manifest.".to_string()
                }
            })?;

            let (status, bytes) = https_download_url(&comp.download_url, 120_000).ok_or_else(|| {
                if lang.is_pt() {
                    format!("Falha no download de {}", comp.filename)
                } else {
                    format!("Failed to download {}", comp.filename)
                }
            })?;

            if status != 200 {
                return Err(if lang.is_pt() {
                    format!("Download de {} falhou com status HTTP {}.", comp.filename, status)
                } else {
                    format!("Download of {} failed with HTTP status {}.", comp.filename, status)
                });
            }

            let hash = sha256_hex(&bytes).to_lowercase();
            if hash != comp.sha256.to_lowercase() {
                return Err(if lang.is_pt() {
                    format!("Falha de integridade SHA-256 no arquivo baixado: {}", comp.filename)
                } else {
                    format!("SHA-256 integrity check failed for downloaded file: {}", comp.filename)
                });
            }

            let cur_exe = std::env::current_exe().map_err(|e| {
                if lang.is_pt() {
                    format!("Não foi possível determinar caminho do executável atual: {}", e)
                } else {
                    format!("Could not determine current executable path: {}", e)
                }
            })?;

            let new_exe = cur_exe.with_extension("new.exe");
            std::fs::write(&new_exe, &bytes).map_err(|e| {
                if lang.is_pt() {
                    format!("Falha ao salvar {} temporariamente: {}", comp.filename, e)
                } else {
                    format!("Failed to save temporary {}: {}", comp.filename, e)
                }
            })?;

            #[cfg(target_os = "windows")]
            {
                let cur_str = sanitize_windows_path(&cur_exe);
                let new_str = sanitize_windows_path(&new_exe);
                let pid = std::process::id();
                let bat_path = std::env::temp_dir().join(format!("chronokeep_swap_{}.bat", pid));

                let bat_script = format!(
                    "@echo off\r\n\
                    chcp 65001 >nul\r\n\
                    timeout /t 2 /nobreak >nul\r\n\
                    :retry_del\r\n\
                    del /f /q \"{cur}\" >nul 2>&1\r\n\
                    if exist \"{cur}\" (\r\n\
                        timeout /t 1 /nobreak >nul\r\n\
                        goto retry_del\r\n\
                    )\r\n\
                    move /y \"{new}\" \"{cur}\" >nul 2>&1\r\n\
                    start \"\" \"{cur}\"\r\n\
                    (goto) 2>nul & del \"%~f0\"\r\n",
                    cur = cur_str,
                    new = new_str
                );

                if let Err(e) = std::fs::write(&bat_path, bat_script) {
                    return Err(if lang.is_pt() {
                        format!("Falha ao criar script de atualização: {}", e)
                    } else {
                        format!("Failed to create update script: {}", e)
                    });
                }

                let mut cmd = std::process::Command::new("cmd.exe");
                cmd.args(&["/c", &bat_path.to_string_lossy().to_string()]);
                cmd.creation_flags(CREATE_NO_WINDOW);
                if let Err(e) = cmd.spawn() {
                    return Err(if lang.is_pt() {
                        format!("Falha ao executar script de atualização: {}", e)
                    } else {
                        format!("Failed to launch update script: {}", e)
                    });
                }

                // Encerra processo atual após 300ms para liberar o executável para substituição
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    std::process::exit(0);
                });
            }

            Ok(UpdateApplyResult {
                success: true,
                message: if lang.is_pt() {
                    "ChronoKeep atualizado! Reiniciando o aplicativo em instantes...".to_string()
                } else {
                    "ChronoKeep updated! Restarting the application shortly...".to_string()
                },
                component_id: "gui".to_string(),
                restart_required: true,
            })
        }
        _ => Err(if lang.is_pt() {
            format!("Componente desconhecido: {}", component_id)
        } else {
            format!("Unknown component: {}", component_id)
        }),
    }
}
