//! Native Google Drive Synchronization Engine for `chrono-daemon.exe`.
//! Implements bounded cloud manifest probes, 3s serve cache, circuit breaker,
//! on-demand SHA-1 verified downloads, all-or-nothing parallel batch uploads,
//! and graceful shutdown upload draining.

use crate::vault::{AppManifest, VaultStorage};
use cr_common::gdrive::{
    clear_folder_cache, delete_drive_file_in_folder, download_drive_file, find_drive_file_meta,
    find_drive_folder_checked, find_or_create_drive_folder, get_valid_access_token,
    list_drive_files_checked, upload_file_multipart, DriveLookup,
};
use cr_proto::sha1::{sha1_digest, to_hex};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

static ACTIVE_PULLS: Mutex<Option<HashSet<(u32, u32)>>> = Mutex::new(None);
static PULL_ALL_ACTIVE: AtomicBool = AtomicBool::new(false);
static APP_UPLOAD_LOCKS: Mutex<Option<HashMap<(u32, u32), Arc<Mutex<()>>>>> = Mutex::new(None);
static LAST_UPLOADED_MANIFEST: Mutex<Option<HashMap<(u32, u32), [u8; 20]>>> = Mutex::new(None);
static LAST_PULLED_AT: Mutex<Option<HashMap<(u32, u32), Instant>>> = Mutex::new(None);

const PULL_COOLDOWN: Duration = Duration::from_secs(300);
const SERVE_CACHE_TTL: Duration = Duration::from_secs(3);
const CIRCUIT_BREAKER_THRESHOLD: u32 = 2;
const CIRCUIT_BREAKER_COOLDOWN: Duration = Duration::from_secs(8);
const MAX_BATCH_WORKERS: usize = 4;
const MAX_INFLIGHT_UPLOAD_BYTES: usize = 24 * 1024 * 1024;

#[derive(Default)]
struct UploadTracker {
    total_active: usize,
    per_app_active: HashMap<(u32, u32), usize>,
}

static UPLOAD_TRACKER: (Mutex<Option<UploadTracker>>, Condvar) = (Mutex::new(None), Condvar::new());

struct ActiveUploadGuard {
    key: (u32, u32),
}

impl ActiveUploadGuard {
    fn acquire(account_id: u32, app_id: u32) -> Self {
        let key = (account_id, app_id);
        let mut guard = UPLOAD_TRACKER.0.lock().unwrap_or_else(|e| e.into_inner());
        let tracker = guard.get_or_insert_with(UploadTracker::default);
        tracker.total_active += 1;
        *tracker.per_app_active.entry(key).or_insert(0) += 1;
        Self { key }
    }
}

impl Drop for ActiveUploadGuard {
    fn drop(&mut self) {
        let mut guard = UPLOAD_TRACKER.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref mut tracker) = *guard {
            tracker.total_active = tracker.total_active.saturating_sub(1);
            if let Some(cnt) = tracker.per_app_active.get_mut(&self.key) {
                *cnt = cnt.saturating_sub(1);
                if *cnt == 0 {
                    tracker.per_app_active.remove(&self.key);
                }
            }
        }
        UPLOAD_TRACKER.1.notify_all();
    }
}

pub fn has_active_upload_for_app(account_id: u32, app_id: u32) -> bool {
    let guard = UPLOAD_TRACKER.0.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .and_then(|t| t.per_app_active.get(&(account_id, app_id)).copied())
        .unwrap_or(0)
        > 0
}

/// Waits up to `timeout` for any in-flight Google Drive upload batch on `(account_id, app_id)` to finish.
pub fn wait_for_app_upload(account_id: u32, app_id: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut guard = UPLOAD_TRACKER.0.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        let active = guard
            .as_ref()
            .and_then(|t| t.per_app_active.get(&(account_id, app_id)).copied())
            .unwrap_or(0);
        if active == 0 {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let (next_guard, _) = UPLOAD_TRACKER
            .1
            .wait_timeout(guard, deadline - now)
            .unwrap_or_else(|e| e.into_inner());
        guard = next_guard;
    }
}

/// Drains all in-flight Google Drive uploads before daemon shutdown (up to `timeout`).
pub fn wait_for_all_uploads(timeout: Duration, log_path: Option<&PathBuf>) {
    let deadline = Instant::now() + timeout;
    let mut guard = UPLOAD_TRACKER.0.lock().unwrap_or_else(|e| e.into_inner());
    let initial = guard.as_ref().map(|t| t.total_active).unwrap_or(0);
    if initial == 0 {
        return;
    }

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Lifecycle] Aguardando conclusao de {} upload(s) ativo(s) para o Google Drive (limite {}s)...",
                initial,
                timeout.as_secs()
            ),
        );
    }

    loop {
        let active = guard.as_ref().map(|t| t.total_active).unwrap_or(0);
        if active == 0 {
            if let Some(lp) = log_path {
                crate::log_msg(
                    lp,
                    "[Lifecycle] Todos os uploads pendentes foram concluidos com seguranca.",
                );
            }
            return;
        }
        let now = Instant::now();
        if now >= deadline {
            if let Some(lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Lifecycle] AVISO: Tempo limite de drenagem esgotado com {} upload(s) ainda ativo(s).",
                        active
                    ),
                );
            }
            return;
        }
        let (next_guard, _) = UPLOAD_TRACKER
            .1
            .wait_timeout(guard, deadline - now)
            .unwrap_or_else(|e| e.into_inner());
        guard = next_guard;
    }
}

#[derive(Debug, Clone)]
pub enum CloudManifestFetch {
    Manifest(AppManifest),
    NoCloudData,
    NotConfigured,
    Inconclusive,
}

static SERVE_CACHE: Mutex<Option<HashMap<(u32, u32), (CloudManifestFetch, Instant)>>> =
    Mutex::new(None);
static CIRCUIT_BREAKER: Mutex<(u32, Option<Instant>)> = Mutex::new((0, None));

pub fn invalidate_serve_cache(account_id: u32, app_id: u32) {
    if let Ok(mut guard) = SERVE_CACHE.lock() {
        if let Some(ref mut map) = *guard {
            map.remove(&(account_id, app_id));
        }
    }
}

fn put_serve_cache(account_id: u32, app_id: u32, val: CloudManifestFetch) {
    if matches!(
        val,
        CloudManifestFetch::Manifest(_) | CloudManifestFetch::NoCloudData
    ) {
        if let Ok(mut guard) = SERVE_CACHE.lock() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert((account_id, app_id), (val, Instant::now()));
        }
    }
}

fn get_serve_cache(account_id: u32, app_id: u32) -> Option<CloudManifestFetch> {
    if let Ok(guard) = SERVE_CACHE.lock() {
        if let Some(ref map) = *guard {
            if let Some((ref val, ref ts)) = map.get(&(account_id, app_id)) {
                if ts.elapsed() < SERVE_CACHE_TTL {
                    return Some(val.clone());
                }
            }
        }
    }
    None
}

fn circuit_breaker_is_open() -> bool {
    if let Ok(guard) = CIRCUIT_BREAKER.lock() {
        if let Some(until) = guard.1 {
            return Instant::now() < until;
        }
    }
    false
}

fn record_cloud_probe_result(ok: bool) {
    if let Ok(mut guard) = CIRCUIT_BREAKER.lock() {
        if ok {
            guard.0 = 0;
            guard.1 = None;
        } else {
            guard.0 = guard.0.saturating_add(1);
            if guard.0 >= CIRCUIT_BREAKER_THRESHOLD {
                guard.1 = Some(Instant::now() + CIRCUIT_BREAKER_COOLDOWN);
            }
        }
    }
}

pub fn resolve_token_path(exe_dir: &Path) -> Option<PathBuf> {
    let cand1 = exe_dir.join("chronokeep_gdrive.json");
    if cand1.exists() {
        return Some(cand1);
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            let cand2 = parent.join("chronokeep_gdrive.json");
            if cand2.exists() {
                return Some(cand2);
            }
        }
    }
    None
}

pub fn is_gdrive_configured(exe_dir: &Path) -> bool {
    resolve_token_path(exe_dir).is_some()
}

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

/// Bounded synchronous probe of the app's remote `manifest.tsv` on Google Drive.
/// Uses a 3s serve-cache and an 8s cooldown circuit breaker after 2 consecutive failures.
pub fn fetch_cloud_manifest_bounded(
    steam_dir: &Path,
    account_id: u32,
    app_id: u32,
    vault: &VaultStorage,
    log_path: Option<&PathBuf>,
    timeout: Duration,
) -> CloudManifestFetch {
    if let Some(cached) = get_serve_cache(account_id, app_id) {
        return cached;
    }

    let Some(token_path) = resolve_token_path(steam_dir) else {
        return CloudManifestFetch::NotConfigured;
    };

    if circuit_breaker_is_open() {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive] Circuit breaker ativo; pulando bloqueio sincrono para app={}",
                    app_id
                ),
            );
        }
        return CloudManifestFetch::Inconclusive;
    }

    let (tx, rx) = mpsc::sync_channel::<CloudManifestFetch>(1);
    let vault_clone = vault.clone();
    let lp_clone = log_path.cloned();

    std::thread::spawn(move || {
        let outcome = fetch_cloud_manifest_worker(
            &token_path,
            account_id,
            app_id,
            &vault_clone,
            lp_clone.as_ref(),
        );
        let _ = tx.send(outcome);
    });

    match rx.recv_timeout(timeout) {
        Ok(res) => {
            let ok = !matches!(res, CloudManifestFetch::Inconclusive);
            record_cloud_probe_result(ok);
            put_serve_cache(account_id, app_id, res.clone());
            res
        }
        Err(_) => {
            record_cloud_probe_result(false);
            if let Some(lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive] Timeout ({}s) ao consultar manifest.tsv na nuvem para app={}",
                        timeout.as_secs(),
                        app_id
                    ),
                );
            }
            CloudManifestFetch::Inconclusive
        }
    }
}

fn fetch_cloud_manifest_worker(
    token_path: &Path,
    account_id: u32,
    app_id: u32,
    vault: &VaultStorage,
    log_path: Option<&PathBuf>,
) -> CloudManifestFetch {
    let Some(token) = get_valid_access_token(token_path) else {
        return CloudManifestFetch::Inconclusive;
    };

    let root_id = match find_drive_folder_checked(&token, "ChronoKeep", None) {
        DriveLookup::Found(id) => id,
        DriveLookup::Missing => return CloudManifestFetch::NoCloudData,
        DriveLookup::Error => return CloudManifestFetch::Inconclusive,
    };
    let acct_id = match find_drive_folder_checked(&token, &account_id.to_string(), Some(&root_id)) {
        DriveLookup::Found(id) => id,
        DriveLookup::Missing => return CloudManifestFetch::NoCloudData,
        DriveLookup::Error => return CloudManifestFetch::Inconclusive,
    };
    let app_folder_id =
        match find_drive_folder_checked(&token, &app_id.to_string(), Some(&acct_id)) {
            DriveLookup::Found(id) => id,
            DriveLookup::Missing => return CloudManifestFetch::NoCloudData,
            DriveLookup::Error => return CloudManifestFetch::Inconclusive,
        };

    let Some(items) = list_drive_files_checked(&token, &app_folder_id) else {
        return CloudManifestFetch::Inconclusive;
    };
    if items.is_empty() {
        return CloudManifestFetch::NoCloudData;
    }

    if let Some(manifest_item) = items.iter().find(|it| {
        it.mime_type != "application/vnd.google-apps.folder"
            && it.name.eq_ignore_ascii_case("manifest.tsv")
    }) {
        let Some(bytes) = download_drive_file(&token, &manifest_item.id) else {
            return CloudManifestFetch::Inconclusive;
        };
        if let Ok(mut guard) = LAST_UPLOADED_MANIFEST.lock() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert((account_id, app_id), sha1_digest(&bytes));
        }
        let cloud_str = String::from_utf8_lossy(&bytes);
        let cloud_manifest = VaultStorage::parse_app_manifest_str(&cloud_str);
        let _ = vault.save_cloud_manifest(account_id, app_id, &cloud_manifest);
        let _ = vault.try_promote_cloud_manifest_if_complete(account_id, app_id);
        return CloudManifestFetch::Manifest(cloud_manifest);
    }

    // Pasta existe no Drive com saves manuais/legados, mas ainda sem manifest.tsv:
    // baixa os arquivos e sintetiza o manifesto.
    let mut cloud_is_newer = false;
    let mut all_ok = true;
    let _ = pull_drive_folder_recursive(
        &token,
        &app_folder_id,
        "",
        account_id,
        app_id,
        vault,
        None,
        &mut cloud_is_newer,
        &mut all_ok,
        log_path,
    );
    if !all_ok {
        return CloudManifestFetch::Inconclusive;
    }
    let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
    let _ = vault.save_cloud_manifest(account_id, app_id, &manifest);
    CloudManifestFetch::Manifest(manifest)
}

/// Downloads a single save file on-demand from `ChronoKeep/<account_id>/<app_id>/<rel_filename>`,
/// verifies its SHA-1 against `expected_sha1` (if provided), caches it in the local Vault,
/// and mirrors to `userdata/remote` ONLY when `root_token` is empty.
pub fn download_app_file_on_demand(
    steam_dir: &Path,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    expected_sha1: Option<[u8; 20]>,
    root_token: &str,
    vault: &VaultStorage,
    log_path: Option<&PathBuf>,
) -> Option<Vec<u8>> {
    let token_path = resolve_token_path(steam_dir)?;
    let token = get_valid_access_token(&token_path)?;

    let root_id = match find_drive_folder_checked(&token, "ChronoKeep", None) {
        DriveLookup::Found(id) => id,
        _ => return None,
    };
    let acct_id = match find_drive_folder_checked(&token, &account_id.to_string(), Some(&root_id)) {
        DriveLookup::Found(id) => id,
        _ => return None,
    };
    let mut parent_id =
        match find_drive_folder_checked(&token, &app_id.to_string(), Some(&acct_id)) {
            DriveLookup::Found(id) => id,
            _ => return None,
        };

    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    for sub in &parts[..parts.len() - 1] {
        parent_id = match find_drive_folder_checked(&token, sub, Some(&parent_id)) {
            DriveLookup::Found(id) => id,
            _ => return None,
        };
    }
    let leaf = parts[parts.len() - 1];

    let meta = match find_drive_file_meta(&token, &parent_id, leaf) {
        DriveLookup::Found((m, _)) => m,
        _ => return None,
    };

    let bytes = download_drive_file(&token, &meta.id)?;
    if let Some(exp) = expected_sha1 {
        let got = sha1_digest(&bytes);
        if got != exp {
            if let Some(lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive Download] ERRO DE INTEGRIDADE SHA-1: app={} file='{}' esperado={} recebido={} -> rejeitando!",
                        app_id,
                        rel_filename,
                        to_hex(&exp),
                        to_hex(&got)
                    ),
                );
            }
            return None;
        }
    }

    let _ = vault.write_save_file(account_id, app_id, rel_filename, &bytes);
    if root_token.is_empty() {
        if let Some(ref sp) = vault.steam_path {
            let user_dest = sp
                .join("userdata")
                .join(account_id.to_string())
                .join(app_id.to_string())
                .join("remote")
                .join(rel_filename);
            if let Some(p) = user_dest.parent() {
                let _ = fs::create_dir_all(p);
            }
            let _ = fs::write(&user_dest, &bytes);
        }
    }
    let _ = vault.try_promote_cloud_manifest_if_complete(account_id, app_id);

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive Download] Sob demanda concluido e verificado: app={} file='{}' ({}B)",
                app_id,
                rel_filename,
                bytes.len()
            ),
        );
    }

    Some(bytes)
}

#[derive(Debug, Clone)]
pub enum BatchUploadOutcome {
    Succeeded(AppManifest),
    Failed,
    StillInProgress,
}

struct InflightBudget {
    used: Mutex<usize>,
    cv: Condvar,
}

impl InflightBudget {
    fn new() -> Self {
        Self {
            used: Mutex::new(0),
            cv: Condvar::new(),
        }
    }

    fn acquire(&self, bytes: usize) {
        let mut guard = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while *guard > 0 && guard.saturating_add(bytes) > MAX_INFLIGHT_UPLOAD_BYTES {
            guard = self.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        *guard = guard.saturating_add(bytes);
    }

    fn release(&self, bytes: usize) {
        let mut guard = self.used.lock().unwrap_or_else(|e| e.into_inner());
        *guard = guard.saturating_sub(bytes);
        self.cv.notify_all();
    }
}

/// Executes an atomic batch upload to Google Drive:
/// - Uploads all `committed_files` across up to 4 parallel workers (capped at 24 MiB in-flight);
/// - Deletes `deleted_files` on Google Drive;
/// - Advances `change_number = assigned_cn` and uploads `manifest.tsv` ONLY if 100% of files succeed!
/// - Waits up to `wait_timeout` for completion before returning `StillInProgress` (preventing Steam's 15s watchdog).
#[allow(clippy::too_many_arguments)]
pub fn execute_batch_upload_bounded(
    steam_dir: PathBuf,
    account_id: u32,
    app_id: u32,
    base_cn: u64,
    assigned_cn: u64,
    app_build_id: u64,
    committed_files: Vec<(String, Vec<u8>)>,
    deleted_files: Vec<String>,
    vault: VaultStorage,
    log_path: Option<PathBuf>,
    wait_timeout: Duration,
) -> BatchUploadOutcome {
    invalidate_serve_cache(account_id, app_id);
    let upload_guard = ActiveUploadGuard::acquire(account_id, app_id);
    let (done_tx, done_rx) = mpsc::sync_channel::<Result<AppManifest, ()>>(1);
    let lp_for_timeout = log_path.clone();

    std::thread::spawn(move || {
        let _active = upload_guard;
        let app_lock = get_app_upload_lock(account_id, app_id);
        let _lock_guard = app_lock.lock().unwrap_or_else(|e| e.into_inner());

        let result = run_batch_upload_locked(
            &steam_dir,
            account_id,
            app_id,
            base_cn,
            assigned_cn,
            app_build_id,
            committed_files,
            deleted_files,
            &vault,
            log_path.as_ref(),
        );
        let _ = done_tx.send(result);
    });

    match done_rx.recv_timeout(wait_timeout) {
        Ok(Ok(manifest)) => BatchUploadOutcome::Succeeded(manifest),
        Ok(Err(())) => BatchUploadOutcome::Failed,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if let Some(ref lp) = lp_for_timeout {
                crate::log_msg(
                    lp,
                    &format!(
                        "[BatchUpload] Upload de lote grande ainda em andamento apos {}s (app={}); concluindo em segundo plano com protecao de shutdown.",
                        wait_timeout.as_secs(),
                        app_id
                    ),
                );
            }
            BatchUploadOutcome::StillInProgress
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => BatchUploadOutcome::Failed,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_batch_upload_locked(
    steam_dir: &Path,
    account_id: u32,
    app_id: u32,
    base_cn: u64,
    assigned_cn: u64,
    app_build_id: u64,
    committed_files: Vec<(String, Vec<u8>)>,
    deleted_files: Vec<String>,
    vault: &VaultStorage,
    log_path: Option<&PathBuf>,
) -> Result<AppManifest, ()> {
    let Some(token_path) = resolve_token_path(steam_dir) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[BatchUpload] Abortado: chronokeep_gdrive.json ausente em {:?}",
                    steam_dir
                ),
            );
        }
        return Err(());
    };

    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[BatchUpload] Erro: Falha ao obter access_token valido para o lote.",
            );
        }
        return Err(());
    };

    let mut app_folder_id = ensure_app_drive_folder(&token, account_id, app_id);
    if app_folder_id.is_none() {
        clear_folder_cache();
        app_folder_id = ensure_app_drive_folder(&token, account_id, app_id);
    }
    let Some(app_folder_id) = app_folder_id else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[BatchUpload] Erro: Falha ao resolver pasta ChronoKeep/{}/{} no Google Drive.",
                    account_id, app_id
                ),
            );
        }
        return Err(());
    };

    // Pré-aquece o cache da pasta raiz do jogo em 1 única listagem e lê o manifest.tsv remoto atual (para merge de delta)
    let mut remote_manifest: Option<AppManifest> = None;
    if let Some(root_items) = list_drive_files_checked(&token, &app_folder_id) {
        if let Some(m_item) = root_items.iter().find(|it| {
            it.mime_type != "application/vnd.google-apps.folder"
                && it.name.eq_ignore_ascii_case("manifest.tsv")
        }) {
            if let Some(m_bytes) = download_drive_file(&token, &m_item.id) {
                let s = String::from_utf8_lossy(&m_bytes);
                remote_manifest = Some(VaultStorage::parse_app_manifest_str(&s));
            }
        }
    }

    // Pré-cria eventuais subpastas de forma determinística antes de paralelizar os workers
    let mut prepared_uploads: VecDeque<(String, String, String, Vec<u8>)> =
        VecDeque::with_capacity(committed_files.len());
    for (rel_name, bytes) in committed_files {
        let Some((parent_id, leaf)) = resolve_or_create_subpath(&token, &app_folder_id, &rel_name)
        else {
            if let Some(lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[BatchUpload] Erro ao preparar subpasta para app={} file='{}'",
                        app_id, rel_name
                    ),
                );
            }
            return Err(());
        };
        prepared_uploads.push_back((rel_name, parent_id, leaf, bytes));
    }

    let any_failed = AtomicBool::new(false);
    if !prepared_uploads.is_empty() {
        let worker_count = prepared_uploads.len().min(MAX_BATCH_WORKERS).max(1);
        let queue = Mutex::new(prepared_uploads);
        let budget = InflightBudget::new();

        std::thread::scope(|s| {
            for _ in 0..worker_count {
                let q_ref = &queue;
                let b_ref = &budget;
                let failed_ref = &any_failed;
                let tok_ref = token.as_str();
                s.spawn(move || loop {
                    if failed_ref.load(Ordering::Acquire) {
                        break;
                    }
                    let next_item = {
                        let mut g = q_ref.lock().unwrap_or_else(|e| e.into_inner());
                        g.pop_front()
                    };
                    let Some((rel_name, parent_id, leaf, bytes)) = next_item else {
                        break;
                    };

                    let byte_len = bytes.len();
                    b_ref.acquire(byte_len);
                    let ok = upload_file_multipart(tok_ref, &parent_id, &leaf, &bytes);
                    b_ref.release(byte_len);

                    if ok {
                        if let Some(lp) = log_path {
                            crate::log_msg(
                                lp,
                                &format!(
                                    "[BatchUpload] OK: app={} file='{}' ({}B)",
                                    app_id, rel_name, byte_len
                                ),
                            );
                        }
                    } else {
                        failed_ref.store(true, Ordering::Release);
                        if let Some(lp) = log_path {
                            crate::log_msg(
                                lp,
                                &format!(
                                    "[BatchUpload] ERRO: Falha ao enviar app={} file='{}' ({}B)",
                                    app_id, rel_name, byte_len
                                ),
                            );
                        }
                        break;
                    }
                });
            }
        });
    }

    if !any_failed.load(Ordering::Acquire) {
        for del_rel in &deleted_files {
            if !delete_rel_path_on_drive(&token, &app_folder_id, del_rel) {
                any_failed.store(true, Ordering::Release);
                if let Some(lp) = log_path {
                    crate::log_msg(
                        lp,
                        &format!(
                            "[BatchUpload] ERRO: Falha ao remover arquivo deletado no Drive app={} file='{}'",
                            app_id, del_rel
                        ),
                    );
                }
                break;
            }
        }
    }

    if any_failed.load(Ordering::Acquire) {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[BatchUpload] ABORTADO: Lote de app={} teve falha; manifest.tsv NAO foi publicado e CN permanece em {}.",
                    app_id, base_cn
                ),
            );
        }
        return Err(());
    }

    // Todos os arquivos do lote subiram com 100% de sucesso!
    // Faz merge seguro com entradas remotas não deletadas e promove o CN.
    let deleted_set: HashSet<String> = deleted_files.into_iter().collect();
    let mut final_manifest = vault.load_and_reconcile_manifest(account_id, app_id);
    if let Some(rem) = remote_manifest.or_else(|| vault.load_cloud_manifest(account_id, app_id)) {
        if final_manifest.default_root_token.is_empty() && !rem.default_root_token.is_empty() {
            final_manifest.default_root_token = rem.default_root_token;
        }
        for (k, v) in rem.files {
            if !deleted_set.contains(&k) && !final_manifest.files.contains_key(&k) {
                final_manifest.files.insert(k, v);
            }
        }
    }
    for del_k in &deleted_set {
        final_manifest.files.remove(del_k);
    }

    let final_cn = assigned_cn.max(final_manifest.change_number);
    final_manifest.change_number = final_cn;
    if app_build_id != 0 {
        final_manifest.app_build_id = app_build_id;
    }

    let manifest_str = VaultStorage::serialize_app_manifest(&final_manifest);
    let manifest_bytes = manifest_str.as_bytes();
    let manifest_hash = sha1_digest(manifest_bytes);

    if !upload_file_multipart(&token, &app_folder_id, "manifest.tsv", manifest_bytes) {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[BatchUpload] ERRO: Saves enviados, mas falhou ao publicar manifest.tsv para app={}; CN local mantido em {}.",
                    app_id, base_cn
                ),
            );
        }
        return Err(());
    }

    let _ = vault.save_manifest(account_id, app_id, &final_manifest);
    let _ = vault.save_cloud_manifest(account_id, app_id, &final_manifest);

    if let Ok(mut guard) = LAST_UPLOADED_MANIFEST.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert((account_id, app_id), manifest_hash);
    }
    put_serve_cache(
        account_id,
        app_id,
        CloudManifestFetch::Manifest(final_manifest.clone()),
    );

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[BatchUpload] Lote concluido com SUCESSO: app={} CN={} ({} arquivos no manifesto)",
                app_id,
                final_cn,
                final_manifest.files.len()
            ),
        );
    }

    Ok(final_manifest)
}

fn ensure_app_drive_folder(access_token: &str, account_id: u32, app_id: u32) -> Option<String> {
    let root_id = find_or_create_drive_folder(access_token, "ChronoKeep", None)?;
    let acct_id =
        find_or_create_drive_folder(access_token, &account_id.to_string(), Some(&root_id))?;
    find_or_create_drive_folder(access_token, &app_id.to_string(), Some(&acct_id))
}

fn resolve_or_create_subpath(
    access_token: &str,
    app_folder_id: &str,
    rel_filename: &str,
) -> Option<(String, String)> {
    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let mut parent_id = app_folder_id.to_string();
    for sub in &parts[..parts.len() - 1] {
        parent_id = find_or_create_drive_folder(access_token, sub, Some(&parent_id))?;
    }
    Some((parent_id, parts[parts.len() - 1].to_string()))
}

fn delete_rel_path_on_drive(access_token: &str, app_folder_id: &str, rel_filename: &str) -> bool {
    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }
    let mut parent_id = app_folder_id.to_string();
    for sub in &parts[..parts.len() - 1] {
        match find_drive_folder_checked(access_token, sub, Some(&parent_id)) {
            DriveLookup::Found(id) => parent_id = id,
            DriveLookup::Missing => return true,
            DriveLookup::Error => return false,
        }
    }
    delete_drive_file_in_folder(access_token, &parent_id, parts[parts.len() - 1])
}

/// Asynchronous-safe helper to upload a single file into `ChronoKeep/<account_id>/<app_id>/<rel_path>` on Google Drive.
pub fn sync_file_to_gdrive(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
    log_path: Option<&PathBuf>,
) -> bool {
    let _active = ActiveUploadGuard::acquire(account_id, app_id);
    if sync_file_to_gdrive_attempt(exe_dir, account_id, app_id, rel_filename, data, log_path) {
        return true;
    }
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
    let Some(token_path) = resolve_token_path(exe_dir) else {
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
    };

    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive Upload] Erro: Falha ao obter access_token valido com refresh_token.",
            );
        }
        return false;
    };

    let Some(app_folder_id) = ensure_app_drive_folder(&token, account_id, app_id) else {
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

    let Some((parent_id, leaf_name)) =
        resolve_or_create_subpath(&token, &app_folder_id, rel_filename)
    else {
        return false;
    };

    let ok = upload_file_multipart(&token, &parent_id, &leaf_name, data);
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

#[allow(clippy::too_many_arguments)]
fn pull_drive_folder_recursive(
    access_token: &str,
    folder_id: &str,
    rel_prefix: &str,
    account_id: u32,
    app_id: u32,
    vault: &VaultStorage,
    cloud_manifest_ref: Option<&AppManifest>,
    cloud_is_newer: &mut bool,
    all_files_ok: &mut bool,
    log_path: Option<&PathBuf>,
) -> usize {
    let Some(items) = list_drive_files_checked(access_token, folder_id) else {
        *all_files_ok = false;
        return 0;
    };
    let mut synced_count = 0;
    let mut owned_cloud_manifest: Option<AppManifest> = None;

    // Na raiz do app, lê manifest.tsv primeiro e persiste em cloud_manifest.tsv
    // (sem sobrescrever manifest.tsv local antes de concluir e verificar os downloads!)
    if rel_prefix.is_empty() {
        let mut parsed_opt = None;
        for item in &items {
            if item.mime_type != "application/vnd.google-apps.folder"
                && item.name.eq_ignore_ascii_case("manifest.tsv")
            {
                match download_drive_file(access_token, &item.id) {
                    Some(bytes) => {
                        let cloud_str = String::from_utf8_lossy(&bytes);
                        let parsed = VaultStorage::parse_app_manifest_str(&cloud_str);
                        let current_manifest = vault.load_and_reconcile_manifest(account_id, app_id);
                        if parsed.change_number > current_manifest.change_number
                            || !vault.manifest_path(account_id, app_id).exists()
                        {
                            *cloud_is_newer = parsed.change_number > current_manifest.change_number;
                        }
                        let _ = vault.save_cloud_manifest(account_id, app_id, &parsed);
                        if let Ok(mut guard) = LAST_UPLOADED_MANIFEST.lock() {
                            let map = guard.get_or_insert_with(HashMap::new);
                            map.insert((account_id, app_id), sha1_digest(&bytes));
                        }
                        parsed_opt = Some(parsed);
                    }
                    None => {
                        *all_files_ok = false;
                    }
                }
            }
        }
        owned_cloud_manifest = parsed_opt;
    }

    let active_cloud_manifest = owned_cloud_manifest.as_ref().or(cloud_manifest_ref);

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
                active_cloud_manifest,
                cloud_is_newer,
                all_files_ok,
                log_path,
            );
        } else {
            let raw_rel_file = if rel_prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{}/{}", rel_prefix, item.name)
            };
            let (_, clean_file) = cr_proto::cloud_messages::split_root_token(&raw_rel_file);
            let rel_file = if !clean_file.is_empty() {
                clean_file
            } else {
                raw_rel_file
            };

            if rel_file.eq_ignore_ascii_case("manifest.tsv") {
                continue;
            }

            // Apenas arquivos sem root_token (root=0) pertencem a Steam\userdata\<acct>\<app>\remote.
            let file_root_tok = vault.effective_root_token_for_file(account_id, app_id, &rel_file);
            let uses_userdata_remote = file_root_tok.is_empty();

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
                                            app_id, rel_file
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                continue;
            }

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
                                        app_id, rel_file
                                    ),
                                );
                            }
                        }
                    }
                }
                continue;
            }

            let local_exists_in_vault = vault_file_path
                .as_ref()
                .map(|p| p.exists())
                .unwrap_or(false);
            if local_exists_in_vault && !*cloud_is_newer {
                continue;
            }

            match download_drive_file(access_token, &item.id) {
                Some(bytes) => {
                    if let Some(cm) = active_cloud_manifest {
                        if let Some(exp_rec) = cm.files.get(&rel_file) {
                            if sha1_digest(&bytes) != exp_rec.sha1 {
                                *all_files_ok = false;
                                if let Some(lp) = log_path {
                                    crate::log_msg(
                                        lp,
                                        &format!(
                                            "[Google Drive] SHA-1 divergente no pre-warm de app={} file='{}'; arquivo ignorado.",
                                            app_id, rel_file
                                        ),
                                    );
                                }
                                continue;
                            }
                        }
                    }

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
                    } else {
                        *all_files_ok = false;
                    }
                }
                None => {
                    *all_files_ok = false;
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
    vault: &VaultStorage,
    log_path: Option<&PathBuf>,
) -> bool {
    if has_active_upload_for_app(account_id, app_id) {
        return false;
    }

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

    let Some(token_path) = resolve_token_path(steam_dir) else {
        return false;
    };
    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive] Falha ao renovar access_token com refresh_token.",
            );
        }
        return false;
    };

    let DriveLookup::Found(root_id) = find_drive_folder_checked(&token, "ChronoKeep", None) else {
        return false;
    };
    let DriveLookup::Found(acct_id) =
        find_drive_folder_checked(&token, &account_id.to_string(), Some(&root_id))
    else {
        return false;
    };
    let DriveLookup::Found(app_folder_id) =
        find_drive_folder_checked(&token, &app_id.to_string(), Some(&acct_id))
    else {
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
    let mut all_files_ok = true;
    let count = pull_drive_folder_recursive(
        &token,
        &app_folder_id,
        "",
        account_id,
        app_id,
        vault,
        None,
        &mut cloud_is_newer,
        &mut all_files_ok,
        log_path,
    );

    // Promove cloud_manifest.tsv para manifest.tsv somente quando todos os saves estiverem íntegros no Vault
    if !vault.try_promote_cloud_manifest_if_complete(account_id, app_id) && all_files_ok {
        let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
        let _ = vault.save_manifest(account_id, app_id, &manifest);
    }

    if all_files_ok {
        if let Ok(mut guard) = LAST_PULLED_AT.lock() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert((account_id, app_id), Instant::now());
        }
    }

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive] Sincronizacao concluida: {} arquivos novos/atualizados para app={} (completo={})",
                count, app_id, all_files_ok
            ),
        );
    }

    all_files_ok
}

pub fn start_background_pull_all(
    steam_dir: PathBuf,
    account_id: u32,
    apps: Vec<u32>,
    vault: VaultStorage,
    log_path: Option<PathBuf>,
) {
    let managed_set: HashSet<u32> = apps
        .into_iter()
        .filter(|&id| id != 0 && id != 7 && id != 760 && id != 241100 && id != 241300)
        .collect();
    if managed_set.is_empty() {
        return;
    }

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

    if PULL_ALL_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        struct PullAllGuard;
        impl Drop for PullAllGuard {
            fn drop(&mut self) {
                PULL_ALL_ACTIVE.store(false, Ordering::SeqCst);
            }
        }
        let _all_guard = PullAllGuard;
        let Some(token_path) = resolve_token_path(&steam_dir) else {
            return;
        };

        let Some(token) = get_valid_access_token(&token_path) else {
            if let Some(ref lp) = log_path {
                crate::log_msg(
                    lp,
                    "[Google Drive] Falha ao renovar token de acesso no pull em background.",
                );
            }
            return;
        };

        let DriveLookup::Found(root_id) = find_drive_folder_checked(&token, "ChronoKeep", None)
        else {
            return;
        };
        let DriveLookup::Found(acct_id) =
            find_drive_folder_checked(&token, &account_id.to_string(), Some(&root_id))
        else {
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

        let Some(app_folders) = list_drive_files_checked(&token, &acct_id) else {
            return;
        };
        let now = Instant::now();

        for item in app_folders {
            if item.mime_type != "application/vnd.google-apps.folder" {
                continue;
            }
            let Ok(app_id) = item.name.parse::<u32>() else {
                continue;
            };
            if app_id == 0
                || app_id == 7
                || app_id == 760
                || app_id == 241100
                || app_id == 241300
                || !managed_set.contains(&app_id)
                || has_active_upload_for_app(account_id, app_id)
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
            let mut all_files_ok = true;
            let count = pull_drive_folder_recursive(
                &token,
                &item.id,
                "",
                account_id,
                app_id,
                &vault,
                None,
                &mut cloud_is_newer,
                &mut all_files_ok,
                log_path.as_ref(),
            );

            if !vault.try_promote_cloud_manifest_if_complete(account_id, app_id) && all_files_ok {
                let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
                let _ = vault.save_manifest(account_id, app_id, &manifest);
            }

            if all_files_ok {
                if let Ok(mut guard) = LAST_PULLED_AT.lock() {
                    guard
                        .get_or_insert_with(HashMap::new)
                        .insert((account_id, app_id), Instant::now());
                }
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

        if let Ok(mut guard) = LAST_PULLED_AT.lock() {
            let map = guard.get_or_insert_with(HashMap::new);
            for &app_id in &managed_set {
                map.entry((account_id, app_id)).or_insert(now);
            }
        }
    });
}
