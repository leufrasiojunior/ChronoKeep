//! WinHTTP-based secure HTTPS client and text/URL/JSON utilities.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

pub type DWORD = u32;
pub type BOOL = i32;
#[allow(non_camel_case_types)]
pub type INTERNET_PORT = u16;
pub type HINTERNET = *mut c_void;

pub const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: DWORD = 0;
pub const INTERNET_DEFAULT_HTTPS_PORT: INTERNET_PORT = 443;
pub const WINHTTP_FLAG_SECURE: DWORD = 0x0080_0000;
pub const WINHTTP_FLAG_ESCAPE_DISABLE: DWORD = 0x0000_0040;
pub const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x2000_0000;
pub const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
pub const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x2000_0000;

#[cfg(target_os = "windows")]
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
    fn WinHttpQueryDataAvailable(
        h_request: HINTERNET,
        lpdw_number_of_bytes_available: *mut DWORD,
    ) -> BOOL;
    fn WinHttpReadData(
        h_request: HINTERNET,
        lp_buffer: *mut c_void,
        dw_number_of_bytes_to_read: DWORD,
        lpdw_number_of_bytes_read: *mut DWORD,
    ) -> BOOL;
    fn WinHttpSetTimeouts(
        h_internet: HINTERNET,
        n_resolve_timeout: i32,
        n_connect_timeout: i32,
        n_send_timeout: i32,
        n_receive_timeout: i32,
    ) -> BOOL;
    fn WinHttpCloseHandle(h_internet: HINTERNET) -> BOOL;
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn url_encode(s: &str) -> String {
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

pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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

#[cfg(target_os = "windows")]
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
        WinHttpSetTimeouts(h_session, 3000, 3000, 5000, 5000);

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

#[cfg(not(target_os = "windows"))]
pub fn https_request(
    _method: &str,
    _host: &str,
    _path: &str,
    _headers: &[String],
    _body: &[u8],
) -> Option<(u32, Vec<u8>)> {
    None
}
