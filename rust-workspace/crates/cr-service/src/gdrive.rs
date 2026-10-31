//! Native Google Drive Synchronization Engine for `chrono-daemon.exe`.
//! Reuses the shared WinHTTP & Google Drive REST API client from `cr-common`.

use cr_common::gdrive::{
    clear_folder_cache, download_drive_file, find_drive_folder, find_or_create_drive_folder,
    get_valid_access_token, list_drive_files, upload_file_multipart,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ACTIVE_PULLS: Mutex<Option<HashSet<(u32, u32)>>> = Mutex::new(None);
static PULL_ALL_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asynchronous-safe helper to upload a save file into `ChronoKeep/<account_id>/<app_id>/<rel_path>` on Google Drive.
pub fn sync_file_to_gdrive(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
    log_path: Option<&PathBuf>,
) -> bool {
    if sync_file_to_gdrive_attempt(exe_dir, account_id, app_id, rel_filename, data, log_path) {
        return true;
    }
    // Auto-recuperação: limpa o cache de pastas e retenta recriando a hierarquia no Drive
    clear_folder_cache();
    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive Upload] Falha inicial ao enviar app={} file='{}'. Auto-recuperando hierarquia no Drive...",
                app_id, rel_filename
            ),
        );
    }
    sync_file_to_gdrive_attempt(exe_dir, account_id, app_id, rel_filename, data, log_path)
}

fn sync_file_to_gdrive_attempt(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
    log_path: Option<&PathBuf>,
) -> bool {
    let mut token_path = exe_dir.join("chronokeep_gdrive.json");
    if !token_path.exists() {
        if let Ok(cur) = std::env::current_exe() {
            if let Some(parent) = cur.parent() {
                let candidate = parent.join("chronokeep_gdrive.json");
                if candidate.exists() {
                    token_path = candidate;
                }
            }
        }
    }

    if !token_path.exists() {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive Upload] Abortado: chronokeep_gdrive.json nao encontrado em {:?}",
                    exe_dir
                ),
            );
        }
        return false;
    }

    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive Upload] Erro: Falha ao obter access_token valido com refresh_token.",
            );
        }
        return false;
    };

    let Some(root_id) = find_or_create_drive_folder(&token, "ChronoKeep", None) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive Upload] Erro: Falha ao encontrar ou criar pasta raiz 'ChronoKeep'.",
            );
        }
        return false;
    };

    let Some(acct_id) =
        find_or_create_drive_folder(&token, &account_id.to_string(), Some(&root_id))
    else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive Upload] Erro: Falha ao encontrar ou criar pasta da conta {}",
                    account_id
                ),
            );
        }
        return false;
    };

    let Some(mut parent_id) =
        find_or_create_drive_folder(&token, &app_id.to_string(), Some(&acct_id))
    else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive Upload] Erro: Falha ao encontrar ou criar pasta do app {}",
                    app_id
                ),
            );
        }
        return false;
    };

    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return false;
    }
    for sub in &parts[..parts.len() - 1] {
        let Some(next_id) = find_or_create_drive_folder(&token, sub, Some(&parent_id)) else {
            return false;
        };
        parent_id = next_id;
    }
    let leaf_name = parts[parts.len() - 1];

    let ok = upload_file_multipart(&token, &parent_id, leaf_name, data);
    if let Some(lp) = log_path {
        if ok {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive Upload] SUCESSO: app={} file='{}' ({}B) enviado com sucesso",
                    app_id,
                    rel_filename,
                    data.len()
                ),
            );
        } else {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive Upload] ERRO: Falha ao enviar app={} file='{}'",
                    app_id, rel_filename
                ),
            );
        }
    }
    ok
}

fn file_matches_remote(path: &Path, expected_size: u64, expected_md5: Option<&str>) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() != expected_size {
        return false;
    }
    if let Some(remote_md5) = expected_md5 {
        if remote_md5.is_empty() {
            return true;
        }
        let Ok(local_md5) = cr_proto::md5::md5_file(path) else {
            return false;
        };
        local_md5.eq_ignore_ascii_case(remote_md5)
    } else {
        true
    }
}

fn pull_drive_folder_recursive(
    access_token: &str,
    folder_id: &str,
    rel_prefix: &str,
    account_id: u32,
    app_id: u32,
    vault: &crate::vault::VaultStorage,
    log_path: Option<&PathBuf>,
) -> usize {
    let items = list_drive_files(access_token, folder_id);
    let mut synced_count = 0;

    for item in items {
        if item.mime_type == "application/vnd.google-apps.folder" {
            // Ignora pastas na nuvem cujo nome seja um token de raiz (ex: %WinMyDocuments%)
            if item.name.starts_with('%') && item.name.ends_with('%') {
                continue;
            }
            let next_prefix = if rel_prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{}/{}", rel_prefix, item.name)
            };
            synced_count += pull_drive_folder_recursive(
                access_token,
                &item.id,
                &next_prefix,
                account_id,
                app_id,
                vault,
                log_path,
            );
        } else {
            let raw_rel_file = if rel_prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{}/{}", rel_prefix, item.name)
            };
            let (_, clean_file) = cr_proto::cloud_messages::split_root_token(&raw_rel_file);
            let rel_file = if !clean_file.is_empty() { clean_file } else { raw_rel_file };

            // Se for manifest.tsv, só adota se a nuvem tiver um CN superior ao local
            if rel_file == "manifest.tsv" {
                if let Some(bytes) = download_drive_file(access_token, &item.id) {
                    let cloud_str = String::from_utf8_lossy(&bytes);
                    let parsed = cr_common::manifest::parse_manifest(&cloud_str);
                    let cloud_cn = parsed.change_number;
                    let current_manifest = vault.load_and_reconcile_manifest(account_id, app_id);
                    if cloud_cn > current_manifest.change_number {
                        let m_path = vault.manifest_path(account_id, app_id);
                        if let Some(parent) = m_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&m_path, bytes);
                    }
                }
                continue;
            }

            // Verifica integridade com o Vault e com userdata
            let vault_file_path = vault.resolve_file_disk_path(account_id, app_id, &rel_file);
            let user_dest_path = vault.steam_path.as_ref().map(|sp| {
                sp.join("userdata")
                    .join(account_id.to_string())
                    .join(app_id.to_string())
                    .join("remote")
                    .join(&rel_file)
            });

            let vault_matches = vault_file_path
                .as_ref()
                .map(|p| file_matches_remote(p, item.size, item.md5_checksum.as_deref()))
                .unwrap_or(false);

            let user_dest_matches = user_dest_path
                .as_ref()
                .map(|p| file_matches_remote(p, item.size, item.md5_checksum.as_deref()))
                .unwrap_or(false);

            // 1. Se o Vault já possui o arquivo com hash/conteúdo correspondente à nuvem:
            if vault_matches {
                if let Some(ref user_dest) = user_dest_path {
                    if !user_dest.exists() {
                        if let Some(ref vf) = vault_file_path {
                            if let Some(p) = user_dest.parent() {
                                let _ = fs::create_dir_all(p);
                            }
                            if fs::copy(vf, user_dest).is_ok() {
                                synced_count += 1;
                                if let Some(lp) = log_path {
                                    crate::log_msg(
                                        lp,
                                        &format!(
                                            "[Google Drive] userdata restaurado a partir do Vault: app={} file='{}'",
                                            app_id,
                                            rel_file
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                continue;
            }

            // 2. Se o Vault não possui, mas userdata já tem o arquivo com o hash idêntico da nuvem:
            if user_dest_matches {
                if let Some(ref user_dest) = user_dest_path {
                    if let Some(ref vf) = vault_file_path {
                        if let Some(p) = vf.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        if fs::copy(user_dest, vf).is_ok() {
                            synced_count += 1;
                            if let Some(lp) = log_path {
                                crate::log_msg(
                                    lp,
                                    &format!(
                                        "[Google Drive] Vault semeado a partir de userdata: app={} file='{}'",
                                        app_id,
                                        rel_file
                                    ),
                                );
                            }
                        }
                    }
                }
                continue;
            }

            // 3. Nem o Vault nem o userdata coincidem com a nuvem: realiza o download
            if let Some(bytes) = download_drive_file(access_token, &item.id) {
                if vault
                    .write_save_file(account_id, app_id, &rel_file, &bytes)
                    .is_ok()
                {
                    synced_count += 1;
                    if let Some(ref sp) = vault.steam_path {
                        let user_dest = sp
                            .join("userdata")
                            .join(account_id.to_string())
                            .join(app_id.to_string())
                            .join("remote")
                            .join(&rel_file);
                        if let Some(p) = user_dest.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        let _ = fs::write(user_dest, &bytes);
                    }
                    if let Some(lp) = log_path {
                        crate::log_msg(
                            lp,
                            &format!(
                                "[Google Drive] Download concluido: app={} file='{}' ({}B)",
                                app_id,
                                rel_file,
                                bytes.len()
                            ),
                        );
                    }
                }
            }
        }
    }

    synced_count
}

pub fn pull_app_from_gdrive(
    steam_dir: &Path,
    account_id: u32,
    app_id: u32,
    vault: &crate::vault::VaultStorage,
    log_path: Option<&PathBuf>,
) -> bool {
    let key = (account_id, app_id);
    {
        let mut guard = match ACTIVE_PULLS.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let set = guard.get_or_insert_with(HashSet::new);
        if !set.insert(key) {
            return false;
        }
    }

    struct PullGuard((u32, u32));
    impl Drop for PullGuard {
        fn drop(&mut self) {
            if let Ok(mut guard) = ACTIVE_PULLS.lock() {
                if let Some(ref mut set) = *guard {
                    set.remove(&self.0);
                }
            }
        }
    }
    let _guard = PullGuard(key);

    let token_path = steam_dir.join("chronokeep_gdrive.json");
    if !token_path.exists() {
        return false;
    }
    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive] Falha ao renovar access_token com refresh_token.",
            );
        }
        return false;
    };

    let Some(root_id) = find_drive_folder(&token, "ChronoKeep", None) else {
        return false;
    };
    let Some(acct_id) = find_drive_folder(&token, &account_id.to_string(), Some(&root_id)) else {
        return false;
    };
    let Some(app_folder_id) = find_drive_folder(&token, &app_id.to_string(), Some(&acct_id)) else {
        return false;
    };

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive] Sincronizando saves da nuvem para o Vault local (app={})",
                app_id
            ),
        );
    }

    let count = pull_drive_folder_recursive(
        &token,
        &app_folder_id,
        "",
        account_id,
        app_id,
        vault,
        log_path,
    );

    // Reconcilia o manifest local após baixar os arquivos da nuvem
    let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
    let _ = vault.save_manifest(account_id, app_id, &manifest);

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive] Sincronizacao concluida: {} arquivos novos/atualizados para app={}",
                count, app_id
            ),
        );
    }

    true
}

pub fn start_background_pull_all(
    steam_dir: PathBuf,
    account_id: u32,
    _apps: Vec<u32>,
    vault: crate::vault::VaultStorage,
    log_path: Option<PathBuf>,
) {
    if PULL_ALL_ACTIVE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        struct PullAllGuard;
        impl Drop for PullAllGuard {
            fn drop(&mut self) {
                PULL_ALL_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _all_guard = PullAllGuard;
        let mut token_path = steam_dir.join("chronokeep_gdrive.json");
        if !token_path.exists() {
            if let Ok(cur) = std::env::current_exe() {
                if let Some(parent) = cur.parent() {
                    let candidate = parent.join("chronokeep_gdrive.json");
                    if candidate.exists() {
                        token_path = candidate;
                    }
                }
            }
        }
        if !token_path.exists() {
            return;
        }

        let Some(token) = get_valid_access_token(&token_path) else {
            if let Some(ref lp) = log_path {
                crate::log_msg(lp, "[Google Drive] Falha ao renovar token de acesso no pull em background.");
            }
            return;
        };

        let Some(root_id) = find_drive_folder(&token, "ChronoKeep", None) else {
            return;
        };
        let Some(acct_id) = find_drive_folder(&token, &account_id.to_string(), Some(&root_id)) else {
            return;
        };

        if let Some(ref lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive] Descobrindo jogos com saves sincronizados no Drive para account_id={}",
                    account_id
                ),
            );
        }

        // Lista as pastas de jogos diretamente no Drive em uma única consulta rápida
        let app_folders = list_drive_files(&token, &acct_id);
        for item in app_folders {
            if item.mime_type != "application/vnd.google-apps.folder" {
                continue;
            }
            let Ok(app_id) = item.name.parse::<u32>() else {
                continue;
            };
            if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
                continue;
            }

            if let Some(ref lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive] Sincronizando saves da nuvem para app={}",
                        app_id
                    ),
                );
            }

            let count = pull_drive_folder_recursive(
                &token,
                &item.id,
                "",
                account_id,
                app_id,
                &vault,
                log_path.as_ref(),
            );

            let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
            let _ = vault.save_manifest(account_id, app_id, &manifest);

            if count > 0 {
                if let Some(ref lp) = log_path {
                    crate::log_msg(
                        lp,
                        &format!(
                            "[Google Drive] Sincronizacao concluida: {} arquivos novos/atualizados para app={}",
                            count, app_id
                        ),
                    );
                }
            }
        }
    });
}
