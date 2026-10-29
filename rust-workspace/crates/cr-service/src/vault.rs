//! Local Vault Storage & Manifest Synchronization Engine.
//! Stores real, uncompressed save files under `<vault_dir>/<account_id>/<app_id>/files/`
//! and maintains atomic manifest metadata in `<vault_dir>/<account_id>/<app_id>/manifest.tsv`.

use cr_proto::cloud_messages::{sanitize_rel_path, CloudFileRecord};
use cr_proto::sha1::sha1_digest;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct AppManifest {
    pub change_number: u64,
    pub app_build_id: u64,
    pub default_root_token: String,
    pub files: HashMap<String, CloudFileRecord>,
}

impl Default for AppManifest {
    fn default() -> Self {
        Self {
            change_number: 0,
            app_build_id: 0,
            default_root_token: String::new(),
            files: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VaultStorage {
    pub vault_root: PathBuf,
    pub steam_path: Option<PathBuf>,
}

impl VaultStorage {
    pub fn new(vault_root: PathBuf, steam_path: Option<PathBuf>) -> Self {
        let _ = fs::create_dir_all(&vault_root);
        Self {
            vault_root,
            steam_path,
        }
    }

    pub fn set_steam_path(&mut self, path: PathBuf) {
        self.steam_path = Some(path);
    }

    pub fn app_dir(&self, account_id: u32, app_id: u32) -> PathBuf {
        self.vault_root
            .join(account_id.to_string())
            .join(app_id.to_string())
    }

    pub fn files_dir(&self, account_id: u32, app_id: u32) -> PathBuf {
        self.app_dir(account_id, app_id).join("files")
    }

    pub fn manifest_path(&self, account_id: u32, app_id: u32) -> PathBuf {
        self.app_dir(account_id, app_id).join("manifest.tsv")
    }

    pub fn resolve_file_disk_path(
        &self,
        account_id: u32,
        app_id: u32,
        clean_filename: &str,
    ) -> Option<PathBuf> {
        let safe_rel = sanitize_rel_path(clean_filename);
        if safe_rel.is_empty() {
            return None;
        }
        let mut full = self.files_dir(account_id, app_id);
        for seg in safe_rel.split('/') {
            full.push(seg);
        }
        Some(full)
    }

    /// Loads the manifest for `(account_id, app_id)`, importing existing local Steam `userdata`
    /// and legacy `cloud_redirect\storage` saves, and reconciles with `<vault_root>/<account_id>/<app_id>/files/`.
    pub fn load_and_reconcile_manifest(&self, account_id: u32, app_id: u32) -> AppManifest {
        let is_first_seed = !self.manifest_path(account_id, app_id).exists();
        self.seed_from_steam_userdata_if_present(account_id, app_id);

        let mut manifest = self.load_manifest_raw(account_id, app_id);
        let (vdf_cn_opt, vdf_files) = self.read_remotecache_metadata(account_id, app_id);
        let legacy_tokens = self.read_legacy_file_tokens(account_id, app_id);
        let vdf_cn = vdf_cn_opt.unwrap_or(0);
        if manifest.change_number < vdf_cn {
            manifest.change_number = vdf_cn;
        }

        let files_root = self.files_dir(account_id, app_id);
        let mut disk_files = Vec::new();
        if files_root.exists() {
            collect_files_recursive(&files_root, &files_root, &mut disk_files);
        }

        let mut modified = false;
        let mut seen_on_disk = std::collections::HashSet::new();

        for (rel_name, full_path) in disk_files {
            seen_on_disk.insert(rel_name.clone());
            let Ok(meta) = fs::metadata(&full_path) else {
                continue;
            };
            let disk_size = meta.len();
            let disk_mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or_else(now_unix);

            let vdf_meta = vdf_files.get(&rel_name);
            let preferred_token = legacy_tokens
                .get(&rel_name)
                .cloned()
                .or_else(|| vdf_meta.map(|(tok, _)| tok.clone()))
                .filter(|t| !t.is_empty());

            let needs_hash = match manifest.files.get(&rel_name) {
                Some(entry) => {
                    let token_mismatch = preferred_token
                        .as_ref()
                        .map(|pt| entry.root_token.is_empty() && !pt.is_empty())
                        .unwrap_or(false);
                    entry.raw_size != disk_size
                        || disk_mtime > entry.timestamp
                        || entry.persist_state != 0
                        || token_mismatch
                }
                None => true,
            };

            if needs_hash {
                if let Ok(bytes) = fs::read(&full_path) {
                    let sha = sha1_digest(&bytes);
                    let root_token = preferred_token
                        .or_else(|| {
                            manifest
                                .files
                                .get(&rel_name)
                                .map(|e| e.root_token.clone())
                                .filter(|t| !t.is_empty())
                        })
                        .unwrap_or_else(|| manifest.default_root_token.clone());
                    let ts = vdf_meta
                        .map(|(_, t)| *t)
                        .filter(|&t| t > 0)
                        .unwrap_or(disk_mtime);
                    manifest.files.insert(
                        rel_name.clone(),
                        CloudFileRecord {
                            filename: rel_name,
                            root_token,
                            sha1: sha,
                            timestamp: ts,
                            raw_size: bytes.len() as u64,
                            persist_state: 0,
                            platforms_to_sync: 0xFFFF_FFFF,
                        },
                    );
                    modified = true;
                }
            }
        }

        let before_len = manifest.files.len();
        manifest.files.retain(|k, _| seen_on_disk.contains(k));
        if manifest.files.len() != before_len {
            modified = true;
        }

        if modified {
            if is_first_seed && vdf_cn > 0 {
                manifest.change_number = vdf_cn;
            } else if manifest.change_number == 0 {
                manifest.change_number = 1;
            }
            // CN nunca deve ser incrementado aqui por mudanças locais no disco.
            // O CN só avança através de upload de batch da Steam ou download do Google Drive.
            let _ = self.save_manifest(account_id, app_id, &manifest);
        }

        manifest
    }

    /// Scans local Steam `userdata` (and legacy cloud_redirect folders) for new or modified
    /// save files, syncs them into the Vault storage, and returns `Vec<(rel_name, bytes)>` of modified files.
    pub fn sync_local_saves_to_vault(
        &self,
        account_id: u32,
        app_id: u32,
    ) -> Vec<(String, Vec<u8>)> {
        let mut modified = Vec::new();
        let mut candidates = Vec::new();
        if let Some(ref steam_root) = self.steam_path {
            candidates.push(
                steam_root
                    .join("userdata")
                    .join(account_id.to_string())
                    .join(app_id.to_string())
                    .join("remote"),
            );
            candidates.push(
                steam_root
                    .join("cloud_redirect")
                    .join("storage")
                    .join(account_id.to_string())
                    .join(app_id.to_string()),
            );
        }
        candidates.push(
            PathBuf::from(r"C:\Backup_CloudRedirect\Steam_Folder\cloud_redirect\storage")
                .join(account_id.to_string())
                .join(app_id.to_string()),
        );

        for src_dir in candidates {
            if !src_dir.exists() {
                continue;
            }
            let mut found = Vec::new();
            collect_files_recursive(&src_dir, &src_dir, &mut found);
            for (rel_name, src_path) in found {
                let lower = rel_name.to_lowercase();
                if lower.starts_with('.')
                    || lower.contains(".cloudredirect")
                    || lower == "cn.dat"
                    || lower == "roottoken.dat"
                    || lower == "root_token.dat"
                    || lower == "filetokens.dat"
                    || lower == "file_tokens.dat"
                    || lower == "manifest.dat"
                    || lower == "deleted.dat"
                    || lower.ends_with(".tmp")
                {
                    continue;
                }
                let Some(dest) = self.resolve_file_disk_path(account_id, app_id, &rel_name) else {
                    continue;
                };

                let Ok(src_bytes) = fs::read(&src_path) else {
                    continue;
                };

                let is_different = if dest.exists() {
                    match fs::read(&dest) {
                        Ok(dest_bytes) => dest_bytes != src_bytes,
                        Err(_) => true,
                    }
                } else {
                    true
                };

                if is_different {
                    if self
                        .write_save_file(account_id, app_id, &rel_name, &src_bytes)
                        .is_ok()
                    {
                        modified.push((rel_name, src_bytes));
                    }
                }
            }
        }

        modified
    }

    /// Imports any missing or modified save files from Steam userdata into the Vault.
    fn seed_from_steam_userdata_if_present(&self, account_id: u32, app_id: u32) {
        let _ = self.sync_local_saves_to_vault(account_id, app_id);
    }

    fn read_legacy_file_tokens(&self, account_id: u32, app_id: u32) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let mut dirs = Vec::new();
        if let Some(ref steam_root) = self.steam_path {
            dirs.push(
                steam_root
                    .join("cloud_redirect")
                    .join("storage")
                    .join(account_id.to_string())
                    .join(app_id.to_string()),
            );
            dirs.push(
                steam_root
                    .join("cloud_redirect")
                    .join("metadata")
                    .join(account_id.to_string())
                    .join(app_id.to_string()),
            );
        }
        dirs.push(
            PathBuf::from(r"C:\Backup_CloudRedirect\Steam_Folder\cloud_redirect\storage")
                .join(account_id.to_string())
                .join(app_id.to_string()),
        );
        dirs.push(
            PathBuf::from(r"C:\Backup_CloudRedirect\Steam_Folder\cloud_redirect\metadata")
                .join(account_id.to_string())
                .join(app_id.to_string()),
        );

        for d in dirs {
            for fname in ["file_tokens.cloudredirect", "file_tokens.dat"] {
                let p = d.join(fname);
                if let Ok(content) = fs::read_to_string(&p) {
                    for line in content.lines() {
                        let line = line.trim_end_matches(&['\r', '\n'][..]);
                        if let Some((clean, tok)) = line.split_once('\t') {
                            let clean = sanitize_rel_path(clean);
                            if !clean.is_empty() && !tok.is_empty() {
                                out.entry(clean).or_insert_with(|| tok.to_string());
                            }
                        }
                    }
                }
            }
        }
        out
    }

    fn read_remotecache_metadata(
        &self,
        account_id: u32,
        app_id: u32,
    ) -> (Option<u64>, HashMap<String, (String, u64)>) {
        let mut cn_opt = None;
        let mut files_meta = HashMap::new();
        let Some(ref steam_root) = self.steam_path else {
            return (None, files_meta);
        };
        let vdf_path = steam_root
            .join("userdata")
            .join(account_id.to_string())
            .join(app_id.to_string())
            .join("remotecache.vdf");
        let Ok(content) = fs::read_to_string(vdf_path) else {
            return (None, files_meta);
        };

        let app_str = app_id.to_string();
        let mut current_file: Option<String> = None;
        let mut cur_root_id: u32 = 0;
        let mut cur_time: u64 = 0;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "}" {
                if let Some(fname) = current_file.take() {
                    let (tok, clean) = cr_proto::cloud_messages::split_root_token(&fname);
                    if !clean.is_empty() {
                        let effective_tok = if !tok.is_empty() {
                            tok
                        } else {
                            root_id_to_token(cur_root_id)
                        };
                        files_meta.insert(clean, (effective_tok, cur_time));
                    }
                }
                continue;
            }
            let parts: Vec<&str> = trimmed.split('"').collect();
            if parts.len() >= 4 {
                let key = parts[1];
                let val = parts[3];
                if current_file.is_none() && key.eq_ignore_ascii_case("ChangeNumber") {
                    if let Ok(cn) = val.parse::<u64>() {
                        cn_opt = Some(cn);
                    }
                } else if current_file.is_some() {
                    if key.eq_ignore_ascii_case("root") {
                        cur_root_id = val.parse::<u32>().unwrap_or(0);
                    } else if key.eq_ignore_ascii_case("remotetime")
                        || (key.eq_ignore_ascii_case("time") && cur_time == 0)
                    {
                        cur_time = val.parse::<u64>().unwrap_or(0);
                    }
                }
            } else if parts.len() >= 2 && current_file.is_none() {
                let key = parts[1];
                if key != app_str && !key.is_empty() {
                    current_file = Some(key.to_string());
                    cur_root_id = 0;
                    cur_time = 0;
                }
            }
        }
        (cn_opt, files_meta)
    }

    fn load_manifest_raw(&self, account_id: u32, app_id: u32) -> AppManifest {
        let path = self.manifest_path(account_id, app_id);
        let Ok(content) = fs::read_to_string(&path) else {
            return AppManifest::default();
        };
        let parsed = cr_common::manifest::parse_manifest(&content);
        AppManifest {
            change_number: parsed.change_number,
            app_build_id: parsed.app_build_id,
            default_root_token: parsed.default_root_token,
            files: parsed
                .files
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        CloudFileRecord {
                            filename: v.filename,
                            root_token: v.root_token,
                            sha1: v.sha1,
                            timestamp: v.timestamp,
                            raw_size: v.raw_size,
                            persist_state: 0,
                            platforms_to_sync: v.platforms_to_sync,
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn save_manifest(
        &self,
        account_id: u32,
        app_id: u32,
        manifest: &AppManifest,
    ) -> std::io::Result<()> {
        let dir = self.app_dir(account_id, app_id);
        fs::create_dir_all(&dir)?;
        let path = self.manifest_path(account_id, app_id);
        let tmp_path = path.with_extension("tsv.tmp");

        let data = cr_common::manifest::ManifestData {
            change_number: manifest.change_number,
            app_build_id: manifest.app_build_id,
            default_root_token: manifest.default_root_token.clone(),
            files: manifest
                .files
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        cr_common::manifest::ManifestFileRecord {
                            filename: v.filename.clone(),
                            root_token: v.root_token.clone(),
                            sha1: v.sha1,
                            timestamp: v.timestamp,
                            raw_size: v.raw_size,
                            platforms_to_sync: v.platforms_to_sync,
                        },
                    )
                })
                .collect(),
        };

        let out = cr_common::manifest::serialize_manifest(&data);

        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(out.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    pub fn write_save_file(
        &self,
        account_id: u32,
        app_id: u32,
        clean_filename: &str,
        data: &[u8],
    ) -> std::io::Result<[u8; 20]> {
        let Some(target) = self.resolve_file_disk_path(account_id, app_id, clean_filename) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid filename",
            ));
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_target = target.with_extension("cr_tmp");
        {
            let mut f = File::create(&tmp_target)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        fs::rename(&tmp_target, &target)?;
        Ok(sha1_digest(data))
    }

    pub fn read_save_file(
        &self,
        account_id: u32,
        app_id: u32,
        clean_filename: &str,
    ) -> Option<Vec<u8>> {
        let target = self.resolve_file_disk_path(account_id, app_id, clean_filename)?;
        let mut f = File::open(target).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    pub fn delete_save_file(&self, account_id: u32, app_id: u32, clean_filename: &str) -> bool {
        let Some(target) = self.resolve_file_disk_path(account_id, app_id, clean_filename) else {
            return false;
        };
        fs::remove_file(target).is_ok()
    }
}

fn collect_files_recursive(base: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) {
    cr_common::fs_utils::collect_files_recursive(base, current, out);
}

pub fn root_id_to_token(root_id: u32) -> String {
    match root_id {
        1 => "%GameInstall%".to_string(),
        2 => "%WinMyDocuments%".to_string(),
        3 => "%WinAppDataLocal%".to_string(),
        4 => "%WinAppDataRoaming%".to_string(),
        5 => "%SteamUserBaseStorage%".to_string(),
        9 => "%WinSavedGames%".to_string(),
        10 => "%WinProgramData%".to_string(),
        11 => "%SteamCloudDocuments%".to_string(),
        12 => "%WinAppDataLocalLow%".to_string(),
        14 => "%LinuxHome%".to_string(),
        15 => "%LinuxXdgDataHome%".to_string(),
        16 => "%LinuxXdgConfigHome%".to_string(),
        18 => "%WindowsHome%".to_string(),
        _ => String::new(),
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1_700_000_000)
}

