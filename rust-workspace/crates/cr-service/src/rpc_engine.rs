//! Steam Cloud RPC Execution Engine (`cr-service`).
//! Handles all `Cloud.*` RPC requests forwarded by `cloud_redirect.dll` (`cr-bridge-dll`)
//! and coordinates staging blobs uploaded/downloaded via HTTP by `steamclient64.dll`.

use crate::vault::{now_unix, VaultStorage};
use cr_proto::cloud_messages::*;
use cr_proto::sha1::sha1_digest;
use cr_proto::wire::{find_field, parse_message, ProtoField};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct StagedUpload {
    pub root_token: String,
    pub timestamp: u64,
    pub platforms_to_sync: u32,
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ActiveBatch {
    pub assigned_cn: u64,
    pub app_build_id: u64,
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
    full_manifest_sent: HashSet<(u32, u32)>,
    synced_apps_from_cloud: HashSet<(u32, u32)>,
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
            full_manifest_sent: HashSet::new(),
            synced_apps_from_cloud: HashSet::new(),
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
            self.namespace_apps = apps.into_iter().filter(|&id| id != 0).collect();
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

    pub fn retrieve_download_blob(
        &self,
        account_id: u32,
        app_id: u32,
        clean_filename: &str,
    ) -> Option<Vec<u8>> {
        let acct = if account_id != 0 {
            account_id
        } else {
            self.account_id
        };
        if let Some(staged) = self
            .staged_uploads
            .get(&(acct, app_id, clean_filename.to_string()))
        {
            if let Some(ref data) = staged.data {
                return Some(data.clone());
            }
        }
        if let Some(bytes) = self.vault.read_save_file(acct, app_id, clean_filename) {
            return Some(bytes);
        }
        let manifest = self.vault.load_and_reconcile_manifest(acct, app_id);
        for k in manifest.files.keys() {
            if k.eq_ignore_ascii_case(clean_filename) {
                if let Some(bytes) = self.vault.read_save_file(acct, app_id, k) {
                    return Some(bytes);
                }
            }
        }
        Some(Vec::new())
    }

    pub fn handle_rpc(
        &mut self,
        method: &str,
        app_id: u32,
        req_account_id: u32,
        req_bytes: &[u8],
    ) -> Option<(i32, Vec<u8>)> {
        if req_account_id != 0 && self.account_id == 0 {
            self.account_id = req_account_id;
        }
        let account_id = if req_account_id != 0 {
            req_account_id
        } else if self.account_id != 0 {
            self.account_id
        } else {
            1 // Safe fallback if Steam hasn't published accountId yet
        };

        let fields = parse_message(req_bytes);
        let resolved_app_id = if app_id != 0 {
            app_id
        } else {
            extract_app_id(method, &fields)
        };

        match method {
            RPC_GET_CHANGELIST => Some(self.on_get_changelist(account_id, resolved_app_id, &fields)),
            RPC_LAUNCH_INTENT => Some((ERESULT_OK, build_launch_intent_response())),
            RPC_SUSPEND_SESSION => {
                self.on_exit_sync(account_id, resolved_app_id);
                Some((ERESULT_OK, Vec::new()))
            }
            RPC_RESUME_SESSION => Some((ERESULT_OK, Vec::new())),
            RPC_QUOTA_USAGE => Some(self.on_quota_usage(account_id, resolved_app_id)),
            RPC_BEGIN_BATCH => Some(self.on_begin_batch(account_id, resolved_app_id, &fields)),
            RPC_BEGIN_UPLOAD => Some(self.on_begin_upload(account_id, resolved_app_id, &fields)),
            RPC_COMMIT_UPLOAD => Some(self.on_commit_upload(account_id, resolved_app_id, &fields)),
            RPC_COMPLETE_BATCH => Some(self.on_complete_batch(account_id, resolved_app_id, &fields)),
            RPC_FILE_DOWNLOAD => Some(self.on_file_download(account_id, resolved_app_id, &fields)),
            RPC_DELETE_FILE => Some(self.on_delete_file(account_id, resolved_app_id, &fields)),
            RPC_EXIT_SYNC => {
                self.on_exit_sync(account_id, resolved_app_id);
                Some((ERESULT_OK, Vec::new()))
            }
            RPC_CONFLICT | RPC_TRANSFER_REPORT | RPC_SYNC_STATS => {
                Some((ERESULT_OK, Vec::new()))
            }
            _ => None,
        }
    }

    fn on_get_changelist(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        // Se este app ainda não foi sincronizado do Google Drive nesta sessão, dispara em background para nunca travar o RPC
        if self.synced_apps_from_cloud.insert((account_id, app_id)) {
            let sp_opt = self.vault.steam_path.clone().or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            });
            if let Some(sp) = sp_opt {
                let vault_clone = self.vault.clone();
                let lp_clone = self.log_path.clone();
                std::thread::spawn(move || {
                    let _ = crate::gdrive::pull_app_from_gdrive(
                        &sp,
                        account_id,
                        app_id,
                        &vault_clone,
                        lp_clone.as_ref(),
                    );
                });
            }
        }

        // Reconcilia arquivos salvos localmente em userdata para o Vault
        let _ = self.vault.sync_local_saves_to_vault(account_id, app_id);

        let synced_cn = find_field(fields, 2).and_then(|f| f.as_u64()).unwrap_or(0);
        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        if manifest.change_number < synced_cn {
            manifest.change_number = synced_cn;
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);
        }

        let key = (account_id, app_id);
        self.full_manifest_sent.insert(key);

        let mut list: Vec<CloudFileRecord> = manifest.files.values().cloned().collect();
        list.sort_by(|a, b| a.filename.cmp(&b.filename));

        let machine_name = std::env::var("COMPUTERNAME")
            .unwrap_or_else(|_| "CHRONOKEEP-PC".to_string());

        let resp = build_changelist_response(
            manifest.change_number,
            false,
            &list,
            &machine_name,
            manifest.app_build_id,
        );
        (ERESULT_OK, resp)
    }

    fn on_quota_usage(&self, account_id: u32, app_id: u32) -> (i32, Vec<u8>) {
        let manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let count = manifest.files.len() as u32;
        let total_bytes: u64 = manifest.files.values().map(|f| f.raw_size).sum();
        let resp = build_quota_response(count, total_bytes, 10_000, 1024 * 1024 * 1024);
        (ERESULT_OK, resp)
    }

    fn on_begin_batch(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let app_build_id = find_field(fields, 6).and_then(|f| f.as_u64()).unwrap_or(0);

        // Capture any root tokens present in upload/delete file lists (fields 3 & 4)
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
        let assigned_cn = manifest.change_number.saturating_add(1);

        self.active_batches.insert(
            (account_id, app_id),
            ActiveBatch {
                assigned_cn,
                app_build_id,
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

    fn on_commit_upload(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        let transfer_succeeded = find_field(fields, 1)
            .and_then(|f| f.as_bool())
            .unwrap_or(false);
        let raw_filename = find_field(fields, 4).and_then(|f| f.as_str()).unwrap_or("");
        let (tok_from_commit, clean_name) = split_root_token(raw_filename);

        if !transfer_succeeded || clean_name.is_empty() {
            self.staged_uploads
                .remove(&(account_id, app_id, clean_name));
            return (ERESULT_OK, build_commit_upload_response(false));
        }

        let Some(staged) = self
            .staged_uploads
            .remove(&(account_id, app_id, clean_name.clone()))
        else {
            return (ERESULT_OK, build_commit_upload_response(false));
        };

        let bytes = staged.data.unwrap_or_default();
        let Ok(sha1) = self
            .vault
            .write_save_file(account_id, app_id, &clean_name, &bytes)
        else {
            return (ERESULT_OK, build_commit_upload_response(false));
        };

        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let effective_token = if !staged.root_token.is_empty() {
            staged.root_token
        } else if !tok_from_commit.is_empty() {
            tok_from_commit
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

        if let Some(batch) = self.active_batches.get(&(account_id, app_id)) {
            manifest.change_number = batch.assigned_cn;
            if batch.app_build_id != 0 {
                manifest.app_build_id = batch.app_build_id;
            }
        } else {
            manifest.change_number = manifest.change_number.saturating_add(1);
        }

        let _ = self.vault.save_manifest(account_id, app_id, &manifest);
        self.full_manifest_sent.remove(&(account_id, app_id));

        let sp_opt = self.vault.steam_path.clone().or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        });

        if let Some(sp) = sp_opt {
            let clean_name_clone = clean_name.clone();
            let bytes_clone = bytes.clone();
            let m_bytes = std::fs::read(self.vault.manifest_path(account_id, app_id)).ok();
            let lp_clone = self.log_path.clone();
            std::thread::spawn(move || {
                let _ = crate::gdrive::sync_file_to_gdrive(
                    &sp,
                    account_id,
                    app_id,
                    &clean_name_clone,
                    &bytes_clone,
                    lp_clone.as_ref(),
                );
                if let Some(mb) = m_bytes {
                    let _ = crate::gdrive::sync_file_to_gdrive(
                        &sp,
                        account_id,
                        app_id,
                        "manifest.tsv",
                        &mb,
                        lp_clone.as_ref(),
                    );
                }
            });
        }

        (ERESULT_OK, build_commit_upload_response(true))
    }

    /// Called when a game session exits, is suspended, or terminates. Reconciles any saves
    /// written to local Steam `userdata` directly by the game into the Vault, and syncs to Google Drive.
    pub fn on_exit_sync(&mut self, account_id: u32, app_id: u32) {
        if account_id == 0 || app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
            return;
        }

        if let Some(ref lp) = self.log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[ExitSync] Jogo finalizado ou sessao suspensa: app={} acct={}. Verificando saves locais...",
                    app_id, account_id
                ),
            );
        }

        let modified = self.vault.sync_local_saves_to_vault(account_id, app_id);
        if !modified.is_empty() {
            let manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);

            if let Some(ref lp) = self.log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[ExitSync] Detectados {} arquivos modificados no disco local para app={}. Disparando upload para o Google Drive!",
                        modified.len(),
                        app_id
                    ),
                );
            }

            let sp_opt = self.vault.steam_path.clone().or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            });

            if let Some(sp) = sp_opt {
                let m_bytes = std::fs::read(self.vault.manifest_path(account_id, app_id)).ok();
                let lp_clone = self.log_path.clone();
                std::thread::spawn(move || {
                    for (rel_name, bytes) in modified {
                        let _ = crate::gdrive::sync_file_to_gdrive(
                            &sp,
                            account_id,
                            app_id,
                            &rel_name,
                            &bytes,
                            lp_clone.as_ref(),
                        );
                    }
                    if let Some(mb) = m_bytes {
                        let _ = crate::gdrive::sync_file_to_gdrive(
                            &sp,
                            account_id,
                            app_id,
                            "manifest.tsv",
                            &mb,
                            lp_clone.as_ref(),
                        );
                    }
                });
            }
        } else {
            if let Some(ref lp) = self.log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[ExitSync] Nenhum arquivo modificado detectado no disco para app={}",
                        app_id
                    ),
                );
            }
        }
    }

    fn on_complete_batch(
        &mut self,
        account_id: u32,
        app_id: u32,
        _fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        if let Some(batch) = self.active_batches.remove(&(account_id, app_id)) {
            let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
            manifest.change_number = batch.assigned_cn;
            if batch.app_build_id != 0 {
                manifest.app_build_id = batch.app_build_id;
            }
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);
            self.full_manifest_sent.remove(&(account_id, app_id));
        }
        (ERESULT_OK, Vec::new())
    }

    fn on_file_download(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        let raw_filename = find_field(fields, 2).and_then(|f| f.as_str()).unwrap_or("");
        let (_, clean_name) = split_root_token(raw_filename);
        if clean_name.is_empty() {
            return (ERESULT_OK, Vec::new());
        }

        let manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        let ci_entry = manifest
            .files
            .get(&clean_name)
            .or_else(|| {
                manifest
                    .files
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&clean_name))
                    .map(|(_, v)| v)
            });

        let (resolved_name, file_size, sha1, timestamp) = if let Some(entry) = ci_entry {
            (
                entry.filename.clone(),
                entry.raw_size,
                entry.sha1,
                entry.timestamp,
            )
        } else if let Some(bytes) = self.vault.read_save_file(account_id, app_id, &clean_name) {
            (
                clean_name.clone(),
                bytes.len() as u64,
                sha1_digest(&bytes),
                now_unix(),
            )
        } else {
            (clean_name.clone(), 0u64, sha1_digest(&[]), now_unix())
        };

        let url_host = format!("127.0.0.1:{}", self.listen_port);
        let url_path = format!(
            "/download/{}/{}/{}",
            account_id,
            app_id,
            url_encode_path(&resolved_name)
        );
        let resp = build_file_download_response(
            app_id, file_size, &sha1, timestamp, &url_host, &url_path,
        );
        (ERESULT_OK, resp)
    }

    fn on_delete_file(
        &mut self,
        account_id: u32,
        app_id: u32,
        fields: &[ProtoField<'_>],
    ) -> (i32, Vec<u8>) {
        let raw_filename = find_field(fields, 2).and_then(|f| f.as_str()).unwrap_or("");
        let (_, clean_name) = split_root_token(raw_filename);
        if clean_name.is_empty() {
            return (ERESULT_OK, Vec::new());
        }

        let _ = self.vault.delete_save_file(account_id, app_id, &clean_name);
        let mut manifest = self.vault.load_and_reconcile_manifest(account_id, app_id);
        if manifest.files.remove(&clean_name).is_some() {
            if let Some(batch) = self.active_batches.get(&(account_id, app_id)) {
                manifest.change_number = batch.assigned_cn;
            } else {
                manifest.change_number = manifest.change_number.saturating_add(1);
            }
            let _ = self.vault.save_manifest(account_id, app_id, &manifest);
            self.full_manifest_sent.remove(&(account_id, app_id));
        }

        (ERESULT_OK, Vec::new())
    }
}
