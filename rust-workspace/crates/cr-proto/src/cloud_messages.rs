//! Steam Cloud Unified Service method constants, root-token helpers, and wire builders.

use crate::wire::{find_field, ProtoField, ProtoWriter};

pub const DEFAULT_SERVICE_PORT: u16 = 39771;

// Steam Cloud Unified Service method names
pub const RPC_GET_CHANGELIST: &str = "Cloud.GetAppFileChangelist#1";
pub const RPC_BEGIN_UPLOAD: &str = "Cloud.ClientBeginFileUpload#1";
pub const RPC_COMMIT_UPLOAD: &str = "Cloud.ClientCommitFileUpload#1";
pub const RPC_FILE_DOWNLOAD: &str = "Cloud.ClientFileDownload#1";
pub const RPC_DELETE_FILE: &str = "Cloud.ClientDeleteFile#1";
pub const RPC_BEGIN_BATCH: &str = "Cloud.BeginAppUploadBatch#1";
pub const RPC_COMPLETE_BATCH: &str = "Cloud.CompleteAppUploadBatchBlocking#1";
pub const RPC_QUOTA_USAGE: &str = "Cloud.ClientGetAppQuotaUsage#1";
pub const RPC_LAUNCH_INTENT: &str = "Cloud.SignalAppLaunchIntent#1";
pub const RPC_SUSPEND_SESSION: &str = "Cloud.SuspendAppSession#1";
pub const RPC_RESUME_SESSION: &str = "Cloud.ResumeAppSession#1";
pub const RPC_EXIT_SYNC: &str = "Cloud.SignalAppExitSyncDone#1";
pub const RPC_CONFLICT: &str = "Cloud.ClientConflictResolution#1";
pub const RPC_TRANSFER_REPORT: &str = "Cloud.ExternalStorageTransferReport#1";
pub const RPC_SYNC_STATS: &str = "ClientMetrics.ClientCloudAppSyncStats#1";

pub const ERESULT_OK: i32 = 1;
pub const ERESULT_FAIL: i32 = 2;

/// Returns true if the method name is a Cloud.* RPC handled by the bridge/service.
pub fn is_handled_cloud_rpc(method: &str) -> bool {
    matches!(
        method,
        RPC_GET_CHANGELIST
            | RPC_BEGIN_UPLOAD
            | RPC_COMMIT_UPLOAD
            | RPC_FILE_DOWNLOAD
            | RPC_DELETE_FILE
            | RPC_BEGIN_BATCH
            | RPC_COMPLETE_BATCH
            | RPC_QUOTA_USAGE
            | RPC_LAUNCH_INTENT
            | RPC_SUSPEND_SESSION
            | RPC_RESUME_SESSION
            | RPC_EXIT_SYNC
            | RPC_CONFLICT
            | RPC_TRANSFER_REPORT
            | RPC_SYNC_STATS
    )
}

/// Extracts the `appid` from any `Cloud.*` request message body.
/// Most `Cloud.*` messages place `appid` at field 1, except `ClientCommitFileUpload#1` (field 2).
pub fn extract_app_id(method: &str, fields: &[ProtoField<'_>]) -> u32 {
    let field_num = if method == RPC_COMMIT_UPLOAD { 2 } else { 1 };
    find_field(fields, field_num)
        .and_then(|f| f.as_u32())
        .unwrap_or(0)
}

/// Splits a Steam Cloud path like `%WinAppDataLocal%MyGame/save.dat` into
/// `(Some("%WinAppDataLocal%"), "MyGame/save.dat")`.
pub fn split_root_token(raw_filename: &str) -> (String, String) {
    let normalized = raw_filename.replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix('%') {
        if let Some(end_idx) = rest.find('%') {
            let token = format!("%{}%", &rest[..end_idx]);
            let after = rest[end_idx + 1..].trim_start_matches('/');
            return (token, sanitize_rel_path(after));
        }
    }
    (String::new(), sanitize_rel_path(&normalized))
}

/// Sanitizes a relative cloud path, stripping leading slashes and rejecting `..` traversal.
pub fn sanitize_rel_path(path: &str) -> String {
    let mut parts = Vec::new();
    let normalized = path.replace('\\', "/");
    for seg in normalized.split('/') {
        let trimmed = seg.trim();
        if trimmed.is_empty() || trimmed == "." {
            continue;
        }
        if trimmed == ".." {
            return String::new();
        }
        parts.push(trimmed);
    }
    parts.join("/")
}

/// Splits a clean relative path `dir/sub/file.sav` into `("dir/sub/", "file.sav")`.
pub fn split_dir_and_leaf(clean_path: &str) -> (&str, &str) {
    if let Some(idx) = clean_path.rfind('/') {
        (&clean_path[..=idx], &clean_path[idx + 1..])
    } else {
        ("", clean_path)
    }
}

pub fn url_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

pub fn url_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct CloudFileRecord {
    pub filename: String,
    pub root_token: String,
    pub sha1: [u8; 20],
    pub timestamp: u64,
    pub raw_size: u64,
    pub persist_state: u32,
    pub platforms_to_sync: u32,
}

/// Builds the Protobuf response for `Cloud.GetAppFileChangelist#1`.
pub fn build_changelist_response(
    change_number: u64,
    is_only_delta: bool,
    files: &[CloudFileRecord],
    machine_name: &str,
    app_buildid_hwm: u64,
) -> Vec<u8> {
    let mut prefix_list: Vec<String> = Vec::new();
    let mut body = ProtoWriter::new();

    body.write_varint(1, change_number);
    body.write_varint(3, if is_only_delta { 1 } else { 0 });

    for f in files {
        let (dir_prefix, leaf) = split_dir_and_leaf(&f.filename);
        let full_prefix = format!("{}{}", f.root_token, dir_prefix);
        let prefix_idx = if let Some(pos) = prefix_list.iter().position(|p| p == &full_prefix) {
            pos as u32
        } else {
            let idx = prefix_list.len() as u32;
            prefix_list.push(full_prefix);
            idx
        };

        let clamped_size = f.raw_size.min(u32::MAX as u64);
        let mut sub = ProtoWriter::new();
        sub.write_str(1, leaf);
        sub.write_bytes(2, &f.sha1);
        sub.write_varint(3, f.timestamp);
        sub.write_varint(4, clamped_size);
        sub.write_varint(5, f.persist_state as u64);
        sub.write_varint(6, f.platforms_to_sync as u64);
        sub.write_varint(7, prefix_idx as u64);
        sub.write_varint(8, 0); // machine_name_index = 0
        body.write_submessage(2, &sub);
    }

    for p in &prefix_list {
        body.write_str(4, p);
    }
    body.write_str(5, machine_name);
    body.write_varint(6, app_buildid_hwm);
    body.into_bytes()
}

/// Builds the Protobuf response for `Cloud.BeginAppUploadBatch#1`.
pub fn build_begin_batch_response(batch_id: u64, app_change_number: u64) -> Vec<u8> {
    let mut w = ProtoWriter::new();
    w.write_varint(1, batch_id);
    w.write_varint(4, app_change_number);
    w.into_bytes()
}

/// Builds the Protobuf response for `Cloud.ClientBeginFileUpload#1`.
pub fn build_begin_upload_response(url_host: &str, url_path: &str, block_len: u64) -> Vec<u8> {
    let mut block_req = ProtoWriter::new();
    block_req.write_str(1, url_host);
    block_req.write_str(2, url_path);
    block_req.write_varint(3, 0); // use_https = false
    block_req.write_varint(4, 4); // http_method = PUT (4)
    block_req.write_varint(6, 0); // block_offset = 0
    block_req.write_varint(7, block_len);

    let mut body = ProtoWriter::new();
    body.write_varint(1, 0); // encrypt_file = false
    body.write_submessage(2, &block_req);
    body.into_bytes()
}

/// Builds the Protobuf response for `Cloud.ClientCommitFileUpload#1`.
pub fn build_commit_upload_response(committed: bool) -> Vec<u8> {
    let mut body = ProtoWriter::new();
    body.write_varint(1, if committed { 1 } else { 0 });
    body.into_bytes()
}

/// Builds the Protobuf response for `Cloud.ClientFileDownload#1`.
pub fn build_file_download_response(
    app_id: u32,
    file_size: u64,
    sha1: &[u8; 20],
    timestamp: u64,
    url_host: &str,
    url_path: &str,
) -> Vec<u8> {
    let clamped = file_size.min(u32::MAX as u64);
    let mut body = ProtoWriter::new();
    body.write_varint(1, app_id as u64);
    body.write_varint(2, clamped); // file_size
    body.write_varint(3, clamped); // raw_file_size
    body.write_bytes(4, sha1);
    body.write_varint(5, timestamp);
    body.write_varint(6, 0); // is_explicit_delete = false
    body.write_str(7, url_host);
    body.write_str(8, url_path);
    body.write_varint(9, 0); // use_https = false
    body.write_varint(11, 0); // encrypted = false
    body.into_bytes()
}

/// Builds the Protobuf response for `Cloud.ClientGetAppQuotaUsage#1`.
pub fn build_quota_response(
    existing_files: u32,
    existing_bytes: u64,
    max_files: u32,
    max_bytes: u64,
) -> Vec<u8> {
    let mut body = ProtoWriter::new();
    body.write_varint(1, existing_files as u64);
    body.write_varint(2, existing_bytes);
    body.write_varint(3, max_files as u64);
    body.write_varint(4, max_bytes);
    body.into_bytes()
}
