//! Native Google Drive Synchronization Engine for `chrono-daemon.exe`.
//! Reuses the shared WinHTTP & Google Drive REST API client from `cr-common`.

use cr_common::gdrive::{
    clear_folder_cache, download_drive_file, find_drive_folder, find_or_create_drive_folder,
    get_valid_access_token, list_drive_files, upload_file_multipart,
};
use cr_proto::sha1::sha1_digest;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static ACTIVE_PULLS: Mutex<Option<HashSet<(u32, u32)>>> = Mutex::new(None);
static PULL_ALL_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static APP_UPLOAD_LOCKS: Mutex<Option<HashMap<(u32, u32), Arc<Mutex<()>>>>> = Mutex::new(None);
static LAST_UPLOADED_MANIFEST: Mutex<Option<HashMap<(u32, u32), [u8; 20]>>> = Mutex::new(None);
static LAST_PULLED_AT: Mutex<Option<HashMap<(u32, u32), Instant>>> = Mutex::new(None);

const PULL_COOLDOWN: Duration = Duration::from_secs(300);

fn get_app_upload_lock(account_id: u32, app_id: u32) -> Arc<Mutex<()>> {
    let mut guard = APP_UPLOAD_LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    Arc::clone(
        map.entry((account_id, app_id))
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

fn sync_manifest_locked(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    manifest_path: &Path,
    log_path: Option<&PathBuf>,
) {
    let Ok(m_bytes) = fs::read(manifest_path) else {
        return;
    };
    let hash = sha1_digest(&m_bytes);
    {
        if let Ok(guard) = LAST_UPLOADED_MANIFEST.lock() {
            if let Some(ref map) = *guard {
                if map.get(&(account_id, app_id)) == Some(&hash) {
                    return;
                }
            }
        }
    }

    if sync_file_to_gdrive(exe_dir, account_id, app_id, "manifest.tsv", &m_bytes, log_path) {
        if let Ok(mut guard) = LAST_UPLOADED_MANIFEST.lock() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert((account_id, app_id), hash);
        }
    }
}

/// Serializes Google Drive uploads per `(account_id, app_id)` so concurrent file commits
/// never race on folder creation or overwrite `manifest.tsv` with stale snapshots.
pub fn sync_committed_file_and_manifest(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    file_bytes: &[u8],
    manifest_path: &Path,
    log_path: Option<&PathBuf>,
) {
    let app_lock = get_app_upload_lock(account_id, app_id);
    let _guard = app_lock.lock().unwrap_or_else(|e| e.into_inner());
    let _ = sync_file_to_gdrive(exe_dir, account_id, app_id, rel_filename, file_bytes, log_path);
    sync_manifest_locked(exe_dir, account_id, app_id, manifest_path, log_path);
}

/// Uploads a batch of modified local files followed by the latest `manifest.tsv` under the per-app lock.
pub fn sync_batch_files_and_manifest(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    files: &[(String, Vec<u8>)],
    manifest_path: &Path,
    log_path: Option<&PathBuf>,
) {
    let app_lock = get_app_upload_lock(account_id, app_id);
    let _guard = app_lock.lock().unwrap_or_else(|e| e.into_inner());
    for (rel_name, bytes) in files {
        let _ = sync_file_to_gdrive(exe_dir, account_id, app_id, rel_name, bytes, log_path);
    }
    sync_manifest_locked(exe_dir, account_id, app_id, manifest_path, log_path);
}

/// Ensures the latest on-disk `manifest.tsv` is uploaded if its content changed since the last upload.
pub fn sync_manifest_if_changed(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    manifest_path: &Path,
    log_path: Option<&PathBuf>,
) {
    let app_lock = get_app_upload_lock(account_id, app_id);
    let _guard = app_lock.lock().unwrap_or_else(|e| e.into_inner());
    sync_manifest_locked(exe_dir, account_id, app_id, manifest_path, log_path);
}

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
    cloud_is_newer: &mut bool,
    log_path: Option<&PathBuf>,
) -> usize {
    let items = list_drive_files(access_token, folder_id);
    let mut synced_count = 0;

    // Na raiz do app, processa manifest.tsv primeiro para adotar CN e root_tokens antes de baixar saves
    if rel_prefix.is_empty() {
        for item in &items {
            if item.mime_type != "application/vnd.google-apps.folder"
                && item.name.eq_ignore_ascii_case("manifest.tsv")
            {
                if let Some(bytes) = download_drive_file(access_token, &item.id) {
                    let cloud_str = String::from_utf8_lossy(&bytes);
                    let parsed = cr_common::manifest::parse_manifest(&cloud_str);
                    let cloud_cn = parsed.change_number;
                    let current_manifest = vault.load_and_reconcile_manifest(account_id, app_id);
                    if cloud_cn > current_manifest.change_number {
                        *cloud_is_newer = true;
                        let m_path = vault.manifest_path(account_id, app_id);
                        if let Some(parent) = m_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&m_path, &bytes);
                        if let Ok(mut guard) = LAST_UPLOADED_MANIFEST.lock() {
                            let map = guard.get_or_insert_with(HashMap::new);
                            map.insert((account_id, app_id), sha1_digest(&bytes));
                        }
                    } else if !vault.manifest_path(account_id, app_id).exists() {
                        let m_path = vault.manifest_path(account_id, app_id);
                        if let Some(parent) = m_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&m_path, &bytes);
                    }
                }
            }
        }
    }

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
                cloud_is_newer,
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

            if rel_file.eq_ignore_ascii_case("manifest.tsv") {
                continue;
            }

            // Apenas arquivos sem root_token (root=0) pertencem a Steam\userdata\<acct>\<app>\remote.
            // Jogos AutoCloud (ex: %WinAppDataLocal%, %WinMyDocuments%) são gerenciados pela Steam via ClientFileDownload.
            let file_root_tok = vault.effective_root_token_for_file(account_id, app_id, &rel_file);
            let uses_userdata_remote = file_root_tok.is_empty();

            // Verifica integridade com o Vault e (se root=0) com userdata/remote
            let vault_file_path = vault.resolve_file_disk_path(account_id, app_id, &rel_file);
            let user_dest_path = if uses_userdata_remote {
                vault.steam_path.as_ref().map(|sp| {
                    sp.join("userdata")
                        .join(account_id.to_string())
                        .join(app_id.to_string())
                        .join("remote")
                        .join(&rel_file)
                })
            } else {
                None
            };

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

            // 3. Se o arquivo já existe no Vault local e a nuvem NÃO possui CN superior, preserva o local (monotonicidade de CN)
            let local_exists_in_vault = vault_file_path.as_ref().map(|p| p.exists()).unwrap_or(false);
            if local_exists_in_vault && !*cloud_is_newer {
                continue;
            }

            // 4. Nem o Vault nem o userdata coincidem com a nuvem (e nuvem é mais recente ou arquivo é inédito): realiza o download
            if let Some(bytes) = download_drive_file(access_token, &item.id) {
                if vault
                    .write_save_file(account_id, app_id, &rel_file, &bytes)
                    .is_ok()
                {
                    synced_count += 1;
                    if let Some(ref user_dest) = user_dest_path {
                        if !user_dest.exists() || *cloud_is_newer {
                            if let Some(p) = user_dest.parent() {
                                let _ = fs::create_dir_all(p);
                            }
                            let _ = fs::write(user_dest, &bytes);
                        }
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

    let mut cloud_is_newer = false;
    let count = pull_drive_folder_recursive(
        &token,
        &app_folder_id,
        "",
        account_id,
        app_id,
        vault,
        &mut cloud_is_newer,
        log_path,
    );

    if let Ok(mut guard) = LAST_PULLED_AT.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert((account_id, app_id), Instant::now());
    }

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
    apps: Vec<u32>,
    vault: crate::vault::VaultStorage,
    log_path: Option<PathBuf>,
) {
    let managed_set: HashSet<u32> = apps
        .into_iter()
        .filter(|&id| id != 0 && id != 7 && id != 760 && id != 241100 && id != 241300)
        .collect();
    if managed_set.is_empty() {
        return;
    }

    // Verifica se há pelo menos um app gerenciado que ainda não passou pelo pull recente
    let has_pending_apps = {
        if let Ok(guard) = LAST_PULLED_AT.lock() {
            if let Some(ref map) = *guard {
                managed_set.iter().any(|&app_id| {
                    map.get(&(account_id, app_id))
                        .map(|t| t.elapsed() >= PULL_COOLDOWN)
                        .unwrap_or(true)
                })
            } else {
                true
            }
        } else {
            true
        }
    };
    if !has_pending_apps {
        return;
    }

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
                    "[Google Drive] Descobrindo jogos gerenciados com saves no Drive para account_id={}",
                    account_id
                ),
            );
        }

        // Lista as pastas de jogos diretamente no Drive em uma única consulta rápida
        let app_folders = list_drive_files(&token, &acct_id);
        let now = Instant::now();

        for item in app_folders {
            if item.mime_type != "application/vnd.google-apps.folder" {
                continue;
            }
            let Ok(app_id) = item.name.parse::<u32>() else {
                continue;
            };
            // Ignora AppIDs internos da Valve e jogos oficiais NÃO gerenciados pelo ChronoKeep
            if app_id == 0
                || app_id == 7
                || app_id == 760
                || app_id == 241100
                || app_id == 241300
                || !managed_set.contains(&app_id)
            {
                continue;
            }

            let recently_pulled = {
                if let Ok(guard) = LAST_PULLED_AT.lock() {
                    guard
                        .as_ref()
                        .and_then(|m| m.get(&(account_id, app_id)))
                        .map(|t| t.elapsed() < PULL_COOLDOWN)
                        .unwrap_or(false)
                } else {
                    false
                }
            };
            if recently_pulled {
                continue;
            }

            // Trava de concorrência por (account_id, app_id) compartilhada com pull_app_from_gdrive
            let key = (account_id, app_id);
            let acquired = {
                if let Ok(mut guard) = ACTIVE_PULLS.lock() {
                    guard.get_or_insert_with(HashSet::new).insert(key)
                } else {
                    false
                }
            };
            if !acquired {
                continue;
            }

            struct ScopedAppPullGuard((u32, u32));
            impl Drop for ScopedAppPullGuard {
                fn drop(&mut self) {
                    if let Ok(mut guard) = ACTIVE_PULLS.lock() {
                        if let Some(ref mut set) = *guard {
                            set.remove(&self.0);
                        }
                    }
                }
            }
            let _app_guard = ScopedAppPullGuard(key);

            if let Some(ref lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive] Sincronizando saves da nuvem para app={}",
                        app_id
                    ),
                );
            }

            let mut cloud_is_newer = false;
            let count = pull_drive_folder_recursive(
                &token,
                &item.id,
                "",
                account_id,
                app_id,
                &vault,
                &mut cloud_is_newer,
                log_path.as_ref(),
            );

            let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
            let _ = vault.save_manifest(account_id, app_id, &manifest);

            if let Ok(mut guard) = LAST_PULLED_AT.lock() {
                guard
                    .get_or_insert_with(HashMap::new)
                    .insert((account_id, app_id), Instant::now());
            }

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

        // Marca também os apps gerenciados sem pasta no Drive como verificados nesta janela
        if let Ok(mut guard) = LAST_PULLED_AT.lock() {
            let map = guard.get_or_insert_with(HashMap::new);
            for &app_id in &managed_set {
                map.entry((account_id, app_id)).or_insert(now);
            }
        }
    });
}
