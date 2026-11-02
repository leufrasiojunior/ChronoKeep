//! Steam Cloud RPC Execution Engine (`cr-service`).
//! Handles all `Cloud.*` RPC requests forwarded by `chronokeep.dll` (`cr-bridge-dll`)
//! and coordinates staging blobs uploaded/downloaded via HTTP by `steamclient64.dll`.

use crate::gdrive::{BatchUploadOutcome, CloudManifestFetch};
use crate::vault::{now_unix, AppManifest, VaultStorage};
use cr_proto::cloud_messages::*;
use cr_proto::sha1::sha1_digest;
use cr_proto::wire::{find_field, parse_message, ProtoField};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct StagedUpload {
    pub root_token: String,
    pub timestamp: u64,
    pub platforms_to_sync: u32,
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ActiveBatch {
    pub batch_id: u64,
    pub base_cn: u64,
    pub assigned_cn: u64,
    pub app_build_id: u64,
    pub committed_files: HashMap<String, Vec<u8>>,
    pub deleted_files: HashSet<String>,
}

pub struct RpcEngine {
    pub vault: VaultStorage,
    pub listen_port: u16,
    pub account_id: u32,
    pub namespace_apps: HashSet<u32>,
    pub log_path: Option<PathBuf>,
    next_batch_id: u64,
    active_batches: HashMap<(u32, u32), ActiveBatch>,
    staged_uploads: HashMap<(u32, u32, String), StagedUpload>,
    cloud_manifests: HashMap<(u32, u32), AppManifest>,
    full_manifest_sent: HashSet<(u32, u32)>,
    synced_apps_from_cloud: HashSet<(u32, u32)>,
}

pub fn is_system_app_id(app_id: u32) -> bool {
    matches!(app_id, 0 | 7 | 760 | 241100 | 241300)
}

/// Dispatches an incoming RPC without holding `engine.lock()` across Google Drive network I/O.
pub fn dispatch_rpc(
    engine: &Arc<Mutex<RpcEngine>>,
    method: &str,
    app_id: u32,
    req_account_id: u32,
    req_bytes: &[u8],
) -> Option<(i32, Vec<u8>)> {
    let fields = parse_message(req_bytes);
    let resolved_app_id = if app_id != 0 {
        app_id
    } else {
        extract_app_id(method, &fields)
    };
    if is_system_app_id(resolved_app_id) {
        return None;
    }

    let (account_id, vault, log_path, steam_dir) = {
        let mut guard = engine.lock().ok()?;
        if !guard.namespace_apps.is_empty() && !guard.namespace_apps.contains(&resolved_app_id) {
            return None;
        }
        let acct = guard.resolve_and_remember_account(req_account_id);
        let sd = guard.effective_steam_dir();
        (acct, guard.vault.clone(), guard.log_path.clone(), sd)
    };

    match method {
        RPC_GET_CHANGELIST => Some(handle_get_changelist_unlocked(
            engine,
            account_id,
            resolved_app_id,
            &fields,
            vault,
            steam_dir,
            log_path,
        )),
        RPC_BEGIN_BATCH => Some(handle_begin_batch_unlocked(
            engine,
            account_id,
            resolved_app_id,
            &fields,
            vault,
            steam_dir,
            log_path,
        )),
        RPC_COMMIT_UPLOAD => Some(handle_commit_upload_unlocked(
            engine,
            account_id,
            resolved_app_id,
            &fields,
            vault,
            steam_dir,
            log_path,
        )),
        RPC_DELETE_FILE => Some(handle_delete_file_unlocked(
            engine,
            account_id,
            resolved_app_id,
            &fields,
            vault,
            steam_dir,
            log_path,
        )),
        RPC_COMPLETE_BATCH => Some(handle_complete_batch_unlocked(
            engine,
            account_id,
            resolved_app_id,
            &fields,
            vault,
            steam_dir,
            log_path,
        )),
        RPC_SUSPEND_SESSION | RPC_EXIT_SYNC => {
            notify_exit_sync(engine, account_id, resolved_app_id, false);
            Some((ERESULT_OK, Vec::new()))
        }
        RPC_LAUNCH_INTENT => Some((ERESULT_OK, build_launch_intent_response())),
        RPC_RESUME_SESSION => Some((ERESULT_OK, Vec::new())),
        RPC_QUOTA_USAGE => {
            let guard = engine.lock().ok()?;
            Some(guard.on_quota_usage(account_id, resolved_app_id))
        }
        RPC_BEGIN_UPLOAD => {
            let mut guard = engine.lock().ok()?;
            Some(guard.on_begin_upload(account_id, resolved_app_id, &fields))
        }
        RPC_FILE_DOWNLOAD => {
            let mut guard = engine.lock().ok()?;
            Some(guard.on_file_download(account_id, resolved_app_id, &fields, req_bytes))
        }
        RPC_CONFLICT | RPC_TRANSFER_REPORT | RPC_SYNC_STATS => Some((ERESULT_OK, Vec::new())),
        _ => None,
    }
}

/// Retrieves a save blob for `GET /download/<acct>/<app>/<file>` without holding `engine.lock()`
/// during on-demand Google Drive downloads, verifying SHA-1 integrity and returning `None` (`404`) if unavailable.
pub fn retrieve_download_blob_with_cloud(
    engine: &Arc<Mutex<RpcEngine>>,
    account_id: u32,
    app_id: u32,
    clean_filename: &str,
) -> Option<Vec<u8>> {
    let (acct, staged_opt, expected_rec, vault, steam_dir, log_path) = {
        let guard = engine.lock().ok()?;
        let acct = if account_id != 0 {
            account_id
        } else {
            guard.account_id.max(1)
        };
        let staged_opt = guard
            .staged_uploads
            .get(&(acct, app_id, clean_filename.to_string()))
            .and_then(|s| s.data.clone());
        let expected_rec = guard.lookup_expected_file_record(acct, app_id, clean_filename);
        let sd = guard.effective_steam_dir();
        (
            acct,
            staged_opt,
            expected_rec,
            guard.vault.clone(),
            sd,
            guard.log_path.clone(),
        )
    };

    if let Some(staged_bytes) = staged_opt {
        return Some(staged_bytes);
    }

    let target_name = expected_rec
        .as_ref()
        .map(|r| r.filename.as_str())
        .unwrap_or(clean_filename);

    // 1. Verifica se já existe no Vault local com SHA-1 íntegro (quando esperado)
    if let Some(local_bytes) = vault.read_save_file(acct, app_id, target_name) {
        if let Some(ref exp) = expected_rec {
            if sha1_digest(&local_bytes) == exp.sha1 {
                return Some(local_bytes);
            }
        } else {
            return Some(local_bytes);
        }
    }

    // 2. Se o arquivo esperado já existir localmente na pasta do jogo com o mesmo SHA-1, semeia o Vault e serve
    if let Some(ref exp) = expected_rec {
        if exp.raw_size == 0 && exp.sha1 == sha1_digest(&[]) {
            let _ = vault.write_save_file(acct, app_id, target_name, &[]);
            let _ = vault.try_promote_cloud_manifest_if_complete(acct, app_id);
            return Some(Vec::new());
        }

        if let Some(lp) =
            vault.resolve_autocloud_local_path(acct, app_id, &exp.root_token, target_name)
        {
            if let Ok(disk_bytes) = std::fs::read(&lp) {
                if sha1_digest(&disk_bytes) == exp.sha1 {
                    let _ = vault.write_save_file(acct, app_id, target_name, &disk_bytes);
                    let _ = vault.try_promote_cloud_manifest_if_complete(acct, app_id);
                    return Some(disk_bytes);
                }
            }
        }
    }

    // 3. Baixa sob demanda do Google Drive validando estritamente o SHA-1 esperado
    if let Some(ref sd) = steam_dir {
        let exp_sha = expected_rec.as_ref().map(|r| r.sha1);
        let root_tok = expected_rec
            .as_ref()
            .map(|r| r.root_token.clone())
            .unwrap_or_else(|| vault.effective_root_token_for_file(acct, app_id, target_name));

        if let Some(downloaded) = crate::gdrive::download_app_file_on_demand(
            sd,
            acct,
            app_id,
            target_name,
            exp_sha,
            &root_tok,
            &vault,
            log_path.as_ref(),
        ) {
            return Some(downloaded);
        }
    }

    if let Some(ref lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Download] ERRO 404: Arquivo indisponivel no Vault e no Google Drive: app={} file='{}'",
                app_id, target_name
            ),
        );
    }
    None
}

/// Reconciles unbatched `userdata/remote` saves when a game exits (`defer_for_steam_batch = true` for `/v1/game_state`).
pub fn notify_exit_sync(
    engine: &Arc<Mutex<RpcEngine>>,
    req_account_id: u32,
    app_id: u32,
    defer_for_steam_batch: bool,
) {
    if is_system_app_id(app_id) {
        return;
    }

    let eng = Arc::clone(engine);
    std::thread::spawn(move || {
        if defer_for_steam_batch {
            // Dá prioridade ao lote oficial BeginAppUploadBatch da Steam após o fechamento do jogo
            std::thread::sleep(Duration::from_millis(1800));
        }

        let (account_id, has_active_batch, vault, steam_dir, log_path) = {
            let Ok(guard) = eng.lock() else {
                return;
            };
            if !guard.namespace_apps.is_empty() && !guard.namespace_apps.contains(&app_id) {
                return;
            }
            let acct = if req_account_id != 0 {
                req_account_id
            } else {
                guard.account_id
            };
            if acct == 0 {
                return;
            }
            let active = guard.active_batches.contains_key(&(acct, app_id));
            (
                acct,
                active,
                guard.vault.clone(),
                guard.effective_steam_dir(),
                guard.log_path.clone(),
            )
        };

        if has_active_batch {
            return;
        }
        let _ = crate::gdrive::wait_for_app_upload(account_id, app_id, Duration::from_secs(10));

        let modified = vault.sync_local_saves_to_vault(account_id, app_id);
        if modified.is_empty() {
            return;
        }

        let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
        let base_cn = manifest.change_number;
        let assigned_cn = base_cn.saturating_add(1);

        if let Some(ref lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[ExitSync] Detectados {} arquivo(s) novos em userdata/remote para app={}; enviando lote CN={}...",
                    modified.len(),
                    app_id,
                    assigned_cn
                ),
            );
        }

        if let Some(sd) = steam_dir {
            if crate::gdrive::is_gdrive_configured(&sd) {
                let outcome = crate::gdrive::execute_batch_upload_bounded(
                    sd,
                    account_id,
                    app_id,
                    base_cn,
                    assigned_cn,
                    manifest.app_build_id,
                    modified,
                    Vec::new(),
                    vault,
                    log_path,
                    Duration::from_secs(30),
                );
                if let BatchUploadOutcome::Succeeded(final_m) = outcome {
                    if let Ok(mut guard) = eng.lock() {
                        guard.cloud_manifests.insert((account_id, app_id), final_m);
                    }
                }
                return;
            }
        }

        let mut updated = manifest;
        updated.change_number = assigned_cn;
        let _ = vault.save_manifest(account_id, app_id, &updated);
    });
}

fn handle_get_changelist_unlocked(
    engine: &Arc<Mutex<RpcEngine>>,
    account_id: u32,
    app_id: u32,
    fields: &[ProtoField<'_>],
    vault: VaultStorage,
    steam_dir: Option<PathBuf>,
    log_path: Option<PathBuf>,
) -> (i32, Vec<u8>) {
    let _ = crate::gdrive::wait_for_app_upload(account_id, app_id, Duration::from_secs(5));

    let key = (account_id, app_id);
    let already_synced = engine
        .lock()
        .map(|g| g.synced_apps_from_cloud.contains(&key))
        .unwrap_or(false);

    let cloud_fetch = if !already_synced {
        if let Some(ref sd) = steam_dir {
            let res = crate::gdrive::fetch_cloud_manifest_bounded(
                sd,
                account_id,
                app_id,
                &vault,
                log_path.as_ref(),
                Duration::from_secs(8),
            );
            if matches!(res, CloudManifestFetch::Manifest(_)) {
                let sd_bg = sd.clone();
                let v_bg = vault.clone();
                let lp_bg = log_path.clone();
                std::thread::spawn(move || {
                    let _ = crate::gdrive::pull_app_from_gdrive(
                        &sd_bg,
                        account_id,
                        app_id,
                        &v_bg,
                        lp_bg.as_ref(),
                    );
                });
            }
            Some(res)
        } else {
            Some(CloudManifestFetch::NotConfigured)
        }
    } else {
        None
    };

    let Ok(mut guard) = engine.lock() else {
        return (ERESULT_FAIL, Vec::new());
    };
    guard.finish_get_changelist(account_id, app_id, fields, cloud_fetch)
}

fn handle_begin_batch_unlocked(
    engine: &Arc<Mutex<RpcEngine>>,
    account_id: u32,
    app_id: u32,
    fields: &[ProtoField<'_>],
    vault: VaultStorage,
    steam_dir: Option<PathBuf>,
    log_path: Option<PathBuf>,
) -> (i32, Vec<u8>) {
    let _ = crate::gdrive::wait_for_app_upload(account_id, app_id, Duration::from_secs(5));

    let cloud_fetch = steam_dir.as_ref().map(|sd| {
        crate::gdrive::fetch_cloud_manifest_bounded(
            sd,
            account_id,
            app_id,
            &vault,
            log_path.as_ref(),
            Duration::from_secs(5),
        )
    });

    let Ok(mut guard) = engine.lock() else {
        return (ERESULT_FAIL, Vec::new());
    };
    guard.finish_begin_batch(account_id, app_id, fields, cloud_fetch)
}

fn handle_commit_upload_unlocked(
    engine: &Arc<Mutex<RpcEngine>>,
    account_id: u32,
    app_id: u32,
    fields: &[ProtoField<'_>],
    vault: VaultStorage,
    steam_dir: Option<PathBuf>,
    log_path: Option<PathBuf>,
) -> (i32, Vec<u8>) {
    let standalone = {
        let Ok(mut guard) = engine.lock() else {
            return (ERESULT_OK, build_commit_upload_response(false));
        };
        match guard.stage_or_commit_upload_inner(account_id, app_id, fields) {
            CommitInnerAction::Done(resp) => return resp,
            CommitInnerAction::StandaloneUpload {
                clean_name,
                bytes,
                base_cn,
                assigned_cn,
                app_build_id,
            } => (clean_name, bytes, base_cn, assigned_cn, app_build_id),
        }
    };

    let (clean_name, bytes, base_cn, assigned_cn, app_build_id) = standalone;
    if let Some(sd) = steam_dir {
        if crate::gdrive::is_gdrive_configured(&sd) {
            let outcome = crate::gdrive::execute_batch_upload_bounded(
                sd,
                account_id,
                app_id,
                base_cn,
                assigned_cn,
                app_build_id,
                vec![(clean_name, bytes)],
                Vec::new(),
                vault,
                log_path,
                Duration::from_secs(11),
            );
            return match outcome {
                BatchUploadOutcome::Succeeded(final_m) => {
                    if let Ok(mut guard) = engine.lock() {
                        guard.cloud_manifests.insert((account_id, app_id), final_m);
                        guard.full_manifest_sent.insert((account_id, app_id));
                    }
                    (ERESULT_OK, build_commit_upload_response(true))
                }
                BatchUploadOutcome::StillInProgress => {
                    (ERESULT_OK, build_commit_upload_response(true))
                }
                BatchUploadOutcome::Failed => (ERESULT_OK, build_commit_upload_response(false)),
            };
        }
    }

    let mut manifest = vault.load_and_reconcile_manifest(account_id, app_id);
    manifest.change_number = assigned_cn;
    let _ = vault.save_manifest(account_id, app_id, &manifest);
    if let Ok(mut guard) = engine.lock() {
        guard.full_manifest_sent.insert((account_id, app_id));
    }
    (ERESULT_OK, build_commit_upload_response(true))
}

fn handle_delete_file_unlocked(
    engine: &Arc<Mutex<RpcEngine>>,
    account_id: u32,
    app_id: u32,
    fields: &[ProtoField<'_>],
    vault: VaultStorage,
    steam_dir: Option<PathBuf>,
    log_path: Option<PathBuf>,
) -> (i32, Vec<u8>) {
    let standalone_del = {
        let Ok(mut guard) = engine.lock() else {
            return (ERESULT_FAIL, Vec::new());
        };
        guard.delete_file_inner(account_id, app_id, fields)
    };

    if let Some((clean_name, base_cn, assigned_cn, app_build_id)) = standalone_del {
        if let Some(sd) = steam_dir {
            if crate::gdrive::is_gdrive_configured(&sd) {
                let outcome = crate::gdrive::execute_batch_upload_bounded(
                    sd,
                    account_id,
                    app_id,
                    base_cn,
                    assigned_cn,
                    app_build_id,
                    Vec::new(),
                    vec![clean_name],
                    vault,
                    log_path,
                    Duration::from_secs(11),
                );
                if let BatchUploadOutcome::Failed = outcome {
                    return (ERESULT_FAIL, Vec::new());
                }
                if let BatchUploadOutcome::Succeeded(final_m) = outcome {
                    if let Ok(mut guard) = engine.lock() {
                        guard.cloud_manifests.insert((account_id, app_id), final_m);
                    }
                }
                return (ERESULT_OK, Vec::new());
            }
        }
        let mut manifest = vault.load_and_reconcile_manifest(account_id, app_id);
        manifest.change_number = assigned_cn;
        let _ = vault.save_manifest(account_id, app_id, &manifest);
    }

    (ERESULT_OK, Vec::new())
}

fn handle_complete_batch_unlocked(
    engine: &Arc<Mutex<RpcEngine>>,
    account_id: u32,
    app_id: u32,
    fields: &[ProtoField<'_>],
    vault: VaultStorage,
    steam_dir: Option<PathBuf>,
    log_path: Option<PathBuf>,
) -> (i32, Vec<u8>) {
    // Campo 3 em CCloud_CompleteAppUploadBatch_Request = batch_eresult (1 = k_EResultOK)
    let steam_batch_result = find_field(fields, 3).and_then(|f| f.as_u32());

    let batch_opt = {
        let Ok(mut guard) = engine.lock() else {
            return (ERESULT_FAIL, Vec::new());
        };
        guard.active_batches.remove(&(account_id, app_id))
    };

    let Some(batch) = batch_opt else {
        return (ERESULT_OK, Vec::new());
    };

    if let Some(res) = steam_batch_result {
        if res != 1 {
            if let Some(ref lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[CompleteBatch] Lote batch={} abortado pela Steam (batch_eresult={}) em app={}; mantendo CN={}.",
                        batch.batch_id, res, app_id, batch.base_cn
                    ),
                );
            }
            return (ERESULT_OK, Vec::new());
        }
    }

    let gdrive_enabled = steam_dir
        .as_ref()
        .map(|sd| crate::gdrive::is_gdrive_configured(sd))
        .unwrap_or(false);

    if !gdrive_enabled {
        let mut manifest = vault.load_and_reconcile_manifest(account_id, app_id);
        manifest.change_number = batch.assigned_cn;
        if batch.app_build_id != 0 {
            manifest.app_build_id = batch.app_build_id;
        }
        let _ = vault.save_manifest(account_id, app_id, &manifest);
        if let Ok(mut guard) = engine.lock() {
            guard.full_manifest_sent.insert((account_id, app_id));
        }
        return (ERESULT_OK, Vec::new());
    }

    let Some(sd) = steam_dir else {
        return (ERESULT_FAIL, Vec::new());
    };

    let committed_vec: Vec<(String, Vec<u8>)> = batch.committed_files.into_iter().collect();
    let deleted_vec: Vec<String> = batch.deleted_files.into_iter().collect();

    let outcome = crate::gdrive::execute_batch_upload_bounded(
        sd,
        account_id,
        app_id,
        batch.base_cn,
        batch.assigned_cn,
        batch.app_build_id,
        committed_vec,
        deleted_vec,
        vault,
        log_path,
        Duration::from_secs(11),
    );

    match outcome {
        BatchUploadOutcome::Succeeded(final_manifest) => {
            if let Ok(mut guard) = engine.lock() {
                guard
                    .cloud_manifests
                    .insert((account_id, app_id), final_manifest);
                guard.full_manifest_sent.insert((account_id, app_id));
                guard.synced_apps_from_cloud.insert((account_id, app_id));
            }
            (ERESULT_OK, Vec::new())
        }
        BatchUploadOutcome::StillInProgress => {
            if let Ok(mut guard) = engine.lock() {
                guard.full_manifest_sent.insert((account_id, app_id));
            }
            (ERESULT_OK, Vec::new())
        }
        BatchUploadOutcome::Failed => {
            if let Ok(mut guard) = engine.lock() {
                guard.full_manifest_sent.remove(&(account_id, app_id));
                guard.synced_apps_from_cloud.remove(&(account_id, app_id));
            }
            (ERESULT_FAIL, Vec::new())
        }
    }
}

enum CommitInnerAction {
    Done((i32, Vec<u8>)),
    StandaloneUpload {
        clean_name: String,
        bytes: Vec<u8>,
        base_cn: u64,
        assigned_cn: u64,
        app_build_id: u64,
    },
}

impl RpcEngine {
    pub fn new(vault: VaultStorage, listen_port: u16, log_path: Option<PathBuf>) -> Self {
        Self {
            vault,
            listen_port,
            account_id: 0,
            namespace_apps: HashSet::new(),
            log_path,
            next_batch_id: 1,
            active_batches: HashMap::new(),
            staged_uploads: HashMap::new(),
            cloud_manifests: HashMap::new(),
            full_manifest_sent: HashSet::new(),
            synced_apps_from_cloud: HashSet::new(),
        }
    }

    pub fn effective_steam_dir(&self) -> Option<PathBuf> {
        self.vault.steam_path.clone().or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        })
    }

    pub fn resolve_and_remember_account(&mut self, req_account_id: u32) -> u32 {
        if req_account_id != 0 && self.account_id == 0 {
            self.account_id = req_account_id;
        }
        if req_account_id != 0 {
            req_account_id
        } else if self.account_id != 0 {
            self.account_id
        } else {
            1
        }
    }

    pub fn update_session(
        &mut self,
        steam_path: Option<String>,
        account_id: u32,
        apps: Vec<u32>,
    ) {
        if let Some(sp) = steam_path {
            if !sp.is_empty() {
                self.vault.set_steam_path(PathBuf::from(sp));
            }
        }
        if account_id != 0 {
            self.account_id = account_id;
        }
        if !apps.is_empty() {
            self.namespace_apps = apps
                .into_iter()
                .filter(|&id| !is_system_app_id(id))
                .collect();
        }
    }

    pub fn store_uploaded_blob(
        &mut self,
        account_id: u32,
        app_id: u32,
        clean_filename: String,
        bytes: Vec<u8>,
    ) {
        let acct = if account_id != 0 {
            account_id
        } else {
            self.account_id
        };
        let entry = self
            .staged_uploads
            .entry((acct, app_id, clean_filename))
            .or_insert_with(|| StagedUpload {
                root_token: String::new(),
                timestamp: now_unix(),
                platforms_to_sync: 0xFFFF_FFFF,
                data: None,
            });
        entry.data = Some(bytes);
    }

    fn lookup_expected_file_record(
        &self,
        account_id: u32,
        app_id: u32,
        clean_filename: &str,
    ) -> Option<CloudFileRecord> {
        let local_m = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let cloud_m = self
            .cloud_manifests
            .get(&(account_id, app_id))
            .cloned()
            .or_else(|| self.vault.load_cloud_manifest(account_id, app_id));

        if let Some(ref cm) = cloud_m {
            if cm.change_number >= local_m.change_number {
                if let Some(rec) = find_record_ci(&cm.files, clean_filename) {
                    return Some(rec.clone());
                }
            }
        }

        if let Some(rec) = find_record_ci(&local_m.files, clean_filename) {
            return Some(rec.clone());
        }

        if let Some(ref cm) = cloud_m {
            if let Some(rec) = find_record_ci(&cm.files, clean_filename) {
                return Some(rec.clone());
            }
        }

        let (_, target_clean) = split_root_token(clean_filename);

        // Fallback: consulta metadados registrados no remotecache.vdf da Steam
        let vdf_records = self.vault.read_remotecache_records(account_id, app_id);
        if let Some(rec) = find_record_ci(&vdf_records, clean_filename) {
            return Some(rec.clone());
        }

        // Fallback: verifica se o arquivo já reside no disco físico do Vault
        if let Some(disk_path) = self.vault.resolve_file_disk_path(account_id, app_id, clean_filename) {
            if let Ok(meta) = std::fs::metadata(&disk_path) {
                if let Ok(bytes) = std::fs::read(&disk_path) {
                    let ts = meta
                        .modified()
                        .ok()
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or_else(now_unix);
                    return Some(CloudFileRecord {
                        filename: target_clean.clone(),
                        root_token: self.vault.effective_root_token_for_file(account_id, app_id, clean_filename),
                        sha1: sha1_digest(&bytes),
                        timestamp: ts,
                        raw_size: bytes.len() as u64,
                        persist_state: 0,
                        platforms_to_sync: 0xFFFF_FFFF,
                    });
                }
            }
        }

        // Fallback: verifica se o arquivo reside na pasta nativa do jogo (AutoCloud)
        let root_tok = self.vault.effective_root_token_for_file(account_id, app_id, clean_filename);
        if let Some(ac_path) = self.vault.resolve_autocloud_local_path(account_id, app_id, &root_tok, clean_filename) {
            if let Ok(meta) = std::fs::metadata(&ac_path) {
                if let Ok(bytes) = std::fs::read(&ac_path) {
                    let ts = meta
                        .modified()
                        .ok()
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or_else(now_unix);
                    return Some(CloudFileRecord {
                        filename: target_clean,
                        root_token: root_tok,
                        sha1: sha1_digest(&bytes),
                        timestamp: ts,
                        raw_size: bytes.len() as u64,
                        persist_state: 0,
                        platforms_to_sync: 0xFFFF_FFFF,
                    });
                }
            }
        }

        None
    }

    #[cfg(test)]
    pub fn retrieve_download_blob(
        &self,
        account_id: u32,
        app_id: u32,
        clean_filename: &str,
    ) -> Option<Vec<u8>> {
        let acct = if account_id != 0 {
            account_id
        } else {
            self.account_id.max(1)
        };
        if let Some(staged) = self
            .staged_uploads
            .get(&(acct, app_id, clean_filename.to_string()))
        {
            if let Some(ref data) = staged.data {
                return Some(data.clone());
            }
        }
        let expected = self.lookup_expected_file_record(acct, app_id, clean_filename);
        let target_name = expected
            .as_ref()
            .map(|r| r.filename.as_str())
            .unwrap_or(clean_filename);

        if let Some(bytes) = self.vault.read_save_file(acct, app_id, target_name) {
            if let Some(ref exp) = expected {
                if sha1_digest(&bytes) == exp.sha1 {
                    return Some(bytes);
                }
            } else {
                return Some(bytes);
            }
        }
        if let Some(ref exp) = expected {
            if exp.raw_size == 0 && exp.sha1 == sha1_digest(&[]) {
                return Some(Vec::new());
            }
        }
        None
    }

    /// Direct synchronous entry point used by unit tests and `self-test`.
    pub fn handle_rpc(
        &mut self,
        method: &str,
        app_id: u32,
        req_account_id: u32,
        req_bytes: &[u8],
    ) -> Option<(i32, Vec<u8>)> {
        let fields = parse_message(req_bytes);
        let resolved_app_id = if app_id != 0 {
            app_id
        } else {
            extract_app_id(method, &fields)
        };
        if is_system_app_id(resolved_app_id) {
            return None;
        }
        if !self.namespace_apps.is_empty() && !self.namespace_apps.contains(&resolved_app_id) {
            return None;
        }
        let account_id = self.resolve_and_remember_account(req_account_id);

        match method {
            RPC_GET_CHANGELIST => Some(self.finish_get_changelist(
                account_id,
                resolved_app_id,
                &fields,
                Some(CloudManifestFetch::NotConfigured),
            )),
            RPC_LAUNCH_INTENT => Some((ERESULT_OK, build_launch_intent_response())),
            RPC_SUSPEND_SESSION | RPC_EXIT_SYNC => Some((ERESULT_OK, Vec::new())),
            RPC_RESUME_SESSION => Some((ERESULT_OK, Vec::new())),
            RPC_QUOTA_USAGE => Some(self.on_quota_usage(account_id, resolved_app_id)),
            RPC_BEGIN_BATCH => {
                Some(self.finish_begin_batch(account_id, resolved_app_id, &fields, None))
            }
            RPC_BEGIN_UPLOAD => Some(self.on_begin_upload(account_id, resolved_app_id, &fields)),
            RPC_COMMIT_UPLOAD => match self.stage_or_commit_upload_inner(
                account_id,
                resolved_app_id,
                &fields,
            ) {
                CommitInnerAction::Done(resp) => Some(resp),
                CommitInnerAction::StandaloneUpload { assigned_cn, .. } => {
                    let mut m = self.vault.load_and_reconcile_manifest(account_id, resolved_app_id);
                    m.change_number = assigned_cn;
                    let _ = self.vault.save_manifest(account_id, resolved_app_id, &m);
                    self.full_manifest_sent.insert((account_id, resolved_app_id));
                    Some((ERESULT_OK, build_commit_upload_response(true)))
                }
            },
            RPC_COMPLETE_BATCH => {
                let steam_batch_result = find_field(&fields, 3).and_then(|f| f.as_u32());
                if let Some(batch) = self.active_batches.remove(&(account_id, resolved_app_id)) {
                    if steam_batch_result.unwrap_or(1) == 1 {
                        let mut m =
                            self.vault
                                .load_and_reconcile_manifest(account_id, resolved_app_id);
                        m.change_number = batch.assigned_cn;
                        if batch.app_build_id != 0 {
                            m.app_build_id = batch.app_build_id;
                        }
                        let _ = self.vault.save_manifest(account_id, resolved_app_id, &m);
                        self.full_manifest_sent.insert((account_id, resolved_app_id));
                    }
                }
                Some((ERESULT_OK, Vec::new()))
            }
            RPC_FILE_DOWNLOAD => {
                Some(self.on_file_download(account_id, resolved_app_id, &fields, req_bytes))
            }
            RPC_DELETE_FILE => {
                if let Some((_, _, assigned_cn, _)) =
                    self.delete_file_inner(account_id, resolved_app_id, &fields)
                {
                    let mut m = self.vault.load_and_reconcile_manifest(account_id, resolved_app_id);
                    m.change_number = assigned_cn;
                    let _ = self.vault.save_manifest(account_id, resolved_app_id, &m);
                }
                Some((ERESULT_OK, Vec::new()))
            }
            RPC_CONFLICT | RPC_TRANSFER_REPORT | RPC_SYNC_STATS => Some((ERESULT_OK, Vec::new())),
            _ => None,
        }
    }

    fn finish_get_changelist(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
        cloud_fetch: Option<CloudManifestFetch>,
    ) -> (i32, Vec<u8>) {
        let _ = self.vault.sync_local_saves_to_vault(account_id, app_id);

        let synced_cn = find_field(fields, 2).and_then(|f| f.as_u64()).unwrap_or(0);
        let mut local_manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let key = (account_id, app_id);
        let repeat_call = self.full_manifest_sent.contains(&key);
        let machine_name =
            std::env::var("COMPUTERNAME").unwrap_or_else(|_| "CHRONOKEEP-PC".to_string());

        if let Some(fetch) = cloud_fetch {
            match fetch {
                CloudManifestFetch::Inconclusive => {
                    // Proteção anti-rollback: responde como delta parcial (is_only_delta = true)
                    // para impedir que a Steam apague saves remotos ou crie conflitos destrutivos!
                    if let Some(ref lp) = self.log_path {
                        crate::log_msg(
                            lp,
                            &format!(
                                "[Changelist] AVISO: Nuvem inconclusiva para app={}; respondendo is_only_delta=true (CN={})",
                                app_id,
                                local_manifest.change_number.max(synced_cn)
                            ),
                        );
                    }
                    let effective_cn = local_manifest.change_number.max(synced_cn);
                    let list = self.prepare_aligned_file_list(account_id, app_id, &local_manifest);
                    let resp = build_changelist_response(
                        effective_cn,
                        true,
                        &list,
                        &machine_name,
                        local_manifest.app_build_id,
                    );
                    return (ERESULT_OK, resp);
                }
                CloudManifestFetch::Manifest(cm) => {
                    self.synced_apps_from_cloud.insert(key);
                    self.cloud_manifests.insert(key, cm.clone());

                    // Se remotecache.vdf local possui CN superior ao da nuvem (ex.: upload anterior interrompido),
                    // retorna delta vazio em synced_cn para preservar os saves locais mais recentes!
                    if synced_cn > cm.change_number && synced_cn >= local_manifest.change_number {
                        if local_manifest.change_number < synced_cn {
                            local_manifest.change_number = synced_cn;
                            let _ = self.vault.save_manifest(account_id, app_id, &local_manifest);
                        }
                        let resp = build_changelist_response(
                            synced_cn,
                            true,
                            &[],
                            &machine_name,
                            local_manifest.app_build_id,
                        );
                        return (ERESULT_OK, resp);
                    }
                }
                CloudManifestFetch::NoCloudData | CloudManifestFetch::NotConfigured => {
                    self.synced_apps_from_cloud.insert(key);
                }
            }
        }

        // Seleciona o manifesto autoritativo: nuvem (se cloud_cn > local_cn) ou Vault local
        let active_manifest = match self.cloud_manifests.get(&key) {
            Some(cm) if cm.change_number > local_manifest.change_number => cm.clone(),
            _ => {
                if local_manifest.change_number < synced_cn {
                    local_manifest.change_number = synced_cn;
                    let _ = self.vault.save_manifest(account_id, app_id, &local_manifest);
                }
                local_manifest
            }
        };

        if repeat_call && synced_cn >= active_manifest.change_number {
            let resp = build_changelist_response(
                active_manifest.change_number,
                true,
                &[],
                &machine_name,
                active_manifest.app_build_id,
            );
            return (ERESULT_OK, resp);
        }

        self.full_manifest_sent.insert(key);
        let list = self.prepare_aligned_file_list(account_id, app_id, &active_manifest);
        let resp = build_changelist_response(
            active_manifest.change_number,
            false,
            &list,
            &machine_name,
            active_manifest.app_build_id,
        );
        (ERESULT_OK, resp)
    }

    fn prepare_aligned_file_list(
        &self,
        account_id: u32,
        app_id: u32,
        manifest: &AppManifest,
    ) -> Vec<CloudFileRecord> {
        let mut list: Vec<CloudFileRecord> = manifest.files.values().cloned().collect();
        for f in &mut list {
            if let Some(local_path) = self.vault.resolve_autocloud_local_path(
                account_id,
                app_id,
                &f.root_token,
                &f.filename,
            ) {
                if let Ok(meta) = std::fs::metadata(&local_path) {
                    if meta.len() == f.raw_size {
                        if let Ok(mtime) = meta.modified() {
                            if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                                if let Ok(bytes) = std::fs::read(&local_path) {
                                    if sha1_digest(&bytes) == f.sha1 {
                                        f.timestamp = d.as_secs();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        list.sort_by(|a, b| a.filename.cmp(&b.filename));
        list
    }

    fn on_quota_usage(&self, account_id: u32, app_id: u32) -> (i32, Vec<u8>) {
        let local_m = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let manifest = match self.cloud_manifests.get(&(account_id, app_id)) {
            Some(cm) if cm.change_number > local_m.change_number => cm,
            _ => &local_m,
        };
        let count = manifest.files.len() as u32;
        let total_bytes: u64 = manifest.files.values().map(|f| f.raw_size).sum();
        let resp = build_quota_response(count, total_bytes, 10_000, 1024 * 1024 * 1024);
        (ERESULT_OK, resp)
    }

    fn finish_begin_batch(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
        cloud_fetch: Option<CloudManifestFetch>,
    ) -> (i32, Vec<u8>) {
        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let app_build_id = find_field(fields, 6).and_then(|f| f.as_u64()).unwrap_or(0);

        let mut cloud_cn = self
            .cloud_manifests
            .get(&(account_id, app_id))
            .map(|m| m.change_number)
            .unwrap_or(0);

        if let Some(CloudManifestFetch::Manifest(cm)) = cloud_fetch {
            cloud_cn = cloud_cn.max(cm.change_number);
            self.cloud_manifests.insert((account_id, app_id), cm);
        }

        for f in fields {
            if (f.number == 3 || f.number == 4) && f.as_str().is_some() {
                let raw_name = f.as_str().unwrap_or("");
                let (tok, _) = split_root_token(raw_name);
                if !tok.is_empty() && manifest.default_root_token.is_empty() {
                    manifest.default_root_token = tok;
                    let _ = self.vault.save_manifest(account_id, app_id, &manifest);
                }
            }
        }

        let batch_id = self.next_batch_id;
        self.next_batch_id = self.next_batch_id.wrapping_add(1).max(1);
        let base_cn = manifest.change_number.max(cloud_cn);
        let assigned_cn = base_cn.saturating_add(1);

        crate::gdrive::invalidate_serve_cache(account_id, app_id);
        self.active_batches.insert(
            (account_id, app_id),
            ActiveBatch {
                batch_id,
                base_cn,
                assigned_cn,
                app_build_id,
                committed_files: HashMap::new(),
                deleted_files: HashSet::new(),
            },
        );

        let resp = build_begin_batch_response(batch_id, assigned_cn);
        (ERESULT_OK, resp)
    }

    fn on_begin_upload(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        let file_size = find_field(fields, 2).and_then(|f| f.as_u64()).unwrap_or(0);
        let raw_file_size = find_field(fields, 3).and_then(|f| f.as_u64()).unwrap_or(0);
        let timestamp = find_field(fields, 5)
            .and_then(|f| f.as_u64())
            .unwrap_or_else(now_unix);
        let raw_filename = find_field(fields, 6).and_then(|f| f.as_str()).unwrap_or("");
        let platforms = find_field(fields, 7)
            .and_then(|f| f.as_u32())
            .unwrap_or(0xFFFF_FFFF);

        let (root_token, clean_name) = split_root_token(raw_filename);
        if clean_name.is_empty() {
            let url_host = format!("127.0.0.1:{}", self.listen_port);
            let url_path = format!("/upload/{}/{}/empty", account_id, app_id);
            return (
                ERESULT_OK,
                build_begin_upload_response(&url_host, &url_path, 0),
            );
        }

        self.staged_uploads.insert(
            (account_id, app_id, clean_name.clone()),
            StagedUpload {
                root_token,
                timestamp,
                platforms_to_sync: platforms,
                data: None,
            },
        );

        let url_host = format!("127.0.0.1:{}", self.listen_port);
        let url_path = format!(
            "/upload/{}/{}/{}",
            account_id,
            app_id,
            url_encode_path(&clean_name)
        );
        let block_len = if file_size > 0 {
            file_size
        } else {
            raw_file_size
        };

        let resp = build_begin_upload_response(&url_host, &url_path, block_len);
        (ERESULT_OK, resp)
    }

    fn stage_or_commit_upload_inner(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> CommitInnerAction {
        let transfer_succeeded = find_field(fields, 1)
            .and_then(|f| f.as_bool())
            .unwrap_or(false);
        let raw_filename = find_field(fields, 4).and_then(|f| f.as_str()).unwrap_or("");
        let (tok_from_commit, clean_name) = split_root_token(raw_filename);

        if !transfer_succeeded || clean_name.is_empty() {
            self.staged_uploads
                .remove(&(account_id, app_id, clean_name));
            return CommitInnerAction::Done((ERESULT_OK, build_commit_upload_response(false)));
        }

        let Some(staged) = self
            .staged_uploads
            .remove(&(account_id, app_id, clean_name.clone()))
        else {
            return CommitInnerAction::Done((ERESULT_OK, build_commit_upload_response(false)));
        };

        // NUNCA usar unwrap_or_default(): se o PUT /upload falhou ou truncou, rejeita o commit!
        let Some(bytes) = staged.data else {
            if let Some(ref lp) = self.log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[CommitUpload] REJEITADO: Payload PUT ausente para app={} file='{}'",
                        app_id, clean_name
                    ),
                );
            }
            return CommitInnerAction::Done((ERESULT_OK, build_commit_upload_response(false)));
        };

        let Ok(sha1) = self
            .vault
            .write_save_file(account_id, app_id, &clean_name, &bytes)
        else {
            return CommitInnerAction::Done((ERESULT_OK, build_commit_upload_response(false)));
        };

        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let effective_token = if !staged.root_token.is_empty() {
            staged.root_token
        } else if !tok_from_commit.is_empty() {
            tok_from_commit
        } else if let Some(existing) = manifest.files.get(&clean_name) {
            if !existing.root_token.is_empty() {
                existing.root_token.clone()
            } else {
                manifest.default_root_token.clone()
            }
        } else {
            manifest.default_root_token.clone()
        };

        if !effective_token.is_empty() && manifest.default_root_token.is_empty() {
            manifest.default_root_token = effective_token.clone();
        }

        manifest.files.insert(
            clean_name.clone(),
            CloudFileRecord {
                filename: clean_name.clone(),
                root_token: effective_token,
                sha1,
                timestamp: if staged.timestamp > 0 {
                    staged.timestamp
                } else {
                    now_unix()
                },
                raw_size: bytes.len() as u64,
                persist_state: 0,
                platforms_to_sync: staged.platforms_to_sync,
            },
        );

        if let Some(batch) = self.active_batches.get_mut(&(account_id, app_id)) {
            // Mantém change_number local em base_cn até que CompleteAppUploadBatchBlocking confirme o lote inteiro!
            if batch.app_build_id != 0 {
                manifest.app_build_id = batch.app_build_id;
            }
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);
            batch.deleted_files.remove(&clean_name);
            batch.committed_files.insert(clean_name, bytes);
            return CommitInnerAction::Done((ERESULT_OK, build_commit_upload_response(true)));
        }

        let base_cn = manifest.change_number;
        let assigned_cn = base_cn.saturating_add(1);
        let app_build_id = manifest.app_build_id;
        let _ = self.vault.save_manifest(account_id, app_id, &manifest);

        CommitInnerAction::StandaloneUpload {
            clean_name,
            bytes,
            base_cn,
            assigned_cn,
            app_build_id,
        }
    }

    fn on_file_download(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
        req_bytes: &[u8],
    ) -> (i32, Vec<u8>) {
        let raw_filename = cr_proto::cloud_messages::extract_download_filename(fields, req_bytes);
        let (_, clean_name) = split_root_token(&raw_filename);
        let effective_name = if !clean_name.is_empty() {
            clean_name
        } else if !raw_filename.is_empty() {
            raw_filename.clone()
        } else {
            String::new()
        };

        let expected = if !effective_name.is_empty() {
            self.lookup_expected_file_record(account_id, app_id, &effective_name)
        } else {
            None
        };

        let (resolved_name, file_size, sha1, timestamp) = if let Some(entry) = expected {
            (
                entry.filename,
                entry.raw_size,
                entry.sha1,
                entry.timestamp,
            )
        } else if let Some(bytes) = self.vault.read_save_file(account_id, app_id, &effective_name) {
            (
                effective_name.clone(),
                bytes.len() as u64,
                sha1_digest(&bytes),
                now_unix(),
            )
        } else {
            // Em caso de corrida com download da nuvem ou inicialização rápida da Steam:
            // NÃO retornar ERESULT_FAIL (código 2), pois isso causa "result Timeout / Erro de sincronização" na Steam!
            // Retorna ERESULT_OK apontando para o endpoint HTTP local. O endpoint /download/ possui
            // toda a inteligência de espera, pre-warm e download sob demanda do Google Drive.
            if let Some(ref lp) = self.log_path {
                crate::lifecycle::log_debug(
                    lp,
                    &format!(
                        "[ClientFileDownload] Arquivo '{}' nao localizado no manifest (app={}); respondendo rota HTTP para pre-warm",
                        effective_name, app_id
                    ),
                );
            }
            (
                effective_name.clone(),
                0,
                sha1_digest(&[]),
                now_unix(),
            )
        };

        let url_host = format!("127.0.0.1:{}", self.listen_port);
        let url_path = format!(
            "/download/{}/{}/{}",
            account_id,
            app_id,
            url_encode_path(&resolved_name)
        );
        let resp =
            build_file_download_response(app_id, file_size, &sha1, timestamp, &url_host, &url_path);
        (ERESULT_OK, resp)
    }

    fn delete_file_inner(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> Option<(String, u64, u64, u64)> {
        let raw_filename = find_field(fields, 2).and_then(|f| f.as_str()).unwrap_or("");
        let (_, clean_name) = split_root_token(raw_filename);
        if clean_name.is_empty() {
            return None;
        }

        let _ = self.vault.delete_save_file(account_id, app_id, &clean_name);
        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let removed_local = manifest.files.remove(&clean_name).is_some();

        if let Some(batch) = self.active_batches.get_mut(&(account_id, app_id)) {
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);
            batch.committed_files.remove(&clean_name);
            batch.deleted_files.insert(clean_name);
            return None;
        }

        if removed_local {
            let base_cn = manifest.change_number;
            let assigned_cn = base_cn.saturating_add(1);
            let app_build_id = manifest.app_build_id;
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);
            return Some((clean_name, base_cn, assigned_cn, app_build_id));
        }

        None
    }
}

fn find_record_ci<'a>(
    map: &'a HashMap<String, CloudFileRecord>,
    target_name: &str,
) -> Option<&'a CloudFileRecord> {
    let (_, clean_target) = split_root_token(target_name);
    if let Some(r) = map.get(target_name) {
        return Some(r);
    }
    if !clean_target.is_empty() {
        if let Some(r) = map.get(&clean_target) {
            return Some(r);
        }
    }
    map.iter()
        .find(|(k, _)| {
            if k.eq_ignore_ascii_case(target_name) {
                return true;
            }
            let (_, clean_k) = split_root_token(k);
            clean_k.eq_ignore_ascii_case(&clean_target)
                || clean_k.eq_ignore_ascii_case(target_name)
                || k.eq_ignore_ascii_case(&clean_target)
        })
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cr_proto::wire::ProtoWriter;

    fn temp_vault(tag: &str) -> (PathBuf, VaultStorage) {
        let dir = std::env::temp_dir().join(format!(
            "ck_rpc_test_{}_{}_{}",
            tag,
            std::process::id(),
            now_unix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vault = VaultStorage::new(dir.join("vault"), Some(dir.join("steam")));
        (dir, vault)
    }

    #[test]
    fn test_commit_upload_rejects_missing_put_body() {
        let (base, vault) = temp_vault("missing_put");
        let mut engine = RpcEngine::new(vault.clone(), 39771, None);
        let acct = 12345u32;
        let app = 3010850u32;

        let mut begin = ProtoWriter::new();
        begin.write_varint(1, app as u64);
        begin.write_varint(2, 1024);
        begin.write_varint(3, 1024);
        begin.write_str(6, "SaveSlot0.sav");
        let _ = engine.handle_rpc(RPC_BEGIN_UPLOAD, app, acct, &begin.into_bytes());

        // Simula CommitFileUpload sem que o PUT /upload tenha armazenado bytes
        let mut commit = ProtoWriter::new();
        commit.write_varint(1, 1); // transfer_succeeded = true
        commit.write_varint(2, app as u64);
        commit.write_str(4, "SaveSlot0.sav");
        let (eresult, resp_bytes) = engine
            .handle_rpc(RPC_COMMIT_UPLOAD, app, acct, &commit.into_bytes())
            .unwrap();
        assert_eq!(eresult, ERESULT_OK);

        let resp_fields = parse_message(&resp_bytes);
        let committed = find_field(&resp_fields, 1)
            .and_then(|f| f.as_bool())
            .unwrap_or(true);
        assert!(!committed, "commit sem PUT não pode confirmar nem gravar save de 0 bytes");
        assert!(vault.read_save_file(acct, app, "SaveSlot0.sav").is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_batch_aborted_by_steam_does_not_advance_cn() {
        let (base, vault) = temp_vault("batch_abort");
        let mut engine = RpcEngine::new(vault.clone(), 39771, None);
        let acct = 12345u32;
        let app = 3010850u32;

        let mut bb = ProtoWriter::new();
        bb.write_varint(1, app as u64);
        let (_, bb_resp) = engine
            .handle_rpc(RPC_BEGIN_BATCH, app, acct, &bb.into_bytes())
            .unwrap();
        let batch_id = find_field(&parse_message(&bb_resp), 1)
            .and_then(|f| f.as_u64())
            .unwrap();

        // Steam aborta o batch com batch_eresult = 2 (k_EResultFail)
        let mut cb = ProtoWriter::new();
        cb.write_varint(1, app as u64);
        cb.write_varint(2, batch_id);
        cb.write_varint(3, 2);
        let _ = engine.handle_rpc(RPC_COMPLETE_BATCH, app, acct, &cb.into_bytes());

        let m = vault.load_manifest_raw(acct, app);
        assert_eq!(m.change_number, 0);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_retrieve_download_missing_returns_none_not_zero_bytes() {
        let (base, vault) = temp_vault("dl_404");
        let engine = RpcEngine::new(vault, 39771, None);
        assert!(engine
            .retrieve_download_blob(999, 3010850, "nonexistent.sav")
            .is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_inconclusive_cloud_changelist_sets_is_only_delta_true() {
        let (base, vault) = temp_vault("inconclusive_delta");
        let mut engine = RpcEngine::new(vault, 39771, None);
        let mut req = ProtoWriter::new();
        req.write_varint(1, 3010850);
        req.write_varint(2, 7); // synced_cn = 7
        let req_bytes = req.into_bytes();
        let req_fields = parse_message(&req_bytes);

        let (eresult, resp) = engine.finish_get_changelist(
            12345,
            3010850,
            &req_fields,
            Some(CloudManifestFetch::Inconclusive),
        );
        assert_eq!(eresult, ERESULT_OK);
        let resp_fields = parse_message(&resp);
        let cn = find_field(&resp_fields, 1).and_then(|f| f.as_u64()).unwrap();
        let is_only_delta = find_field(&resp_fields, 3).and_then(|f| f.as_bool()).unwrap();
        assert_eq!(cn, 7);
        assert!(is_only_delta);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_on_file_download_unknown_file_returns_ok_with_prewarm_route() {
        let (base, vault) = temp_vault("dl_prewarm");
        let mut engine = RpcEngine::new(vault, 39771, None);

        let mut pw = ProtoWriter::new();
        pw.write_varint(1, 2868840); // app_id
        pw.write_str(2, "profile1/saves/history/1793546464.run"); // filename
        let req_bytes = pw.into_bytes();
        let fields = parse_message(&req_bytes);

        let (eresult, resp) = engine.on_file_download(12345, 2868840, &fields, &req_bytes);
        assert_eq!(eresult, ERESULT_OK);
        assert!(!resp.is_empty());

        let resp_fields = parse_message(&resp);
        let host = find_field(&resp_fields, 7).and_then(|f| f.as_str()).unwrap();
        let path = find_field(&resp_fields, 8).and_then(|f| f.as_str()).unwrap();
        assert_eq!(host, "127.0.0.1:39771");
        assert!(path.contains("profile1/saves/history/1793546464.run"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_find_record_ci_with_and_without_root_tokens() {
        let mut records = HashMap::new();
        records.insert(
            "saves/prefs.save".to_string(),
            CloudFileRecord {
                filename: "saves/prefs.save".to_string(),
                root_token: "%WinMyDocuments%".to_string(),
                sha1: [1u8; 20],
                timestamp: 1000,
                raw_size: 100,
                persist_state: 0,
                platforms_to_sync: 0xFFFF_FFFF,
            },
        );

        // Matching direct clean name
        assert!(find_record_ci(&records, "saves/prefs.save").is_some());
        // Matching case-insensitive
        assert!(find_record_ci(&records, "Saves/Prefs.Save").is_some());
        // Matching with root token prepended in query
        assert!(find_record_ci(&records, "%WinMyDocuments%/saves/prefs.save").is_some());
        assert!(find_record_ci(&records, "%winmydocuments%saves/prefs.save").is_some());
    }
}
