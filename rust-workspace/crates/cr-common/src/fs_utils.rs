//! Filesystem recursive scanning and path sanitization helpers.

use cr_proto::cloud_messages::sanitize_rel_path;
use std::fs;
use std::path::{Path, PathBuf};

/// Recursively collects valid files from `current` relative to `base`.
/// Ignores `.cr_tmp` temporary files.
pub fn collect_files_recursive(base: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(base, &path, out);
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "cr_tmp" || ext == "tmp" {
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

/// Formats a byte size into a human-readable string (e.g. "1.5 MB").
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
