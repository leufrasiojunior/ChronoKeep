//! Local Vault Storage & Manifest Synchronization Engine.
//! Stores real, uncompressed save files under `<vault_dir>/<account_id>/<app_id>/files/`
//! and maintains atomic manifest metadata in `<vault_dir>/<account_id>/<app_id>/manifest.tsv`.

use cr_proto::cloud_messages::{sanitize_rel_path, CloudFileRecord};
use cr_proto::sha1::{from_hex, sha1_digest, to_hex};
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
    /// saves on first run if the Vault is new, and reconciles with `<vault_root>/<account_id>/<app_id>/files/`.
    pub fn load_and_reconcile_manifest(&self, account_id: u32, app_id: u32) -> AppManifest {
        if !self.manifest_path(account_id, app_id).exists()
            && !self.files_dir(account_id, app_id).exists()
        {
            self.seed_from_steam_userdata_if_present(account_id, app_id);
        }

        let mut manifest = self.load_manifest_raw(account_id, app_id);
        if manifest.change_number == 0 {
            if let Some(vdf_cn) = self.read_remotecache_change_number(account_id, app_id) {
                if vdf_cn > 0 {
                    manifest.change_number = vdf_cn;
                }
            }
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

            let needs_hash = match manifest.files.get(&rel_name) {
                Some(entry) => entry.raw_size != disk_size || entry.persist_state != 0,
                None => true,
            };

            if needs_hash {
                if let Ok(bytes) = fs::read(&full_path) {
                    let sha = sha1_digest(&bytes);
                    let root_token = manifest
                        .files
                        .get(&rel_name)
                        .map(|e| e.root_token.clone())
                        .unwrap_or_else(|| manifest.default_root_token.clone());
                    manifest.files.insert(
                        rel_name.clone(),
                        CloudFileRecord {
                            filename: rel_name,
                            root_token,
                            sha1: sha,
                            timestamp: disk_mtime,
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
            if manifest.change_number == 0 {
                manifest.change_number = 1;
            } else {
                manifest.change_number = manifest.change_number.saturating_add(1);
            }
            let _ = self.save_manifest(account_id, app_id, &manifest);
        }

        manifest
    }

    /// On first initialization of an AppID in the Vault, copies any existing save files from:
    /// 1) `<SteamDir>/userdata/<account_id>/<app_id>/remote/`
    /// 2) `<SteamDir>/cloud_redirect_cache/<account_id>/<app_id>/` (if migrating from original CloudRedirect)
    fn seed_from_steam_userdata_if_present(&self, account_id: u32, app_id: u32) {
        let Some(ref steam_root) = self.steam_path else {
            return;
        };

        let candidates = [
            steam_root
                .join("userdata")
                .join(account_id.to_string())
                .join(app_id.to_string())
                .join("remote"),
            steam_root
                .join("cloud_redirect")
                .join("blobs")
                .join(account_id.to_string())
                .join(app_id.to_string()),
        ];

        for src_dir in candidates {
            if !src_dir.exists() {
                continue;
            }
            let mut found = Vec::new();
            collect_files_recursive(&src_dir, &src_dir, &mut found);
            for (rel_name, src_path) in found {
                if rel_name.starts_with('.') || rel_name.contains(".cloudredirect") {
                    continue;
                }
                if let Ok(bytes) = fs::read(&src_path) {
                    let _ = self.write_save_file(account_id, app_id, &rel_name, &bytes);
                }
            }
        }
    }

    fn read_remotecache_change_number(&self, account_id: u32, app_id: u32) -> Option<u64> {
        let steam_root = self.steam_path.as_ref()?;
        let vdf_path = steam_root
            .join("userdata")
            .join(account_id.to_string())
            .join(app_id.to_string())
            .join("remotecache.vdf");
        let content = fs::read_to_string(vdf_path).ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("\"ChangeNumber\"") {
                let parts: Vec<&str> = trimmed.split('"').collect();
                if parts.len() >= 4 {
                    if let Ok(cn) = parts[3].parse::<u64>() {
                        return Some(cn);
                    }
                }
            }
        }
        None
    }

    fn load_manifest_raw(&self, account_id: u32, app_id: u32) -> AppManifest {
        let path = self.manifest_path(account_id, app_id);
        let Ok(content) = fs::read_to_string(&path) else {
            return AppManifest::default();
        };
        let mut manifest = AppManifest::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts[0] == "META" && parts.len() >= 4 {
                manifest.change_number = parts[1].parse().unwrap_or(0);
                manifest.app_build_id = parts[2].parse().unwrap_or(0);
                manifest.default_root_token = parts[3].to_string();
            } else if parts[0] == "FILE" && parts.len() >= 7 {
                let filename = sanitize_rel_path(parts[1]);
                if filename.is_empty() {
                    continue;
                }
                let root_token = parts[2].to_string();
                let Some(sha_vec) = from_hex(parts[3]) else {
                    continue;
                };
                if sha_vec.len() != 20 {
                    continue;
                }
                let mut sha1 = [0u8; 20];
                sha1.copy_from_slice(&sha_vec);
                let timestamp = parts[4].parse().unwrap_or(0);
                let raw_size = parts[5].parse().unwrap_or(0);
                let platforms_to_sync = parts[6].parse().unwrap_or(0xFFFF_FFFF);
                manifest.files.insert(
                    filename.clone(),
                    CloudFileRecord {
                        filename,
                        root_token,
                        sha1,
                        timestamp,
                        raw_size,
                        persist_state: 0,
                        platforms_to_sync,
                    },
                );
            }
        }
        manifest
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

        let mut out = String::new();
        out.push_str("# cr-clone vault manifest v1\n");
        out.push_str(&format!(
            "META\t{}\t{}\t{}\n",
            manifest.change_number, manifest.app_build_id, manifest.default_root_token
        ));

        let mut sorted: Vec<&CloudFileRecord> = manifest.files.values().collect();
        sorted.sort_by(|a, b| a.filename.cmp(&b.filename));
        for f in sorted {
            out.push_str(&format!(
                "FILE\t{}\t{}\t{}\t{}\t{}\t{}\n",
                f.filename,
                f.root_token,
                to_hex(&f.sha1),
                f.timestamp,
                f.raw_size,
                f.platforms_to_sync
            ));
        }

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
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(base, &path, out);
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "cr_tmp" {
                    continue;
                }
            }
            if let Ok(rel) = path.strip_prefix(base) {
                let rel_str = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let clean = sanitize_rel_path(&rel_str);
                if !clean.is_empty() {
                    out.push((clean, path));
                }
            }
        }
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1_700_000_000)
}

