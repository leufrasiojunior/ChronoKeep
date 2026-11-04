//! Manifest serialization and parsing routines with backward compatibility.
//!
//! Format canonical specification:
//! META\t<change_number>\t<app_build_id>\t<default_root_token>
//! FILE\t<filename>\t<root_token>\t<sha1_hex>\t<timestamp>\t<raw_size>\t<platforms_to_sync>

use cr_proto::cloud_messages::split_root_token;
use cr_proto::sha1::{from_hex, to_hex};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestFileRecord {
    pub filename: String,
    pub root_token: String,
    pub sha1: [u8; 20],
    pub timestamp: u64,
    pub raw_size: u64,
    pub platforms_to_sync: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestData {
    pub change_number: u64,
    pub app_build_id: u64,
    pub default_root_token: String,
    pub files: HashMap<String, ManifestFileRecord>,
}

/// Serializes the manifest into canonical TSV text.
pub fn serialize_manifest(manifest: &ManifestData) -> String {
    let mut out = String::new();
    out.push_str("# cr-clone vault manifest v1\n");
    out.push_str(&format!(
        "META\t{}\t{}\t{}\n",
        manifest.change_number, manifest.app_build_id, manifest.default_root_token
    ));

    let mut sorted: Vec<&ManifestFileRecord> = manifest.files.values().collect();
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
    out
}

/// Parses manifest TSV content supporting both canonical format (META/FILE)
/// and legacy fallback format (cn/build/default_token/file).
pub fn parse_manifest(content: &str) -> ManifestData {
    let mut manifest = ManifestData::default();

    for raw_line in content.lines() {
        let line = raw_line.trim_end_matches(&['\r', '\n'][..]);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let tag = parts[0];

        // 1. Canonical format
        if tag == "META" && parts.len() >= 3 {
            manifest.change_number = parts[1].parse().unwrap_or(0);
            manifest.app_build_id = parts[2].parse().unwrap_or(0);
            if parts.len() >= 4 {
                manifest.default_root_token = parts[3].to_string();
            }
            continue;
        }

        if tag == "FILE" && parts.len() >= 7 {
            let (tok, clean) = split_root_token(parts[1]);
            let filename = clean;
            if filename.is_empty() {
                continue;
            }
            let root_token = if !parts[2].is_empty() {
                parts[2].to_string()
            } else {
                tok
            };
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
                ManifestFileRecord {
                    filename,
                    root_token,
                    sha1,
                    timestamp,
                    raw_size,
                    platforms_to_sync,
                },
            );
            continue;
        }

        // 2. Fallback for legacy key-value format
        if tag == "cn" && parts.len() >= 2 {
            manifest.change_number = parts[1].parse().unwrap_or(0);
        } else if tag == "build" && parts.len() >= 2 {
            manifest.app_build_id = parts[1].parse().unwrap_or(0);
        } else if tag == "default_token" && parts.len() >= 2 {
            manifest.default_root_token = parts[1].to_string();
        } else if tag == "file" && parts.len() >= 6 {
            // legacy: file \t rel_name \t root_token \t sha \t file_ts \t bytes_len [\t persist \t platforms]
            let (tok, clean) = split_root_token(parts[1]);
            let filename = clean;
            if filename.is_empty() {
                continue;
            }
            let root_token = if !parts[2].is_empty() {
                parts[2].to_string()
            } else {
                tok
            };
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
            let platforms_to_sync = if parts.len() >= 8 {
                parts[7].parse().unwrap_or(0xFFFF_FFFF)
            } else {
                0xFFFF_FFFF
            };

            manifest.files.insert(
                filename.clone(),
                ManifestFileRecord {
                    filename,
                    root_token,
                    sha1,
                    timestamp,
                    raw_size,
                    platforms_to_sync,
                },
            );
        }
    }

    manifest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_manifest_roundtrip() {
        let mut manifest = ManifestData {
            change_number: 42,
            app_build_id: 100,
            default_root_token: "%WinMyDocuments%".to_string(),
            files: HashMap::new(),
        };

        manifest.files.insert(
            "save1.dat".to_string(),
            ManifestFileRecord {
                filename: "save1.dat".to_string(),
                root_token: "%WinMyDocuments%".to_string(),
                sha1: [1u8; 20],
                timestamp: 1700000000,
                raw_size: 1024,
                platforms_to_sync: 0xFFFF_FFFF,
            },
        );

        let tsv = serialize_manifest(&manifest);
        let parsed = parse_manifest(&tsv);

        assert_eq!(parsed.change_number, 42);
        assert_eq!(parsed.app_build_id, 100);
        assert_eq!(parsed.default_root_token, "%WinMyDocuments%");
        assert_eq!(parsed.files.len(), 1);
        let f = &parsed.files["save1.dat"];
        assert_eq!(f.raw_size, 1024);
        assert_eq!(f.sha1, [1u8; 20]);
    }

    #[test]
    fn test_legacy_format_compatibility() {
        let legacy_tsv = "cn\t15\nbuild\t0\ndefault_token\t\nfile\tsave.bin\t\t0102030405060708090a0b0c0d0e0f1011121314\t1680000000\t2048\t0\t4294967295\n";
        let parsed = parse_manifest(legacy_tsv);

        assert_eq!(parsed.change_number, 15);
        assert_eq!(parsed.files.len(), 1);
        let f = &parsed.files["save.bin"];
        assert_eq!(f.raw_size, 2048);
        assert_eq!(f.timestamp, 1680000000);
    }
}
