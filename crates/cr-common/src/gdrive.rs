//! Shared Google Drive REST API v3 client and OAuth2 routines.
//! Includes exponential backoff with jitter, resumable uploads (> 4 MiB),
//! MD5 content deduplication, pagination, and deterministic duplicate resolution.

use crate::http::{
    escape_json_str, extract_json_str, https_request_raw, parse_url, transfer_timeout_ms,
    url_encode, HttpResponse,
};
use cr_proto::md5::md5_hex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_GDRIVE_CLIENT_ID: &str =
    "1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com";
pub const DEFAULT_GDRIVE_CLIENT_SECRET: &str = "v6V3fKV_zWU7iw1DrpO1rknX";
pub const DEFAULT_GDRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

/// Files strictly above 4 MiB use Google Drive `uploadType=resumable`
/// (Google's `uploadType=multipart` enforces a 5 MB ceiling).
const RESUMABLE_UPLOAD_THRESHOLD: usize = 4 * 1024 * 1024;
const MAX_DRIVE_ATTEMPTS: usize = 5;

static TOKEN_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static FOLDER_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static FILE_CACHE: Mutex<Option<HashMap<String, DriveFileItem>>> = Mutex::new(None);
static FOLDER_CREATE_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveLookup<T> {
    Found(T),
    Missing,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFileItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub md5_checksum: Option<String>,
}

pub fn clear_folder_cache() {
    if let Ok(mut guard) = FOLDER_CACHE.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = FILE_CACHE.lock() {
        *guard = None;
    }
}

pub fn clear_token_cache() {
    if let Ok(mut guard) = TOKEN_CACHE.lock() {
        *guard = None;
    }
}

pub fn escape_drive_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(c),
        }
    }
    out
}

fn cache_file_item(parent_id: &str, item: DriveFileItem) {
    let key = format!("{}:{}", parent_id, item.name);
    if let Ok(mut guard) = FILE_CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(key, item);
    }
}

fn get_cached_file_item(parent_id: &str, filename: &str) -> Option<DriveFileItem> {
    let key = format!("{}:{}", parent_id, filename);
    if let Ok(guard) = FILE_CACHE.lock() {
        if let Some(ref map) = *guard {
            return map.get(&key).cloned();
        }
    }
    None
}

fn remove_cached_file_item(parent_id: &str, filename: &str) {
    let key = format!("{}:{}", parent_id, filename);
    if let Ok(mut guard) = FILE_CACHE.lock() {
        if let Some(ref mut map) = *guard {
            map.remove(&key);
        }
    }
}

fn is_retryable_drive_error(status: u32, body: &[u8]) -> bool {
    match status {
        429 | 500 | 502 | 503 | 504 => true,
        403 => {
            let s = String::from_utf8_lossy(body);
            s.contains("rateLimitExceeded")
                || s.contains("userRateLimitExceeded")
                || s.contains("backendError")
                || s.contains("sharingRateLimitExceeded")
        }
        _ => false,
    }
}

fn backoff_sleep(attempt: usize) {
    let base_ms: u64 = match attempt {
        0 => 750,
        1 => 1500,
        2 => 3000,
        _ => 6000,
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let jitter_ms = nanos % 350;
    std::thread::sleep(Duration::from_millis(base_ms + jitter_ms));
}

fn drive_request_with_retry(
    method: &str,
    host: &str,
    path: &str,
    headers: &[String],
    body: &[u8],
    timeout_ms: u32,
) -> Option<HttpResponse> {
    for attempt in 0..MAX_DRIVE_ATTEMPTS {
        match https_request_raw(method, host, path, headers, body, timeout_ms) {
            Some(resp) => {
                if resp.status == 401 {
                    clear_token_cache();
                    return Some(resp);
                }
                if is_retryable_drive_error(resp.status, &resp.body)
                    && attempt + 1 < MAX_DRIVE_ATTEMPTS
                {
                    backoff_sleep(attempt);
                    continue;
                }
                return Some(resp);
            }
            None => {
                if attempt + 1 < MAX_DRIVE_ATTEMPTS {
                    backoff_sleep(attempt);
                    continue;
                }
                return None;
            }
        }
    }
    None
}

pub fn refresh_access_token(token_file: &Path) -> Option<String> {
    let content = fs::read_to_string(token_file).ok()?;
    let refresh_token = extract_json_str(&content, "refresh_token")?;
    if refresh_token.is_empty() {
        return None;
    }

    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        url_encode(DEFAULT_GDRIVE_CLIENT_ID),
        url_encode(DEFAULT_GDRIVE_CLIENT_SECRET),
        url_encode(&refresh_token)
    );
    let headers = vec!["Content-Type: application/x-www-form-urlencoded".to_string()];
    let resp = drive_request_with_retry(
        "POST",
        "oauth2.googleapis.com",
        "/token",
        &headers,
        body.as_bytes(),
        20_000,
    )?;
    if resp.status != 200 {
        return None;
    }
    let resp_str = String::from_utf8_lossy(&resp.body);
    extract_json_str(&resp_str, "access_token")
}

pub fn get_valid_access_token(token_file: &Path) -> Option<String> {
    if let Ok(guard) = TOKEN_CACHE.lock() {
        if let Some((ref tok, ref inst)) = *guard {
            if inst.elapsed().as_secs() < 3000 {
                return Some(tok.clone());
            }
        }
    }

    let token = refresh_access_token(token_file)?;
    if let Ok(mut guard) = TOKEN_CACHE.lock() {
        *guard = Some((token.clone(), Instant::now()));
    }
    Some(token)
}

pub fn exchange_code_for_tokens(code: &str, redirect_uri: &str) -> Option<String> {
    let body = format!(
        "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
        url_encode(code),
        url_encode(DEFAULT_GDRIVE_CLIENT_ID),
        url_encode(DEFAULT_GDRIVE_CLIENT_SECRET),
        url_encode(redirect_uri)
    );
    let headers = vec!["Content-Type: application/x-www-form-urlencoded".to_string()];
    let resp = drive_request_with_retry(
        "POST",
        "oauth2.googleapis.com",
        "/token",
        &headers,
        body.as_bytes(),
        20_000,
    )?;
    if resp.status != 200 {
        return None;
    }
    String::from_utf8(resp.body).ok()
}

/// Deterministic folder lookup: orders by `createdTime` ascending so if two PCs
/// concurrently created duplicate folders, both converge on the oldest canonical folder.
pub fn find_drive_folder_checked(
    access_token: &str,
    name: &str,
    parent_id: Option<&str>,
) -> DriveLookup<String> {
    let cache_key = format!("{}:{}", parent_id.unwrap_or("root"), name);
    if let Ok(guard) = FOLDER_CACHE.lock() {
        if let Some(ref map) = *guard {
            if let Some(id) = map.get(&cache_key) {
                return DriveLookup::Found(id.clone());
            }
        }
    }

    let parent_clause = match parent_id {
        Some(pid) => format!("'{}' in parents", escape_drive_query(pid)),
        None => "'root' in parents".to_string(),
    };
    let q = format!(
        "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false and {}",
        escape_drive_query(name),
        parent_clause
    );
    let path = format!(
        "/drive/v3/files?q={}&orderBy={}&fields=files(id,name,mimeType)&pageSize=10",
        url_encode(&q),
        url_encode("createdTime")
    );
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let Some(resp) =
        drive_request_with_retry("GET", "www.googleapis.com", &path, &auth_hdr, &[], 20_000)
    else {
        return DriveLookup::Error;
    };

    if resp.status != 200 {
        if resp.status == 404 || resp.status == 400 {
            clear_folder_cache();
        }
        return DriveLookup::Error;
    }

    let s = String::from_utf8_lossy(&resp.body);
    let items = parse_drive_file_list(&s);
    if let Some(first) = items.first() {
        if !first.id.is_empty() {
            if let Ok(mut guard) = FOLDER_CACHE.lock() {
                let map = guard.get_or_insert_with(HashMap::new);
                map.insert(cache_key, first.id.clone());
            }
            return DriveLookup::Found(first.id.clone());
        }
    }
    DriveLookup::Missing
}

pub fn find_drive_folder(
    access_token: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    match find_drive_folder_checked(access_token, name, parent_id) {
        DriveLookup::Found(id) => Some(id),
        DriveLookup::Missing | DriveLookup::Error => None,
    }
}

pub fn find_or_create_drive_folder(
    access_token: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    let cache_key = format!("{}:{}", parent_id.unwrap_or("root"), name);
    if let Ok(guard) = FOLDER_CACHE.lock() {
        if let Some(ref map) = *guard {
            if let Some(id) = map.get(&cache_key) {
                return Some(id.clone());
            }
        }
    }

    // Serializa a checagem e criação para evitar que threads de upload simultâneas
    // criem pastas duplicadas no Drive.
    let _create_guard = FOLDER_CREATE_MUTEX.lock().ok();

    match find_drive_folder_checked(access_token, name, parent_id) {
        DriveLookup::Found(id) => return Some(id),
        // Nunca criar pasta nova quando a consulta falhar por erro de rede/rate-limit!
        DriveLookup::Error => return None,
        DriveLookup::Missing => {}
    }

    let safe_name = escape_json_str(name);
    let meta = match parent_id {
        Some(pid) => format!(
            "{{\"name\":\"{}\",\"mimeType\":\"application/vnd.google-apps.folder\",\"parents\":[\"{}\"]}}",
            safe_name,
            escape_json_str(pid)
        ),
        None => format!(
            "{{\"name\":\"{}\",\"mimeType\":\"application/vnd.google-apps.folder\"}}",
            safe_name
        ),
    };
    let headers = vec![
        format!("Authorization: Bearer {}", access_token),
        "Content-Type: application/json; charset=UTF-8".to_string(),
    ];
    let resp = drive_request_with_retry(
        "POST",
        "www.googleapis.com",
        "/drive/v3/files?fields=id",
        &headers,
        meta.as_bytes(),
        25_000,
    )?;
    if (200..300).contains(&resp.status) {
        let s = String::from_utf8_lossy(&resp.body);
        if let Some(id) = extract_json_str(&s, "id") {
            if !id.is_empty() {
                if let Ok(mut guard) = FOLDER_CACHE.lock() {
                    let map = guard.get_or_insert_with(HashMap::new);
                    map.insert(cache_key, id.clone());
                }
                return Some(id);
            }
        }
    } else if resp.status == 404 || resp.status == 400 {
        clear_folder_cache();
    }
    None
}

/// String-aware JSON object splitter for Drive `files: [ { ... }, { ... } ]` responses.
/// Safely handles filenames containing `{`, `}`, or escaped quotes `\"`.
pub fn parse_drive_file_list(json: &str) -> Vec<DriveFileItem> {
    let mut items = Vec::new();
    let Some(files_idx) = json.find("\"files\"") else {
        return items;
    };
    let rest = &json[files_idx + 7..];
    let Some(arr_start) = rest.find('[') else {
        return items;
    };
    let slice = &rest[arr_start + 1..];
    let bytes = slice.as_bytes();
    let mut i = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut depth = 0i32;
    let mut obj_start: Option<usize> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => {
                    if depth == 0 {
                        obj_start = Some(i + 1);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = obj_start.take() {
                            let obj_str = &slice[start..i];
                            if let (Some(id), Some(name)) = (
                                extract_json_str(obj_str, "id"),
                                extract_json_str(obj_str, "name"),
                            ) {
                                let mime_type =
                                    extract_json_str(obj_str, "mimeType").unwrap_or_default();
                                let size: u64 = extract_json_str(obj_str, "size")
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0);
                                let md5_checksum = extract_json_str(obj_str, "md5Checksum");
                                items.push(DriveFileItem {
                                    id,
                                    name,
                                    mime_type,
                                    size,
                                    md5_checksum,
                                });
                            }
                        }
                    }
                }
                b']' if depth == 0 => break,
                _ => {}
            }
        }
        i += 1;
    }

    items
}

fn delete_drive_file_by_id_best_effort(access_token: &str, file_id: &str) -> bool {
    let path = format!("/drive/v3/files/{}", url_encode(file_id));
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    if let Some(resp) =
        drive_request_with_retry("DELETE", "www.googleapis.com", &path, &auth_hdr, &[], 20_000)
    {
        return (200..300).contains(&resp.status) || resp.status == 404;
    }
    false
}

/// Paginated, error-checked listing of all non-trashed items in `parent_id`.
/// Orders by `modifiedTime desc`, populates `FILE_CACHE` & `FOLDER_CACHE`, and
/// deduplicates same-name files by keeping the most recently modified revision.
pub fn list_drive_files_checked(access_token: &str, parent_id: &str) -> Option<Vec<DriveFileItem>> {
    let q = format!("'{}' in parents and trashed=false", escape_drive_query(parent_id));
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let mut all_raw = Vec::new();
    let mut page_token: Option<String> = None;

    for _ in 0..25 {
        let mut path = format!(
            "/drive/v3/files?q={}&orderBy={}&fields=nextPageToken,files(id,name,mimeType,size,md5Checksum)&pageSize=1000",
            url_encode(&q),
            url_encode("modifiedTime desc")
        );
        if let Some(ref pt) = page_token {
            path.push_str("&pageToken=");
            path.push_str(&url_encode(pt));
        }

        let resp =
            drive_request_with_retry("GET", "www.googleapis.com", &path, &auth_hdr, &[], 30_000)?;
        if resp.status != 200 {
            if resp.status == 404 || resp.status == 400 {
                clear_folder_cache();
            }
            return None;
        }

        let s = String::from_utf8_lossy(&resp.body);
        all_raw.extend(parse_drive_file_list(&s));

        match extract_json_str(&s, "nextPageToken").filter(|t| !t.is_empty()) {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }

    let mut deduped = Vec::with_capacity(all_raw.len());
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut seen_folders: HashSet<String> = HashSet::new();

    for item in all_raw {
        if item.mime_type == "application/vnd.google-apps.folder" {
            if seen_folders.insert(item.name.clone()) {
                if let Ok(mut guard) = FOLDER_CACHE.lock() {
                    let map = guard.get_or_insert_with(HashMap::new);
                    map.insert(format!("{}:{}", parent_id, item.name), item.id.clone());
                }
                deduped.push(item);
            }
        } else if seen_files.insert(item.name.clone()) {
            cache_file_item(parent_id, item.clone());
            deduped.push(item);
        } else {
            // Limpa cópia antiga duplicada no Drive (orderBy=modifiedTime desc garante que a 1ª era a mais recente)
            let _ = delete_drive_file_by_id_best_effort(access_token, &item.id);
        }
    }

    Some(deduped)
}

pub fn list_drive_files(access_token: &str, parent_id: &str) -> Vec<DriveFileItem> {
    list_drive_files_checked(access_token, parent_id).unwrap_or_default()
}

pub fn download_drive_file(access_token: &str, file_id: &str) -> Option<Vec<u8>> {
    let path = format!("/drive/v3/files/{}?alt=media", url_encode(file_id));
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let resp =
        drive_request_with_retry("GET", "www.googleapis.com", &path, &auth_hdr, &[], 120_000)?;
    if resp.status == 200 {
        return Some(resp.body);
    }
    None
}

fn verify_cached_file_still_live(access_token: &str, file_id: &str) -> DriveLookup<DriveFileItem> {
    let path = format!(
        "/drive/v3/files/{}?fields=id,name,mimeType,size,md5Checksum,trashed",
        url_encode(file_id)
    );
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let Some(resp) =
        drive_request_with_retry("GET", "www.googleapis.com", &path, &auth_hdr, &[], 20_000)
    else {
        return DriveLookup::Error;
    };
    if resp.status == 404 {
        return DriveLookup::Missing;
    }
    if resp.status != 200 {
        return DriveLookup::Error;
    }
    let s = String::from_utf8_lossy(&resp.body);
    if s.contains("\"trashed\": true") || s.contains("\"trashed\":true") {
        return DriveLookup::Missing;
    }
    let Some(id) = extract_json_str(&s, "id") else {
        return DriveLookup::Error;
    };
    let name = extract_json_str(&s, "name").unwrap_or_default();
    let mime_type = extract_json_str(&s, "mimeType").unwrap_or_default();
    let size: u64 = extract_json_str(&s, "size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let md5_checksum = extract_json_str(&s, "md5Checksum");
    DriveLookup::Found(DriveFileItem {
        id,
        name,
        mime_type,
        size,
        md5_checksum,
    })
}

pub fn find_drive_file_meta(
    access_token: &str,
    parent_id: &str,
    filename: &str,
) -> DriveLookup<(DriveFileItem, bool)> {
    if let Some(cached) = get_cached_file_item(parent_id, filename) {
        return DriveLookup::Found((cached, true));
    }

    let q = format!(
        "name='{}' and '{}' in parents and mimeType!='application/vnd.google-apps.folder' and trashed=false",
        escape_drive_query(filename),
        escape_drive_query(parent_id)
    );
    let search_path = format!(
        "/drive/v3/files?q={}&orderBy={}&fields=files(id,name,mimeType,size,md5Checksum)&pageSize=10",
        url_encode(&q),
        url_encode("modifiedTime desc")
    );
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let Some(resp) = drive_request_with_retry(
        "GET",
        "www.googleapis.com",
        &search_path,
        &auth_hdr,
        &[],
        20_000,
    ) else {
        return DriveLookup::Error;
    };

    if resp.status != 200 {
        if resp.status == 404 || resp.status == 400 {
            clear_folder_cache();
        }
        return DriveLookup::Error;
    }

    let s = String::from_utf8_lossy(&resp.body);
    let items = parse_drive_file_list(&s);
    if items.is_empty() {
        return DriveLookup::Missing;
    }

    let newest = items[0].clone();
    for dup in items.iter().skip(1) {
        let _ = delete_drive_file_by_id_best_effort(access_token, &dup.id);
    }
    cache_file_item(parent_id, newest.clone());
    DriveLookup::Found((newest, false))
}

fn upload_multipart_small(
    access_token: &str,
    parent_id: &str,
    filename: &str,
    existing_id: Option<&str>,
    data: &[u8],
    local_md5: &str,
) -> Option<bool> {
    let boundary = "chronokeep_multipart_boundary_99182";
    let safe_name = escape_json_str(filename);
    let safe_parent = escape_json_str(parent_id);
    let meta_json = if existing_id.is_some() {
        format!("{{\"name\":\"{}\"}}", safe_name)
    } else {
        format!(
            "{{\"name\":\"{}\",\"parents\":[\"{}\"]}}",
            safe_name, safe_parent
        )
    };

    let mut body = Vec::with_capacity(meta_json.len() + data.len() + 256);
    body.extend_from_slice(
        format!(
            "--{}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{}\r\nContent-Type: application/octet-stream\r\n\r\n",
            boundary, meta_json, boundary
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let (method, upload_path) = match existing_id {
        Some(id) => (
            "PATCH",
            format!(
                "/upload/drive/v3/files/{}?uploadType=multipart&fields=id,size,md5Checksum",
                url_encode(id)
            ),
        ),
        None => (
            "POST",
            "/upload/drive/v3/files?uploadType=multipart&fields=id,size,md5Checksum".to_string(),
        ),
    };

    let headers = vec![
        format!("Authorization: Bearer {}", access_token),
        format!("Content-Type: multipart/related; boundary={}", boundary),
    ];

    let timeout = transfer_timeout_ms(body.len());
    let resp = drive_request_with_retry(
        method,
        "www.googleapis.com",
        &upload_path,
        &headers,
        &body,
        timeout,
    )?;

    if resp.status == 404 && existing_id.is_some() {
        // ID em cache ficou obsoleto (ex.: arquivo removido manualmente na web); sinaliza fallback para POST
        remove_cached_file_item(parent_id, filename);
        return None;
    }

    if (200..300).contains(&resp.status) {
        let s = String::from_utf8_lossy(&resp.body);
        if let Some(id) = extract_json_str(&s, "id").or_else(|| existing_id.map(|e| e.to_string()))
        {
            cache_file_item(
                parent_id,
                DriveFileItem {
                    id,
                    name: filename.to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    size: data.len() as u64,
                    md5_checksum: Some(local_md5.to_string()),
                },
            );
        }
        return Some(true);
    }

    Some(false)
}

fn upload_resumable_large(
    access_token: &str,
    parent_id: &str,
    filename: &str,
    existing_id: Option<&str>,
    data: &[u8],
    local_md5: &str,
) -> Option<bool> {
    let safe_name = escape_json_str(filename);
    let safe_parent = escape_json_str(parent_id);
    let meta_json = if existing_id.is_some() {
        format!("{{\"name\":\"{}\"}}", safe_name)
    } else {
        format!(
            "{{\"name\":\"{}\",\"parents\":[\"{}\"]}}",
            safe_name, safe_parent
        )
    };

    let (method, init_path) = match existing_id {
        Some(id) => (
            "PATCH",
            format!(
                "/upload/drive/v3/files/{}?uploadType=resumable&fields=id,size,md5Checksum",
                url_encode(id)
            ),
        ),
        None => (
            "POST",
            "/upload/drive/v3/files?uploadType=resumable&fields=id,size,md5Checksum".to_string(),
        ),
    };

    let init_headers = vec![
        format!("Authorization: Bearer {}", access_token),
        "Content-Type: application/json; charset=UTF-8".to_string(),
        "X-Upload-Content-Type: application/octet-stream".to_string(),
        format!("X-Upload-Content-Length: {}", data.len()),
    ];

    let init_resp = drive_request_with_retry(
        method,
        "www.googleapis.com",
        &init_path,
        &init_headers,
        meta_json.as_bytes(),
        30_000,
    )?;

    if init_resp.status == 404 && existing_id.is_some() {
        remove_cached_file_item(parent_id, filename);
        return None;
    }

    if !(200..300).contains(&init_resp.status) {
        return Some(false);
    }

    let session_url = init_resp.location?;
    let (upload_host, upload_path) = parse_url(&session_url)?;

    let put_headers = vec![
        "Content-Type: application/octet-stream".to_string(),
    ];
    let timeout = transfer_timeout_ms(data.len());
    let put_resp = drive_request_with_retry(
        "PUT",
        &upload_host,
        &upload_path,
        &put_headers,
        data,
        timeout,
    )?;

    if (200..300).contains(&put_resp.status) {
        let s = String::from_utf8_lossy(&put_resp.body);
        if let Some(id) = extract_json_str(&s, "id").or_else(|| existing_id.map(|e| e.to_string()))
        {
            cache_file_item(
                parent_id,
                DriveFileItem {
                    id,
                    name: filename.to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    size: data.len() as u64,
                    md5_checksum: Some(local_md5.to_string()),
                },
            );
        }
        return Some(true);
    }

    Some(false)
}

/// Resilient Google Drive file upload:
/// - Skips unchanged payloads via MD5 + size deduplication (`0` bytes transferred);
/// - Uses `uploadType=multipart` for payloads `<= 4 MiB` and `uploadType=resumable` for payloads `> 4 MiB`;
/// - Applies 5-attempt exponential backoff with jitter and heals stale cached IDs automatically.
pub fn upload_file_multipart(
    access_token: &str,
    parent_id: &str,
    filename: &str,
    data: &[u8],
) -> bool {
    let local_md5 = md5_hex(data);

    let existing_id = match find_drive_file_meta(access_token, parent_id, filename) {
        DriveLookup::Found((meta, from_cache)) => {
            let hash_matches = meta.size == data.len() as u64
                && meta
                    .md5_checksum
                    .as_deref()
                    .map(|m| m.eq_ignore_ascii_case(&local_md5))
                    .unwrap_or(false);

            if hash_matches {
                if !from_cache {
                    // Acabou de ser consultado na API do Drive e já possui exatamente o mesmo conteúdo!
                    return true;
                }
                // Veio do cache em memória: valida com GET leve de metadados que não foi apagado na web
                match verify_cached_file_still_live(access_token, &meta.id) {
                    DriveLookup::Found(live) => {
                        let still_same = live.size == data.len() as u64
                            && live
                                .md5_checksum
                                .as_deref()
                                .map(|m| m.eq_ignore_ascii_case(&local_md5))
                                .unwrap_or(false);
                        cache_file_item(parent_id, live.clone());
                        if still_same {
                            return true;
                        }
                        Some(live.id)
                    }
                    DriveLookup::Missing => {
                        remove_cached_file_item(parent_id, filename);
                        None
                    }
                    DriveLookup::Error => return false,
                }
            } else {
                Some(meta.id)
            }
        }
        DriveLookup::Missing => None,
        // Falha na busca: nunca disparar POST cego para não criar arquivos duplicados
        DriveLookup::Error => return false,
    };

    let use_resumable = data.len() > RESUMABLE_UPLOAD_THRESHOLD;
    let first_try = if use_resumable {
        upload_resumable_large(
            access_token,
            parent_id,
            filename,
            existing_id.as_deref(),
            data,
            &local_md5,
        )
    } else {
        upload_multipart_small(
            access_token,
            parent_id,
            filename,
            existing_id.as_deref(),
            data,
            &local_md5,
        )
    };

    match first_try {
        Some(ok) => ok,
        None if existing_id.is_some() => {
            // Fallback limpo para criação (POST) quando o file_id antigo retornou 404
            if use_resumable {
                upload_resumable_large(access_token, parent_id, filename, None, data, &local_md5)
                    .unwrap_or(false)
            } else {
                upload_multipart_small(access_token, parent_id, filename, None, data, &local_md5)
                    .unwrap_or(false)
            }
        }
        None => false,
    }
}

/// Deletes all non-folder files named `filename` inside `parent_id` on Google Drive.
/// Returns `true` if the file was deleted or already absent, `false` on network/API failure.
pub fn delete_drive_file_in_folder(access_token: &str, parent_id: &str, filename: &str) -> bool {
    remove_cached_file_item(parent_id, filename);
    let q = format!(
        "name='{}' and '{}' in parents and mimeType!='application/vnd.google-apps.folder' and trashed=false",
        escape_drive_query(filename),
        escape_drive_query(parent_id)
    );
    let search_path = format!(
        "/drive/v3/files?q={}&fields=files(id,name)&pageSize=20",
        url_encode(&q)
    );
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let Some(resp) = drive_request_with_retry(
        "GET",
        "www.googleapis.com",
        &search_path,
        &auth_hdr,
        &[],
        20_000,
    ) else {
        return false;
    };
    if resp.status != 200 {
        return false;
    }

    let s = String::from_utf8_lossy(&resp.body);
    let items = parse_drive_file_list(&s);
    for item in items {
        if !delete_drive_file_by_id_best_effort(access_token, &item.id) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_drive_query() {
        assert_eq!(
            escape_drive_query("Assassin's Creed\\save'1.sav"),
            "Assassin\\'s Creed\\\\save\\'1.sav"
        );
    }

    #[test]
    fn test_parse_drive_file_list_handles_braces_and_escapes() {
        let sample = r#"{
            "nextPageToken": "tok_123",
            "files": [
                {
                    "id": "file_1",
                    "name": "slot_{0}_\"main\".sav",
                    "mimeType": "application/octet-stream",
                    "size": "10485760",
                    "md5Checksum": "9e107d9d372bb6826bd81d3542a419d6"
                },
                {
                    "id": "folder_2",
                    "name": "Saves",
                    "mimeType": "application/vnd.google-apps.folder"
                }
            ]
        }"#;

        let items = parse_drive_file_list(sample);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "file_1");
        assert_eq!(items[0].name, "slot_{0}_\"main\".sav");
        assert_eq!(items[0].size, 10_485_760);
        assert_eq!(
            items[0].md5_checksum.as_deref(),
            Some("9e107d9d372bb6826bd81d3542a419d6")
        );
        assert_eq!(items[1].id, "folder_2");
        assert_eq!(items[1].mime_type, "application/vnd.google-apps.folder");
    }
}
