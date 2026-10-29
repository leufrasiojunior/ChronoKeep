//! Shared Google Drive REST API v3 client and OAuth2 routines.

use crate::http::{extract_json_str, https_request, url_encode};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

pub const DEFAULT_GDRIVE_CLIENT_ID: &str =
    "1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com";
pub const DEFAULT_GDRIVE_CLIENT_SECRET: &str = "v6V3fKV_zWU7iw1DrpO1rknX";
pub const DEFAULT_GDRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

static TOKEN_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static FOLDER_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static FOLDER_CREATE_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
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
    let (status, resp) = https_request(
        "POST",
        "oauth2.googleapis.com",
        "/token",
        &headers,
        body.as_bytes(),
    )?;
    if status != 200 {
        return None;
    }
    let resp_str = String::from_utf8_lossy(&resp);
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
    let (status, resp) = https_request(
        "POST",
        "oauth2.googleapis.com",
        "/token",
        &headers,
        body.as_bytes(),
    )?;
    if status != 200 {
        return None;
    }
    String::from_utf8(resp).ok()
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
    // criem pastas duplicadas no Drive (ex: 241100, 241100(1), etc.)
    let _create_guard = FOLDER_CREATE_MUTEX.lock().ok();

    if let Ok(guard) = FOLDER_CACHE.lock() {
        if let Some(ref map) = *guard {
            if let Some(id) = map.get(&cache_key) {
                return Some(id.clone());
            }
        }
    }

    let parent_clause = match parent_id {
        Some(pid) => format!("'{}' in parents", pid),
        None => "'root' in parents".to_string(),
    };
    let q = format!(
        "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false and {}",
        name.replace('\'', "\\'"),
        parent_clause
    );
    let path = format!("/drive/v3/files?q={}&fields=files(id)&pageSize=1", url_encode(&q));
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    if let Some((200, body)) = https_request("GET", "www.googleapis.com", &path, &auth_hdr, &[]) {
        let s = String::from_utf8_lossy(&body);
        if let Some(id) = extract_json_str(&s, "id") {
            if !id.is_empty() {
                if let Ok(mut guard) = FOLDER_CACHE.lock() {
                    let map = guard.get_or_insert_with(HashMap::new);
                    map.insert(cache_key.clone(), id.clone());
                }
                return Some(id);
            }
        }
    }

    // Cria a pasta caso não exista
    let meta = match parent_id {
        Some(pid) => format!(
            "{{\"name\":\"{}\",\"mimeType\":\"application/vnd.google-apps.folder\",\"parents\":[\"{}\"]}}",
            name, pid
        ),
        None => format!(
            "{{\"name\":\"{}\",\"mimeType\":\"application/vnd.google-apps.folder\"}}",
            name
        ),
    };
    let headers = vec![
        format!("Authorization: Bearer {}", access_token),
        "Content-Type: application/json; charset=UTF-8".to_string(),
    ];
    let (status, body) = https_request(
        "POST",
        "www.googleapis.com",
        "/drive/v3/files?fields=id",
        &headers,
        meta.as_bytes(),
    )?;
    if (200..300).contains(&status) {
        let s = String::from_utf8_lossy(&body);
        if let Some(id) = extract_json_str(&s, "id") {
            if !id.is_empty() {
                if let Ok(mut guard) = FOLDER_CACHE.lock() {
                    let map = guard.get_or_insert_with(HashMap::new);
                    map.insert(cache_key, id.clone());
                }
                return Some(id);
            }
        }
    } else if status == 404 || status == 400 {
        clear_folder_cache();
    }
    None
}

pub fn find_drive_folder(
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

    let parent_clause = match parent_id {
        Some(pid) => format!("'{}' in parents", pid),
        None => "'root' in parents".to_string(),
    };
    let q = format!(
        "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false and {}",
        name.replace('\'', "\\'"),
        parent_clause
    );
    let path = format!("/drive/v3/files?q={}&fields=files(id)&pageSize=1", url_encode(&q));
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    if let Some((200, body)) = https_request("GET", "www.googleapis.com", &path, &auth_hdr, &[]) {
        let s = String::from_utf8_lossy(&body);
        if let Some(id) = extract_json_str(&s, "id") {
            if !id.is_empty() {
                if let Ok(mut guard) = FOLDER_CACHE.lock() {
                    let map = guard.get_or_insert_with(HashMap::new);
                    map.insert(cache_key, id.clone());
                }
                return Some(id);
            }
        }
    }
    None
}

pub fn parse_drive_file_list(json: &str) -> Vec<DriveFileItem> {
    let mut items = Vec::new();
    let Some(files_idx) = json.find("\"files\"") else {
        return items;
    };
    let rest = &json[files_idx + 7..];
    let Some(arr_start) = rest.find('[') else {
        return items;
    };
    let mut slice = &rest[arr_start + 1..];

    while let Some(obj_start) = slice.find('{') {
        let after_start = &slice[obj_start + 1..];
        let Some(obj_end) = after_start.find('}') else {
            break;
        };
        let obj_str = &after_start[..obj_end];
        if let (Some(id), Some(name)) = (
            extract_json_str(obj_str, "id"),
            extract_json_str(obj_str, "name"),
        ) {
            let mime_type = extract_json_str(obj_str, "mimeType").unwrap_or_default();
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
        slice = &after_start[obj_end + 1..];
    }
    items
}

pub fn list_drive_files(access_token: &str, parent_id: &str) -> Vec<DriveFileItem> {
    let q = format!("'{}' in parents and trashed=false", parent_id);
    let path = format!(
        "/drive/v3/files?q={}&fields=files(id,name,mimeType,size,md5Checksum)&pageSize=1000",
        url_encode(&q)
    );
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    if let Some((200, body)) = https_request("GET", "www.googleapis.com", &path, &auth_hdr, &[]) {
        let s = String::from_utf8_lossy(&body);
        return parse_drive_file_list(&s);
    }
    Vec::new()
}

pub fn download_drive_file(access_token: &str, file_id: &str) -> Option<Vec<u8>> {
    let path = format!("/drive/v3/files/{}?alt=media", file_id);
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    if let Some((200, body)) = https_request("GET", "www.googleapis.com", &path, &auth_hdr, &[]) {
        return Some(body);
    }
    None
}

pub fn upload_file_multipart(
    access_token: &str,
    parent_id: &str,
    filename: &str,
    data: &[u8],
) -> bool {
    let q = format!(
        "name='{}' and '{}' in parents and mimeType!='application/vnd.google-apps.folder' and trashed=false",
        filename.replace('\'', "\\'"),
        parent_id
    );
    let search_path = format!("/drive/v3/files?q={}&fields=files(id)&pageSize=1", url_encode(&q));
    let auth_hdr = vec![format!("Authorization: Bearer {}", access_token)];
    let existing_id = https_request("GET", "www.googleapis.com", &search_path, &auth_hdr, &[])
        .and_then(|(st, b)| {
            if st == 200 {
                extract_json_str(&String::from_utf8_lossy(&b), "id")
            } else {
                None
            }
        });

    let boundary = "chronokeep_multipart_boundary_99182";
    let meta_json = if existing_id.is_some() {
        format!("{{\"name\":\"{}\"}}", filename)
    } else {
        format!(
            "{{\"name\":\"{}\",\"parents\":[\"{}\"]}}",
            filename, parent_id
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
        Some(ref id) => (
            "PATCH",
            format!("/upload/drive/v3/files/{}?uploadType=multipart&fields=id", id),
        ),
        None => (
            "POST",
            "/upload/drive/v3/files?uploadType=multipart&fields=id".to_string(),
        ),
    };

    let headers = vec![
        format!("Authorization: Bearer {}", access_token),
        format!("Content-Type: multipart/related; boundary={}", boundary),
    ];

    if let Some((status, _)) = https_request(method, "www.googleapis.com", &upload_path, &headers, &body) {
        return (200..300).contains(&status);
    }
    false
}
