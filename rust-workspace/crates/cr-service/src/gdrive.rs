//! Native Google Drive REST API v3 Client (`winhttp.dll`) for `chrono-daemon.exe`.
//! Uses the built-in verified Desktop OAuth2 client by default (or custom credentials if set)
//! and automatically refreshes the `access_token` from `chronokeep_gdrive.json`.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Instant;

static TOKEN_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static FOLDER_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static FOLDER_CREATE_MUTEX: Mutex<()> = Mutex::new(());
static ACTIVE_PULLS: Mutex<Option<HashSet<(u32, u32)>>> = Mutex::new(None);
static PULL_ALL_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub const DEFAULT_GDRIVE_CLIENT_ID: &str =
    "1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com";
pub const DEFAULT_GDRIVE_CLIENT_SECRET: &str = "v6V3fKV_zWU7iw1DrpO1rknX";

type HINTERNET = *mut c_void;
type DWORD = u32;
type BOOL = i32;
#[allow(non_camel_case_types)]
type INTERNET_PORT = u16;

const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: DWORD = 0;
const INTERNET_DEFAULT_HTTPS_PORT: INTERNET_PORT = 443;
const WINHTTP_FLAG_SECURE: DWORD = 0x0080_0000;
const WINHTTP_FLAG_ESCAPE_DISABLE: DWORD = 0x0000_0040;
const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x2000_0000;
const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x2000_0000;

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(
        psz_agent_w: *const u16,
        dw_access_type: DWORD,
        psz_proxy_w: *const u16,
        psz_proxy_bypass_w: *const u16,
        dw_flags: DWORD,
    ) -> HINTERNET;
    fn WinHttpConnect(
        h_session: HINTERNET,
        pswz_server_name: *const u16,
        n_server_port: INTERNET_PORT,
        dw_reserved: DWORD,
    ) -> HINTERNET;
    fn WinHttpOpenRequest(
        h_connect: HINTERNET,
        pwsz_verb: *const u16,
        pwsz_object_name: *const u16,
        pwsz_version: *const u16,
        pwsz_referrer: *const u16,
        ppwsz_accept_types: *const *const u16,
        dw_flags: DWORD,
    ) -> HINTERNET;
    fn WinHttpAddRequestHeaders(
        h_request: HINTERNET,
        lpsz_headers: *const u16,
        dw_headers_length: DWORD,
        dw_modifiers: DWORD,
    ) -> BOOL;
    fn WinHttpSendRequest(
        h_request: HINTERNET,
        lpsz_headers: *const u16,
        dw_headers_length: DWORD,
        lp_optional: *const c_void,
        dw_optional_length: DWORD,
        dw_total_length: DWORD,
        dw_context: usize,
    ) -> BOOL;
    fn WinHttpReceiveResponse(h_request: HINTERNET, lp_reserved: *mut c_void) -> BOOL;
    fn WinHttpQueryHeaders(
        h_request: HINTERNET,
        dw_info_level: DWORD,
        pwsz_name: *const u16,
        lp_buffer: *mut c_void,
        lpdw_buffer_length: *mut DWORD,
        lpdw_index: *mut DWORD,
    ) -> BOOL;
    fn WinHttpQueryDataAvailable(h_request: HINTERNET, lpdw_number_of_bytes_available: *mut DWORD)
        -> BOOL;
    fn WinHttpReadData(
        h_request: HINTERNET,
        lp_buffer: *mut c_void,
        dw_number_of_bytes_to_read: DWORD,
        lpdw_number_of_bytes_read: *mut DWORD,
    ) -> BOOL;
    fn WinHttpCloseHandle(h_internet: HINTERNET) -> BOOL;
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn https_request(
    method: &str,
    host: &str,
    path: &str,
    headers: &[String],
    body: &[u8],
) -> Option<(u32, Vec<u8>)> {
    unsafe {
        let agent = to_wide("ChronoKeep/1.0");
        let h_session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            null(),
            null(),
            0,
        );
        if h_session.is_null() {
            return None;
        }

        let host_w = to_wide(host);
        let h_conn = WinHttpConnect(h_session, host_w.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if h_conn.is_null() {
            WinHttpCloseHandle(h_session);
            return None;
        }

        let verb_w = to_wide(method);
        let path_w = to_wide(path);
        let h_req = WinHttpOpenRequest(
            h_conn,
            verb_w.as_ptr(),
            path_w.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE | WINHTTP_FLAG_ESCAPE_DISABLE,
        );
        if h_req.is_null() {
            WinHttpCloseHandle(h_conn);
            WinHttpCloseHandle(h_session);
            return None;
        }

        for hdr in headers {
            let hdr_w = to_wide(hdr);
            let len = hdr_w.len().saturating_sub(1) as DWORD;
            WinHttpAddRequestHeaders(h_req, hdr_w.as_ptr(), len, WINHTTP_ADDREQ_FLAG_ADD);
        }

        let body_ptr = if body.is_empty() {
            null()
        } else {
            body.as_ptr() as *const c_void
        };
        let body_len = body.len() as DWORD;

        let ok = WinHttpSendRequest(h_req, null(), 0, body_ptr, body_len, body_len, 0);
        if ok == 0 || WinHttpReceiveResponse(h_req, null_mut()) == 0 {
            WinHttpCloseHandle(h_req);
            WinHttpCloseHandle(h_conn);
            WinHttpCloseHandle(h_session);
            return None;
        }

        let mut status_code: DWORD = 0;
        let mut status_len: DWORD = std::mem::size_of::<DWORD>() as DWORD;
        WinHttpQueryHeaders(
            h_req,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status_code as *mut _ as *mut c_void,
            &mut status_len,
            null_mut(),
        );

        let mut resp_bytes = Vec::new();
        loop {
            let mut avail: DWORD = 0;
            if WinHttpQueryDataAvailable(h_req, &mut avail) == 0 || avail == 0 {
                break;
            }
            let mut chunk = vec![0u8; avail as usize];
            let mut read: DWORD = 0;
            if WinHttpReadData(
                h_req,
                chunk.as_mut_ptr() as *mut c_void,
                avail,
                &mut read,
            ) == 0
                || read == 0
            {
                break;
            }
            resp_bytes.extend_from_slice(&chunk[..read as usize]);
        }

        WinHttpCloseHandle(h_req);
        WinHttpCloseHandle(h_conn);
        WinHttpCloseHandle(h_session);
        Some((status_code, resp_bytes))
    }
}

pub fn extract_json_str(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let idx = json.find(&pattern)?;
    let after_key = &json[idx + pattern.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let content = &after_colon[1..];
    let mut out = String::new();
    let mut chars = content.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(esc) = chars.next() {
                out.push(esc);
            }
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
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

pub fn clear_folder_cache() {
    if let Ok(mut guard) = FOLDER_CACHE.lock() {
        *guard = None;
    }
}

fn find_or_create_drive_folder(access_token: &str, name: &str, parent_id: Option<&str>) -> Option<String> {
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

    // Create folder
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
        // Parent folder may have been renamed/deleted externally. Invalidate cache.
        clear_folder_cache();
    }
    None
}

/// Asynchronous-safe helper to upload a save file into `ChronoKeep/<account_id>/<app_id>/<rel_path>` on Google Drive.
pub fn sync_file_to_gdrive(
    exe_dir: &Path,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
    log_path: Option<&PathBuf>,
) -> bool {
    if sync_file_to_gdrive_attempt(exe_dir, account_id, app_id, rel_filename, data, log_path) {
        return true;
    }
    // Auto-recuperação: limpa o cache de pastas e retenta recriando a hierarquia no Drive
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
    let mut token_path = exe_dir.join("chronokeep_gdrive.json");
    if !token_path.exists() {
        if let Ok(cur) = std::env::current_exe() {
            if let Some(parent) = cur.parent() {
                let candidate = parent.join("chronokeep_gdrive.json");
                if candidate.exists() {
                    token_path = candidate;
                }
            }
        }
    }

    if !token_path.exists() {
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
    }

    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive Upload] Erro: Falha ao obter access_token valido com refresh_token.",
            );
        }
        return false;
    };

    let Some(root_id) = find_or_create_drive_folder(&token, "ChronoKeep", None) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive Upload] Erro: Falha ao encontrar ou criar pasta raiz 'ChronoKeep'.",
            );
        }
        return false;
    };

    let Some(acct_id) =
        find_or_create_drive_folder(&token, &account_id.to_string(), Some(&root_id))
    else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive Upload] Erro: Falha ao encontrar ou criar pasta da conta {}",
                    account_id
                ),
            );
        }
        return false;
    };

    let Some(mut parent_id) =
        find_or_create_drive_folder(&token, &app_id.to_string(), Some(&acct_id))
    else {
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

    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return false;
    }
    for sub in &parts[..parts.len() - 1] {
        let Some(next_id) = find_or_create_drive_folder(&token, sub, Some(&parent_id)) else {
            return false;
        };
        parent_id = next_id;
    }
    let leaf_name = parts[parts.len() - 1];

    // Check if file already exists in `parent_id`
    let q = format!(
        "name='{}' and '{}' in parents and mimeType!='application/vnd.google-apps.folder' and trashed=false",
        leaf_name.replace('\'', "\\'"),
        parent_id
    );
    let search_path = format!("/drive/v3/files?q={}&fields=files(id)&pageSize=1", url_encode(&q));
    let auth_hdr = vec![format!("Authorization: Bearer {}", token)];
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
        format!("{{\"name\":\"{}\"}}", leaf_name)
    } else {
        format!(
            "{{\"name\":\"{}\",\"parents\":[\"{}\"]}}",
            leaf_name, parent_id
        )
    };

    let mut body = Vec::with_capacity(meta_json.len() + data.len() + 256);
    body.extend_from_slice(format!("--{}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{}\r\nContent-Type: application/octet-stream\r\n\r\n", boundary, meta_json, boundary).as_bytes());
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
        format!("Authorization: Bearer {}", token),
        format!("Content-Type: multipart/related; boundary={}", boundary),
    ];
    if let Some((status, _)) = https_request(method, "www.googleapis.com", &upload_path, &headers, &body) {
        let ok = (200..300).contains(&status);
        if let Some(lp) = log_path {
            if ok {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive Upload] SUCESSO: app={} file='{}' ({}B) enviado via HTTP {}",
                        app_id, rel_filename, data.len(), status
                    ),
                );
            } else {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive Upload] ERRO HTTP {}: Falha ao enviar app={} file='{}'",
                        status, app_id, rel_filename
                    ),
                );
            }
        }
        return ok;
    }

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive Upload] Erro de rede/conexao HTTPS ao enviar app={} file='{}'",
                app_id, rel_filename
            ),
        );
    }
    false
}

#[derive(Debug, Clone)]
pub struct DriveFileItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
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
            items.push(DriveFileItem {
                id,
                name,
                mime_type,
                size,
            });
        }
        slice = &after_start[obj_end + 1..];
    }
    items
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

pub fn list_drive_files(access_token: &str, parent_id: &str) -> Vec<DriveFileItem> {
    let q = format!("'{}' in parents and trashed=false", parent_id);
    let path = format!(
        "/drive/v3/files?q={}&fields=files(id,name,mimeType,size)&pageSize=1000",
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

fn pull_drive_folder_recursive(
    access_token: &str,
    folder_id: &str,
    rel_prefix: &str,
    account_id: u32,
    app_id: u32,
    vault: &crate::vault::VaultStorage,
    log_path: Option<&PathBuf>,
) -> usize {
    let items = list_drive_files(access_token, folder_id);
    let mut downloaded_count = 0;

    for item in items {
        if item.mime_type == "application/vnd.google-apps.folder" {
            let next_prefix = if rel_prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{}/{}", rel_prefix, item.name)
            };
            downloaded_count += pull_drive_folder_recursive(
                access_token,
                &item.id,
                &next_prefix,
                account_id,
                app_id,
                vault,
                log_path,
            );
        } else {
            let rel_file = if rel_prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{}/{}", rel_prefix, item.name)
            };

            // Se for manifest.tsv, só adota se a nuvem tiver um CN superior ao local
            if rel_file == "manifest.tsv" {
                if let Some(bytes) = download_drive_file(access_token, &item.id) {
                    let cloud_str = String::from_utf8_lossy(&bytes);
                    let mut cloud_cn = 0u64;
                    for l in cloud_str.lines() {
                        if let Some((k, v)) = l.split_once('\t') {
                            if k == "cn" || k == "META" {
                                let val_str = if k == "META" {
                                    v.split('\t').next().unwrap_or("0")
                                } else {
                                    v
                                };
                                if let Ok(cn) = val_str.parse::<u64>() {
                                    cloud_cn = cn;
                                    break;
                                }
                            }
                        }
                    }
                    let current_manifest = vault.load_and_reconcile_manifest(account_id, app_id);
                    if cloud_cn > current_manifest.change_number {
                        let m_path = vault.manifest_path(account_id, app_id);
                        if let Some(parent) = m_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&m_path, bytes);
                    }
                }
                continue;
            }

            // Verifica se o arquivo local já existe com o mesmo tamanho (no Vault ou em userdata)
            let user_dest_matches = vault
                .steam_path
                .as_ref()
                .map(|sp| {
                    sp.join("userdata")
                        .join(account_id.to_string())
                        .join(app_id.to_string())
                        .join("remote")
                        .join(&rel_file)
                })
                .and_then(|p| fs::metadata(&p).ok())
                .map(|m| m.len() == item.size && item.size > 0)
                .unwrap_or(false);

            let local_exists_and_matches = user_dest_matches
                || vault
                    .resolve_file_disk_path(account_id, app_id, &rel_file)
                    .and_then(|p| fs::metadata(&p).ok())
                    .map(|m| m.len() == item.size && item.size > 0)
                    .unwrap_or(false);

            if !local_exists_and_matches {
                if let Some(bytes) = download_drive_file(access_token, &item.id) {
                    if vault
                        .write_save_file(account_id, app_id, &rel_file, &bytes)
                        .is_ok()
                    {
                        downloaded_count += 1;
                        if let Some(ref sp) = vault.steam_path {
                            let user_dest = sp
                                .join("userdata")
                                .join(account_id.to_string())
                                .join(app_id.to_string())
                                .join("remote")
                                .join(&rel_file);
                            let needs_write = match fs::metadata(&user_dest) {
                                Ok(m) => m.len() != bytes.len() as u64,
                                Err(_) => true,
                            };
                            if needs_write {
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
                    }
                }
            }
        }
    }

    downloaded_count
}

pub fn pull_app_from_gdrive(
    steam_dir: &Path,
    account_id: u32,
    app_id: u32,
    vault: &crate::vault::VaultStorage,
    log_path: Option<&PathBuf>,
) -> bool {
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

    let token_path = steam_dir.join("chronokeep_gdrive.json");
    if !token_path.exists() {
        return false;
    }
    let Some(token) = get_valid_access_token(&token_path) else {
        if let Some(lp) = log_path {
            crate::log_msg(
                lp,
                "[Google Drive] Falha ao renovar access_token com refresh_token.",
            );
        }
        return false;
    };

    let Some(root_id) = find_drive_folder(&token, "ChronoKeep", None) else {
        return false;
    };
    let Some(acct_id) = find_drive_folder(&token, &account_id.to_string(), Some(&root_id)) else {
        return false;
    };
    let Some(app_folder_id) = find_drive_folder(&token, &app_id.to_string(), Some(&acct_id)) else {
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

    let count = pull_drive_folder_recursive(
        &token,
        &app_folder_id,
        "",
        account_id,
        app_id,
        vault,
        log_path,
    );

    // Reconcilia o manifest local após baixar os arquivos da nuvem
    let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
    let _ = vault.save_manifest(account_id, app_id, &manifest);

    if let Some(lp) = log_path {
        crate::log_msg(
            lp,
            &format!(
                "[Google Drive] Sincronizacao concluida: {} arquivos novos/atualizados para app={}",
                count, app_id
            ),
        );
    }

    true
}

pub fn start_background_pull_all(
    steam_dir: PathBuf,
    account_id: u32,
    _apps: Vec<u32>,
    vault: crate::vault::VaultStorage,
    log_path: Option<PathBuf>,
) {
    if PULL_ALL_ACTIVE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        struct PullAllGuard;
        impl Drop for PullAllGuard {
            fn drop(&mut self) {
                PULL_ALL_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _all_guard = PullAllGuard;
        let mut token_path = steam_dir.join("chronokeep_gdrive.json");
        if !token_path.exists() {
            if let Ok(cur) = std::env::current_exe() {
                if let Some(parent) = cur.parent() {
                    let candidate = parent.join("chronokeep_gdrive.json");
                    if candidate.exists() {
                        token_path = candidate;
                    }
                }
            }
        }
        if !token_path.exists() {
            return;
        }

        let Some(token) = get_valid_access_token(&token_path) else {
            if let Some(ref lp) = log_path {
                crate::log_msg(lp, "[Google Drive] Falha ao renovar token de acesso no pull em background.");
            }
            return;
        };

        let Some(root_id) = find_drive_folder(&token, "ChronoKeep", None) else {
            return;
        };
        let Some(acct_id) = find_drive_folder(&token, &account_id.to_string(), Some(&root_id)) else {
            return;
        };

        if let Some(ref lp) = log_path {
            crate::log_msg(
                lp,
                &format!(
                    "[Google Drive] Descobrindo jogos com saves sincronizados no Drive para account_id={}",
                    account_id
                ),
            );
        }

        // Lista as pastas de jogos diretamente no Drive em uma única consulta rápida
        let app_folders = list_drive_files(&token, &acct_id);
        for item in app_folders {
            if item.mime_type != "application/vnd.google-apps.folder" {
                continue;
            }
            let Ok(app_id) = item.name.parse::<u32>() else {
                continue;
            };
            if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
                continue;
            }

            // Armazena no cache de pastas para futuras chamadas
            if let Ok(mut guard) = FOLDER_CACHE.lock() {
                let map = guard.get_or_insert_with(HashMap::new);
                map.insert(format!("{}:{}", acct_id, item.name), item.id.clone());
            }

            if let Some(ref lp) = log_path {
                crate::log_msg(
                    lp,
                    &format!(
                        "[Google Drive] Sincronizando saves da nuvem para app={}",
                        app_id
                    ),
                );
            }

            let count = pull_drive_folder_recursive(
                &token,
                &item.id,
                "",
                account_id,
                app_id,
                &vault,
                log_path.as_ref(),
            );

            let manifest = vault.load_and_reconcile_manifest(account_id, app_id);
            let _ = vault.save_manifest(account_id, app_id, &manifest);

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
    });
}
