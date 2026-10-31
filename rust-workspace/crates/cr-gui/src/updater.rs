//! Automatic update verification and deployment for ChronoKeep components.
//! Supports decoupled updates of ChronoKeep (GUI), chrono-daemon, and chronokeep.dll.

use crate::installer::deploy_bytes_safe;
use crate::service_mgr::{is_process_running, kill_process_and_wait, start_service_from_gui, stop_service_from_gui};
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

pub fn fetch_remote_manifest() -> Result<RemoteManifest, String> {
    // Tenta URL primária do release
    let res = https_download_url(PRIMARY_VERSIONS_URL, 15_000)
        .or_else(|| https_download_url(FALLBACK_VERSIONS_URL, 15_000));

    let (status, bytes) = res.ok_or_else(|| {
        "Não foi possível conectar ao servidor de versões do GitHub. Verifique sua conexão com a internet.".to_string()
    })?;

    if status != 200 {
        return Err(format!("Servidor de atualização retornou HTTP {}.", status));
    }

    serde_json::from_slice::<RemoteManifest>(&bytes)
        .map_err(|e| format!("Falha ao decodificar manifesto de versões: {}", e))
}

pub fn check_updates(steam_dir: &Path) -> UpdateCheckResult {
    let manifest = match fetch_remote_manifest() {
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

    let mut components = Vec::new();

    // 1. ChronoKeep (GUI)
    let gui_manifest = manifest.components.get("gui");
    let (gui_hash, gui_status, gui_dl, gui_filename) = if let Some(m) = gui_manifest {
        let remote_hash = m.sha256.to_lowercase();
        let current_hash = std::env::current_exe()
            .ok()
            .and_then(|p| sha256_file(&p).ok())
            .unwrap_or_default()
            .to_lowercase();

        let status = if current_hash.is_empty() {
            "error".to_string()
        } else if current_hash == remote_hash {
            "up_to_date".to_string()
        } else {
            "update_available".to_string()
        };

        (current_hash, status, m.download_url.clone(), m.filename.clone())
    } else {
        (String::new(), "up_to_date".to_string(), String::new(), "ChronoKeep.exe".to_string())
    };

    components.push(ComponentUpdateInfo {
        id: "gui".to_string(),
        name: "ChronoKeep (Painel)".to_string(),
        current_version: format!("v{}", env!("CARGO_PKG_VERSION")),
        latest_version: manifest.release_version.clone(),
        current_hash: gui_hash,
        latest_hash: gui_manifest.map(|m| m.sha256.to_lowercase()).unwrap_or_default(),
        status: gui_status,
        download_url: gui_dl,
        filename: gui_filename,
    });

    // 2. chrono-daemon (Serviço)
    let daemon_manifest = manifest.components.get("daemon");
    let daemon_path = steam_dir.join("chrono-daemon.exe");
    let (daemon_hash, daemon_status, daemon_ver, daemon_dl, daemon_filename) = if let Some(m) = daemon_manifest {
        let remote_hash = m.sha256.to_lowercase();
        if daemon_path.exists() {
            let current_hash = sha256_file(&daemon_path).unwrap_or_default().to_lowercase();
            let (status, ver) = if current_hash == remote_hash {
                ("up_to_date".to_string(), manifest.release_version.clone())
            } else {
                ("update_available".to_string(), "Desatualizado".to_string())
            };
            (current_hash, status, ver, m.download_url.clone(), m.filename.clone())
        } else {
            (String::new(), "not_installed".to_string(), "Não instalado".to_string(), m.download_url.clone(), m.filename.clone())
        }
    } else {
        (String::new(), "up_to_date".to_string(), "OK".to_string(), String::new(), "chrono-daemon.exe".to_string())
    };

    components.push(ComponentUpdateInfo {
        id: "daemon".to_string(),
        name: "chrono-daemon (Serviço)".to_string(),
        current_version: daemon_ver,
        latest_version: manifest.release_version.clone(),
        current_hash: daemon_hash,
        latest_hash: daemon_manifest.map(|m| m.sha256.to_lowercase()).unwrap_or_default(),
        status: daemon_status,
        download_url: daemon_dl,
        filename: daemon_filename,
    });

    // 3. chronokeep.dll (Bridge Steam Cloud)
    let bridge_manifest = manifest.components.get("bridge")
        .or_else(|| manifest.components.get("bridge_dll"));
    let bridge_path = steam_dir.join("chronokeep.dll");
    let (bridge_hash, bridge_status, bridge_ver, bridge_dl, bridge_filename) = if let Some(m) = bridge_manifest {
        let remote_hash = m.sha256.to_lowercase();
        if bridge_path.exists() {
            let current_hash = sha256_file(&bridge_path).unwrap_or_default().to_lowercase();
            let (status, ver) = if current_hash == remote_hash {
                ("up_to_date".to_string(), manifest.release_version.clone())
            } else {
                ("update_available".to_string(), "Desatualizado".to_string())
            };
            (current_hash, status, ver, m.download_url.clone(), m.filename.clone())
        } else {
            (String::new(), "not_installed".to_string(), "Não instalado".to_string(), m.download_url.clone(), m.filename.clone())
        }
    } else {
        (String::new(), "up_to_date".to_string(), "OK".to_string(), String::new(), "chronokeep.dll".to_string())
    };

    components.push(ComponentUpdateInfo {
        id: "bridge".to_string(),
        name: "chronokeep.dll (Bridge Steam Cloud)".to_string(),
        current_version: bridge_ver,
        latest_version: manifest.release_version.clone(),
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
            "Atualizações disponíveis!".to_string()
        } else {
            "Todos os componentes do ChronoKeep estão atualizados.".to_string()
        },
        release_tag: manifest.release_version,
        has_updates,
        components,
    }
}

pub fn apply_update(component_id: &str, steam_dir: &Path) -> Result<UpdateApplyResult, String> {
    let manifest = fetch_remote_manifest()?;

    match component_id {
        "daemon" => {
            let comp = manifest.components.get("daemon")
                .ok_or_else(|| "Componente 'daemon' não encontrado no manifesto remoto.".to_string())?;

            let (status, bytes) = https_download_url(&comp.download_url, 60_000)
                .ok_or_else(|| format!("Falha no download de {}", comp.filename))?;

            if status != 200 {
                return Err(format!("Download de {} falhou com status HTTP {}.", comp.filename, status));
            }

            let hash = sha256_hex(&bytes).to_lowercase();
            if hash != comp.sha256.to_lowercase() {
                return Err(format!("Falha de integridade SHA-256 no arquivo baixado: {}", comp.filename));
            }

            let port = resolve_service_port(&steam_dir.to_string_lossy());
            stop_service_from_gui(port);
            kill_process_and_wait("chrono-daemon.exe", 2000);

            let dst = steam_dir.join("chrono-daemon.exe");
            if !deploy_bytes_safe(&bytes, &dst) {
                return Err("Falha ao gravar novo chrono-daemon.exe em disco.".to_string());
            }

            let restarted = start_service_from_gui(steam_dir, port);

            Ok(UpdateApplyResult {
                success: true,
                message: if restarted {
                    "chrono-daemon.exe atualizado e reiniciado com sucesso!".to_string()
                } else {
                    "chrono-daemon.exe atualizado com sucesso!".to_string()
                },
                component_id: "daemon".to_string(),
                restart_required: false,
            })
        }
        "bridge" => {
            let comp = manifest.components.get("bridge")
                .or_else(|| manifest.components.get("bridge_dll"))
                .ok_or_else(|| "Componente 'bridge' não encontrado no manifesto remoto.".to_string())?;

            let (status, bytes) = https_download_url(&comp.download_url, 60_000)
                .ok_or_else(|| format!("Falha no download de {}", comp.filename))?;

            if status != 200 {
                return Err(format!("Download de {} falhou com status HTTP {}.", comp.filename, status));
            }

            let hash = sha256_hex(&bytes).to_lowercase();
            if hash != comp.sha256.to_lowercase() {
                return Err(format!("Falha de integridade SHA-256 no arquivo baixado: {}", comp.filename));
            }

            let steam_running = is_process_running("steam.exe");

            let dst_chronokeep = steam_dir.join("chronokeep.dll");
            let dst_redirect = steam_dir.join("cloud_redirect.dll");

            let ok1 = deploy_bytes_safe(&bytes, &dst_chronokeep);
            let _ = deploy_bytes_safe(&bytes, &dst_redirect);

            if !ok1 {
                return Err("Falha ao gravar chronokeep.dll no diretório da Steam.".to_string());
            }

            let msg = if steam_running {
                "chronokeep.dll atualizada! Reinicie a Steam para carregar a nova versão.".to_string()
            } else {
                "chronokeep.dll atualizada com sucesso!".to_string()
            };

            Ok(UpdateApplyResult {
                success: true,
                message: msg,
                component_id: "bridge".to_string(),
                restart_required: steam_running,
            })
        }
        "gui" => {
            let comp = manifest.components.get("gui")
                .ok_or_else(|| "Componente 'gui' não encontrado no manifesto remoto.".to_string())?;

            let (status, bytes) = https_download_url(&comp.download_url, 120_000)
                .ok_or_else(|| format!("Falha no download de {}", comp.filename))?;

            if status != 200 {
                return Err(format!("Download de {} falhou com status HTTP {}.", comp.filename, status));
            }

            let hash = sha256_hex(&bytes).to_lowercase();
            if hash != comp.sha256.to_lowercase() {
                return Err(format!("Falha de integridade SHA-256 no arquivo baixado: {}", comp.filename));
            }

            let cur_exe = std::env::current_exe()
                .map_err(|e| format!("Não foi possível determinar caminho do executável atual: {}", e))?;

            let new_exe = cur_exe.with_extension("new.exe");
            std::fs::write(&new_exe, &bytes)
                .map_err(|e| format!("Falha ao salvar {} temporariamente: {}", comp.filename, e))?;

            #[cfg(target_os = "windows")]
            {
                let cur_str = cur_exe.to_string_lossy().to_string();
                let new_str = new_exe.to_string_lossy().to_string();
                let batch_cmd = format!(
                    "ping 127.0.0.1 -n 3 >nul & del /f /q \"{}\" & move /y \"{}\" \"{}\" & start \"\" \"{}\"",
                    cur_str, new_str, cur_str, cur_str
                );
                let mut cmd = std::process::Command::new("cmd.exe");
                cmd.args(&["/c", &batch_cmd]);
                cmd.creation_flags(CREATE_NO_WINDOW);
                let _ = cmd.spawn();

                // Encerra processo atual após 1.5 segundo para liberar o arquivo para troca
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    std::process::exit(0);
                });
            }

            Ok(UpdateApplyResult {
                success: true,
                message: "ChronoKeep atualizado! Reiniciando o aplicativo em instantes...".to_string(),
                component_id: "gui".to_string(),
                restart_required: true,
            })
        }
        _ => Err(format!("Componente desconhecido: {}", component_id)),
    }
}
