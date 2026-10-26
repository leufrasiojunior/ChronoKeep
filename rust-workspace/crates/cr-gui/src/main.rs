#![windows_subsystem = "windows"]
//! Native Win32 GUI Installer & Configurator (`ChronoKeepSetup.exe`).
//! Installs the Bridge DLL (`chronokeep.dll` + optional host loader compatibility),
//! deploys `chrono-daemon.exe`, configures the Vault path, and provides 1-Click
//! Google Drive OAuth2 Browser Login using the pre-verified Desktop OAuth client.

use cr_proto::cloud_messages::DEFAULT_SERVICE_PORT;
use std::env;
use std::ffi::c_void;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::Duration;

const GDRIVE_CLIENT_ID: &str =
    "1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com";
const GDRIVE_CLIENT_SECRET: &str = "v6V3fKV_zWU7iw1DrpO1rknX";
const GDRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

type HWND = usize;
type HINSTANCE = usize;
type HMENU = usize;
type HFONT = usize;
type HBRUSH = usize;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type HINTERNET = *mut c_void;
type DWORD = u32;
type BOOL = i32;

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_BORDER: u32 = 0x0080_0000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_PUSHBUTTON: u32 = 0x0000;

const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_COMMAND: u32 = 0x0111;
const WM_SETFONT: u32 = 0x0030;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_SETICON: u32 = 0x0080;
const ICON_SMALL: usize = 0;
const ICON_BIG: usize = 1;

const COLOR_WINDOW: i32 = 5;
const DEFAULT_GUI_FONT: i32 = 17;
const SW_SHOWNORMAL: i32 = 1;
const MB_OK: u32 = 0x0000_0000;
const MB_YESNO: u32 = 0x0000_0004;
const MB_ICONINFORMATION: u32 = 0x0000_0040;
const MB_ICONQUESTION: u32 = 0x0000_0020;
const MB_ICONWARNING: u32 = 0x0000_0030;
const MB_ICONERROR: u32 = 0x0000_0010;
const IDYES: i32 = 6;
#[allow(dead_code)]
const IDNO: i32 = 7;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const EMBEDDED_CHRONOKEEP_DLL: &[u8] = include_bytes!("../embedded/chronokeep.dll");
const EMBEDDED_CHRONO_DAEMON: &[u8] = include_bytes!("../embedded/chrono-daemon.exe");

const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: DWORD = 0;
const INTERNET_DEFAULT_HTTPS_PORT: u16 = 443;
const WINHTTP_FLAG_SECURE: DWORD = 0x0080_0000;
const WINHTTP_FLAG_ESCAPE_DISABLE: DWORD = 0x0000_0040;
const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x2000_0000;
const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x2000_0000;

const ID_EDIT_STEAM: usize = 101;
const ID_EDIT_VAULT: usize = 102;
const ID_BTN_INSTALL: usize = 201;
const ID_BTN_START_SVC: usize = 202;
const ID_BTN_OPEN_VAULT: usize = 203;
const ID_BTN_OPEN_LOGS: usize = 204;
const ID_BTN_GOOGLE_LOGIN: usize = 205;
const ID_LBL_STATUS: usize = 301;
const ID_LBL_GDRIVE_STATUS: usize = 302;

#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: HINSTANCE,
    h_icon: usize,
    h_cursor: usize,
    hbr_background: HBRUSH,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: usize,
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    w_param: WPARAM,
    l_param: LPARAM,
    time: u32,
    pt: POINT,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dw_ex_style: u32,
        lp_class_name: *const u16,
        lp_window_name: *const u16,
        dw_style: u32,
        x: i32,
        y: i32,
        n_width: i32,
        n_height: i32,
        h_wnd_parent: HWND,
        h_menu: HMENU,
        h_instance: HINSTANCE,
        lp_param: *mut std::ffi::c_void,
    ) -> HWND;
    fn ShowWindow(h_wnd: HWND, n_cmd_show: i32) -> i32;
    fn UpdateWindow(h_wnd: HWND) -> i32;
    fn GetMessageW(lp_msg: *mut MSG, h_wnd: HWND, w_msg_filter_min: u32, w_msg_filter_max: u32)
        -> i32;
    fn TranslateMessage(lp_msg: *const MSG) -> i32;
    fn DispatchMessageW(lp_msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    fn PostQuitMessage(n_exit_code: i32);
    fn SendMessageW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    fn GetDlgItem(h_dlg: HWND, n_id_dlg_item: i32) -> HWND;
    fn SetWindowTextW(h_wnd: HWND, lp_string: *const u16) -> i32;
    fn GetWindowTextW(h_wnd: HWND, lp_string: *mut u16, n_max_count: i32) -> i32;
    fn MessageBoxW(h_wnd: HWND, lp_text: *const u16, lp_caption: *const u16, u_type: u32) -> i32;
    fn LoadCursorW(h_instance: HINSTANCE, lp_cursor_name: *const u16) -> usize;
    fn LoadIconW(h_instance: HINSTANCE, lp_icon_name: *const u16) -> usize;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(i: i32) -> usize;
    fn SetBkMode(hdc: usize, mode: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(lp_module_name: *const u16) -> HINSTANCE;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: HWND,
        lp_operation: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show_cmd: i32,
    ) -> usize;
}

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
        n_server_port: u16,
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

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex_str, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
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

fn extract_json_str(json: &str, key: &str) -> Option<String> {
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

unsafe fn get_control_text(hwnd_parent: HWND, id: usize) -> String {
    let h_ctrl = GetDlgItem(hwnd_parent, id as i32);
    if h_ctrl == 0 {
        return String::new();
    }
    let mut buf = [0u16; 1024];
    let len = GetWindowTextW(h_ctrl, buf.as_mut_ptr(), buf.len() as i32);
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
        .trim()
        .to_string()
}

unsafe fn set_control_text(hwnd_parent: HWND, id: usize, text: &str) {
    let h_ctrl = GetDlgItem(hwnd_parent, id as i32);
    if h_ctrl != 0 {
        let w = to_wide(text);
        SetWindowTextW(h_ctrl, w.as_ptr());
    }
}

fn detect_default_steam_dir() -> String {
    let candidates = [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
        r"D:\Steam",
        r"E:\Steam",
    ];
    for c in candidates {
        if Path::new(c).join("steam.exe").exists() {
            return c.to_string();
        }
    }
    r"C:\Program Files (x86)\Steam".to_string()
}

fn detect_default_vault_dir() -> String {
    if let Ok(profile) = env::var("USERPROFILE") {
        return format!(r"{}\Saved Games\ChronoKeepVault", profile);
    }
    r"C:\ChronoKeepVault".to_string()
}

fn check_service_online(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(180)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let req = format!(
        "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        port
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 128];
    let Ok(n) = stream.read(&mut buf) else {
        return false;
    };
    String::from_utf8_lossy(&buf[..n]).contains("200 OK")
}

unsafe fn create_child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
    font: HFONT,
) -> HWND {
    let cls_w = to_wide(class);
    let txt_w = to_wide(text);
    let hwnd = CreateWindowExW(
        0,
        cls_w.as_ptr(),
        txt_w.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id,
        0,
        null_mut(),
    );
    if hwnd != 0 && font != 0 {
        SendMessageW(hwnd, WM_SETFONT, font, 1);
    }
    hwnd
}

unsafe fn populate_existing_config(hwnd: HWND) {
    let steam_dir = detect_default_steam_dir();
    let mut vault_dir = detect_default_vault_dir();

    let ini_path = Path::new(&steam_dir).join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                if k.eq_ignore_ascii_case("vault_dir") && !v.is_empty() {
                    vault_dir = v.to_string();
                }
            }
        }
    }

    set_control_text(hwnd, ID_EDIT_STEAM, &steam_dir);
    set_control_text(hwnd, ID_EDIT_VAULT, &vault_dir);

    prompt_enable_cloud_if_needed(hwnd, Path::new(&steam_dir));
    refresh_status_label(hwnd);
}

fn is_gdrive_connected(steam_dir: &str) -> bool {
    let token_path = Path::new(steam_dir).join("chronokeep_gdrive.json");
    if let Ok(content) = fs::read_to_string(token_path) {
        if let Some(rt) = extract_json_str(&content, "refresh_token") {
            return !rt.is_empty();
        }
    }
    false
}

fn resolve_service_port_for_gui(steam_path: &str) -> u16 {
    let port_dat = Path::new(steam_path).join("chrono_port.dat");
    if let Ok(content) = fs::read_to_string(&port_dat) {
        if let Ok(p) = content.trim().parse::<u16>() {
            if p > 0 {
                return p;
            }
        }
    }
    let ini_path = Path::new(steam_path).join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("port") {
                    if let Ok(p) = v.trim().parse::<u16>() {
                        return p;
                    }
                }
            }
        }
    }
    DEFAULT_SERVICE_PORT
}

unsafe fn refresh_status_label(hwnd: HWND) {
    let steam_dir = get_control_text(hwnd, ID_EDIT_STEAM);
    let port = resolve_service_port_for_gui(&steam_dir);

    let dll_exists = Path::new(&steam_dir).join("chronokeep.dll").exists()
        || Path::new(&steam_dir).join("cloud_redirect.dll").exists();
    let svc_online = check_service_online(port);
    let gdrive_ok = is_gdrive_connected(&steam_dir);

    let gdrive_text = if gdrive_ok {
        "3. Conta Google Drive: [CONECTADA — Sincronizacao em nuvem ativa]"
    } else {
        "3. Conta Google Drive: [NAO CONECTADA — Clique abaixo para logar no navegador]"
    };
    set_control_text(hwnd, ID_LBL_GDRIVE_STATUS, gdrive_text);

    let status_text = format!(
        "Status: DLL na Steam: [{}]   |   Daemon (127.0.0.1:{}): [{}]   |   GDrive: [{}]",
        if dll_exists {
            "INSTALADA"
        } else {
            "NAO INSTALADA"
        },
        port,
        if svc_online { "ONLINE" } else { "OFFLINE" },
        if gdrive_ok { "OK" } else { "OFF" }
    );
    set_control_text(hwnd, ID_LBL_STATUS, &status_text);
}

fn find_companion_binary(name: &str) -> Option<PathBuf> {
    if let Ok(cur) = env::current_exe() {
        if let Some(dir) = cur.parent() {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let cwd_cand = PathBuf::from(name);
    if cwd_cand.exists() {
        return Some(cwd_cand);
    }
    None
}

fn ensure_opensteamtool_cloud_enabled(steam_dir: &Path) {
    let toml_path = steam_dir.join("opensteamtool.toml");
    if !toml_path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(&toml_path) else {
        return;
    };

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut in_cloud_section = false;
    let mut cloud_found = false;
    let mut enabled_found = false;

    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_cloud_section = trimmed.eq_ignore_ascii_case("[cloud]");
            if in_cloud_section {
                cloud_found = true;
            }
        } else if in_cloud_section && trimmed.to_ascii_lowercase().starts_with("enabled") {
            *line = "enabled = true".to_string();
            enabled_found = true;
        }
    }

    if !cloud_found {
        lines.push(String::new());
        lines.push("[cloud]".to_string());
        lines.push("enabled = true".to_string());
    } else if !enabled_found {
        if let Some(pos) = lines.iter().position(|l| l.trim().eq_ignore_ascii_case("[cloud]")) {
            lines.insert(pos + 1, "enabled = true".to_string());
        }
    }

    let updated = lines.join("\r\n") + "\r\n";
    let _ = fs::write(&toml_path, updated);
}

fn is_opensteamtool_cloud_disabled(steam_dir: &Path) -> bool {
    let toml_path = steam_dir.join("opensteamtool.toml");
    if !toml_path.exists() {
        return false;
    }
    let Ok(content) = fs::read_to_string(&toml_path) else {
        return false;
    };
    let mut in_cloud = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_cloud = t.eq_ignore_ascii_case("[cloud]");
        } else if in_cloud && t.to_ascii_lowercase().starts_with("enabled") {
            if let Some((_, v)) = t.split_once('=') {
                return !v.trim().eq_ignore_ascii_case("true");
            }
        }
    }
    // Secao cloud ausente ou sem enabled=true
    true
}

unsafe fn prompt_enable_cloud_if_needed(hwnd: HWND, steam_dir: &Path) -> bool {
    let toml_path = steam_dir.join("opensteamtool.toml");
    if !toml_path.exists() {
        return true;
    }
    if is_opensteamtool_cloud_disabled(steam_dir) {
        let msg = to_wide(
            "AVISO IMPORTANTE:\r\n\r\n\
             O recurso [cloud] do OpenSteamTool está DESATIVADO no arquivo 'opensteamtool.toml'.\r\n\r\n\
             Se você não ativar o [cloud], o redirecionamento de saves da Steam pelo ChronoKeep NÃO irá funcionar!\r\n\r\n\
             Deseja que o ChronoKeep ative automaticamente o recurso [cloud] no seu opensteamtool.toml agora?",
        );
        let cap = to_wide("ChronoKeep — Recurso Cloud do OpenSteamTool");
        let resp = MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_YESNO | MB_ICONQUESTION);
        if resp == IDYES {
            ensure_opensteamtool_cloud_enabled(steam_dir);
            let ok_msg = to_wide("Recurso [cloud] ativado com sucesso em opensteamtool.toml!");
            let ok_cap = to_wide("Configuracao Atualizada");
            MessageBoxW(hwnd, ok_msg.as_ptr(), ok_cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
            return true;
        } else {
            let warn = to_wide(
                "Atenção: O recurso [cloud] permaneceu desativado.\r\n\
                 Lembre-se de ativá-lo no opensteamtool.toml para que os saves da nuvem sejam interceptados.",
            );
            let warn_cap = to_wide("Aviso");
            MessageBoxW(hwnd, warn.as_ptr(), warn_cap.as_ptr(), MB_OK | MB_ICONWARNING);
            return false;
        }
    }
    true
}

fn exchange_code_for_tokens(code: &str, redirect_uri: &str) -> Option<String> {
    unsafe {
        let body = format!(
            "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
            url_encode(code),
            url_encode(GDRIVE_CLIENT_ID),
            url_encode(GDRIVE_CLIENT_SECRET),
            url_encode(redirect_uri)
        );
        let body_bytes = body.as_bytes();

        let agent = to_wide("ChronoKeepSetup/1.0");
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

        let host_w = to_wide("oauth2.googleapis.com");
        let h_conn = WinHttpConnect(h_session, host_w.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if h_conn.is_null() {
            WinHttpCloseHandle(h_session);
            return None;
        }

        let verb_w = to_wide("POST");
        let path_w = to_wide("/token");
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

        let hdr_w = to_wide("Content-Type: application/x-www-form-urlencoded");
        WinHttpAddRequestHeaders(
            h_req,
            hdr_w.as_ptr(),
            hdr_w.len().saturating_sub(1) as DWORD,
            WINHTTP_ADDREQ_FLAG_ADD,
        );

        let ok = WinHttpSendRequest(
            h_req,
            null(),
            0,
            body_bytes.as_ptr() as *const c_void,
            body_bytes.len() as DWORD,
            body_bytes.len() as DWORD,
            0,
        );
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

        if status_code != 200 {
            return None;
        }
        let json = String::from_utf8_lossy(&resp_bytes).into_owned();
        if extract_json_str(&json, "refresh_token").is_some() {
            Some(json)
        } else {
            None
        }
    }
}

unsafe fn on_google_browser_login(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let steam_dir = Path::new(&steam_dir_str);
    if !steam_dir.exists() {
        let msg = to_wide("Verifique o caminho da pasta da Steam antes de conectar o Google Drive.");
        let cap = to_wide("ChronoKeep");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
        let msg = to_wide("Falha ao abrir porta local temporaria para o callback OAuth.");
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let redirect_uri = format!("http://localhost:{}/callback", port);

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        url_encode(GDRIVE_CLIENT_ID),
        url_encode(&redirect_uri),
        url_encode(GDRIVE_SCOPE)
    );

    set_control_text(
        hwnd,
        ID_LBL_GDRIVE_STATUS,
        "4. Conta Google Drive: [AGUARDANDO LOGIN NO NAVEGADOR...]",
    );

    let op = to_wide("open");
    let url_w = to_wide(&auth_url);
    ShellExecuteW(hwnd, op.as_ptr(), url_w.as_ptr(), null(), null(), SW_SHOWNORMAL);

    // Wait for browser redirect callback
    let mut auth_code: Option<String> = None;
    for stream in listener.incoming().flatten() {
        let mut s = stream;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0u8; 4096];
        let Ok(n) = s.read(&mut buf) else {
            continue;
        };
        let req_str = String::from_utf8_lossy(&buf[..n]);
        let first_line = req_str.lines().next().unwrap_or("");
        if !first_line.contains("/callback") {
            let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            continue;
        }

        if let Some(q_idx) = first_line.find('?') {
            let query_part = first_line[q_idx + 1..]
                .split_whitespace()
                .next()
                .unwrap_or("");
            for pair in query_part.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    if k == "code" {
                        auth_code = Some(url_decode(v));
                    }
                }
            }
        }

        let html = "<html><head><meta charset='utf-8'><title>ChronoKeep</title></head>\
            <body style='background:#0F172A;color:#F8FAFC;font-family:Segoe UI,sans-serif;text-align:center;padding-top:80px;'>\
            <h1 style='color:#38BDF8;'>ChronoKeep Conectado!</h1>\
            <p>Sua conta do Google Drive foi vinculada com sucesso. Voce ja pode fechar esta aba e voltar ao ChronoKeep.</p>\
            </body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = s.write_all(resp.as_bytes());
        let _ = s.flush();
        break;
    }

    let Some(code) = auth_code else {
        refresh_status_label(hwnd);
        let msg = to_wide("Nenhum codigo de autorizacao recebido do navegador.");
        let cap = to_wide("ChronoKeep");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    };

    if let Some(token_json) = exchange_code_for_tokens(&code, &redirect_uri) {
        let token_path = steam_dir.join("chronokeep_gdrive.json");
        let _ = fs::write(&token_path, token_json);
        refresh_status_label(hwnd);
        let msg = to_wide(
            "Conta Google Drive conectada com sucesso!\r\n\
             Os seus saves agora serao sincronizados automaticamente na pasta 'ChronoKeep' do seu Google Drive.",
        );
        let cap = to_wide("ChronoKeep — Google Drive Conectado");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
    } else {
        refresh_status_label(hwnd);
        let msg = to_wide("Falha ao trocar o codigo OAuth pelo token de acesso.");
        let cap = to_wide("Erro OAuth");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

fn is_process_running(proc_name: &str) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {}", proc_name)])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    if let Ok(output) = out {
        let text = String::from_utf8_lossy(&output.stdout);
        text.to_ascii_lowercase().contains(&proc_name.to_ascii_lowercase())
    } else {
        false
    }
}

fn kill_process_and_wait(proc_name: &str, max_wait_ms: u64) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", proc_name])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < max_wait_ms as u128 {
        if !is_process_running(proc_name) {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn launch_steam(steam_dir: &Path) {
    let steam_exe = steam_dir.join("steam.exe");
    if steam_exe.exists() {
        let mut cmd = std::process::Command::new(&steam_exe);
        cmd.current_dir(steam_dir);
        let _ = cmd.spawn();
    }
}

fn deploy_bytes_safe(data: &[u8], dst: &Path) -> bool {
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::write(dst, data).is_ok() {
        return true;
    }
    let old_path = dst.with_extension("old");
    let _ = fs::remove_file(&old_path);
    let _ = fs::rename(dst, &old_path);
    fs::write(dst, data).is_ok()
}

fn deploy_component(target: &Path, companion_name: &str, embedded: &[u8]) -> bool {
    let bytes = if let Some(local_path) = find_companion_binary(companion_name) {
        fs::read(local_path).unwrap_or_else(|_| embedded.to_vec())
    } else {
        embedded.to_vec()
    };
    deploy_bytes_safe(&bytes, target)
}

fn run_silent_install() -> i32 {
    let steam_dir_str = detect_default_steam_dir();
    let vault_dir_str = detect_default_vault_dir();
    let port = DEFAULT_SERVICE_PORT;
    let steam_dir = Path::new(&steam_dir_str);

    if !steam_dir.exists() {
        return 1;
    }

    let _ = fs::create_dir_all(&vault_dir_str);

    // 1. Grava chronokeep.ini
    let ini_content = format!(
        "# Configuracao do ChronoKeep (Modo Silencioso)\r\n\
         port={}\r\n\
         vault_dir={}\r\n\
         dll_name=chronokeep.dll\r\n",
        port, vault_dir_str
    );
    let ini_path = steam_dir.join("chronokeep.ini");
    let _ = fs::write(&ini_path, ini_content);

    // 2. Encerra Steam e daemons anteriores
    let steam_was_running = is_process_running("steam.exe");
    if steam_was_running {
        kill_process_and_wait("steam.exe", 5000);
    }
    kill_process_and_wait("chrono-daemon.exe", 2500);

    // 3. Extrai binarios embutidos
    deploy_component(&steam_dir.join("chronokeep.dll"), "chronokeep.dll", EMBEDDED_CHRONOKEEP_DLL);
    deploy_component(&steam_dir.join("cloud_redirect.dll"), "cloud_redirect.dll", EMBEDDED_CHRONOKEEP_DLL);
    deploy_component(&steam_dir.join("chrono-daemon.exe"), "chrono-daemon.exe", EMBEDDED_CHRONO_DAEMON);

    // 4. Ativa opensteamtool.toml se existir
    ensure_opensteamtool_cloud_enabled(steam_dir);

    // 5. Inicia o servico
    start_service_from_gui(steam_dir, port);

    // 6. Reabre a Steam se estava rodando
    if steam_was_running {
        std::thread::sleep(Duration::from_millis(600));
        launch_steam(steam_dir);
    }

    0
}

unsafe fn on_install_and_save(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);

    let steam_dir = Path::new(&steam_dir_str);
    if !steam_dir.exists() {
        let msg = to_wide("A pasta da Steam informada nao existe. Verifique o caminho.");
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    let _ = fs::create_dir_all(&vault_dir_str);

    // 1. Grava chronokeep.ini com a porta padrao (o daemon aloca dinamicamente se ocupada)
    let port: u16 = DEFAULT_SERVICE_PORT;
    let ini_content = format!(
        "# Configuracao do ChronoKeep gerada pelo Instalador GUI\r\n\
         port={}\r\n\
         vault_dir={}\r\n\
         dll_name=chronokeep.dll\r\n",
        port, vault_dir_str
    );
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Err(e) = fs::write(&ini_path, ini_content) {
        let msg = to_wide(&format!("Falha ao gravar chronokeep.ini: {}", e));
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    // 2. Verifica se a Steam estava em execucao
    let steam_was_running = is_process_running("steam.exe");
    if steam_was_running {
        kill_process_and_wait("steam.exe", 5000);
    }
    kill_process_and_wait("chrono-daemon.exe", 2500);

    // 3. Implanta os arquivos utilizando os binários embutidos no próprio EXE!
    let mut deployed_files = Vec::new();

    // Sempre implanta chronokeep.dll
    let target_dll = steam_dir.join("chronokeep.dll");
    if deploy_component(&target_dll, "chronokeep.dll", EMBEDDED_CHRONOKEEP_DLL) {
        deployed_files.push("chronokeep.dll".to_string());
    }

    // Sempre implanta cloud_redirect.dll para compatibilidade com OpenSteamTool e HubcapTools
    let target_compat = steam_dir.join("cloud_redirect.dll");
    if deploy_component(&target_compat, "cloud_redirect.dll", EMBEDDED_CHRONOKEEP_DLL) {
        deployed_files.push("cloud_redirect.dll (compatibilidade OpenSteamTool/Hubcap)".to_string());
    }

    let target_svc = steam_dir.join("chrono-daemon.exe");
    if deploy_component(&target_svc, "chrono-daemon.exe", EMBEDDED_CHRONO_DAEMON) {
        deployed_files.push("chrono-daemon.exe".to_string());
    }

    // 4. Verifica se opensteamtool.toml esta presente e pergunta se deseja ativar o [cloud]
    prompt_enable_cloud_if_needed(hwnd, steam_dir);

    // 5. Inicia chrono-daemon.exe
    start_service_from_gui(steam_dir, port);
    refresh_status_label(hwnd);

    // 6. Se a Steam estava aberta antes, reabre-a automaticamente
    if steam_was_running {
        std::thread::sleep(Duration::from_millis(600));
        launch_steam(steam_dir);
    }

    let restart_notice = if steam_was_running {
        "\r\n\r\n[OK] A Steam estava aberta e foi reaberta automaticamente com o ChronoKeep carregado!"
    } else {
        "\r\n\r\nAgora voce ja pode abrir a sua Steam normalmente para jogar!"
    };

    let summary = format!(
        "Instalacao do ChronoKeep concluida com sucesso!\r\n\r\n\
         Pasta da Steam: {}\r\n\
         Pasta de Saves (Vault): {}\r\n\
         Arquivos implantados: {}{}",
        steam_dir_str,
        vault_dir_str,
        deployed_files.join(", "),
        restart_notice
    );
    let msg = to_wide(&summary);
    let cap = to_wide("ChronoKeep — Instalacao Concluida");
    MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
}

fn start_service_from_gui(steam_dir: &Path, port: u16) -> bool {
    if check_service_online(port) {
        return true;
    }
    let svc_exe = steam_dir.join("chrono-daemon.exe");
    let exe_to_run = if svc_exe.exists() {
        Some(svc_exe)
    } else {
        find_companion_binary("chrono-daemon.exe")
    };

    if let Some(exe) = exe_to_run {
        let mut cmd = std::process::Command::new(&exe);
        cmd.current_dir(steam_dir);
        cmd.creation_flags(CREATE_NO_WINDOW);
        if cmd.spawn().is_ok() {
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(60));
                if check_service_online(port) {
                    return true;
                }
            }
        }
    }
    check_service_online(port)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let font = GetStockObject(DEFAULT_GUI_FONT);

            create_child(
                hwnd,
                "STATIC",
                "1. Pasta da Steam (onde a DLL e o servico serao instalados):",
                0,
                20,
                16,
                560,
                20,
                0,
                font,
            );
            create_child(
                hwnd,
                "EDIT",
                "",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                20,
                38,
                560,
                24,
                ID_EDIT_STEAM,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "2. Pasta Local do Cofre de Saves (Cache Local / Vault):",
                0,
                20,
                72,
                560,
                20,
                0,
                font,
            );
            create_child(
                hwnd,
                "EDIT",
                "",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                20,
                94,
                560,
                24,
                ID_EDIT_VAULT,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "3. Conta Google Drive: [Verificando...]",
                0,
                20,
                130,
                560,
                20,
                ID_LBL_GDRIVE_STATUS,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Conectar Conta Google Drive (Login de 1 Clique no Navegador)",
                BS_PUSHBUTTON | WS_TABSTOP,
                20,
                152,
                560,
                34,
                ID_BTN_GOOGLE_LOGIN,
                font,
            );

            create_child(
                hwnd,
                "BUTTON",
                "Instalar na Steam (1-Clique)",
                BS_PUSHBUTTON | WS_TABSTOP,
                20,
                200,
                240,
                34,
                ID_BTN_INSTALL,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Iniciar Servico",
                BS_PUSHBUTTON | WS_TABSTOP,
                270,
                200,
                115,
                34,
                ID_BTN_START_SVC,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Abrir Vault",
                BS_PUSHBUTTON | WS_TABSTOP,
                395,
                200,
                95,
                34,
                ID_BTN_OPEN_VAULT,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Ver Logs",
                BS_PUSHBUTTON | WS_TABSTOP,
                500,
                200,
                80,
                34,
                ID_BTN_OPEN_LOGS,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "Status: Verificando...",
                0,
                20,
                248,
                560,
                22,
                ID_LBL_STATUS,
                font,
            );

            populate_existing_config(hwnd);
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = w_param;
            SetBkMode(hdc, 1); // TRANSPARENT
            GetStockObject(0) as LRESULT // WHITE_BRUSH
        }
        WM_COMMAND => {
            let id = w_param & 0xFFFF;
            match id {
                ID_BTN_GOOGLE_LOGIN => {
                    on_google_browser_login(hwnd);
                }
                ID_BTN_INSTALL => {
                    on_install_and_save(hwnd);
                }
                ID_BTN_START_SVC => {
                    let steam_dir = get_control_text(hwnd, ID_EDIT_STEAM);
                    let port = resolve_service_port_for_gui(&steam_dir);
                    let ok = start_service_from_gui(Path::new(&steam_dir), port);
                    refresh_status_label(hwnd);
                    let actual_port = resolve_service_port_for_gui(&steam_dir);
                    let txt = if ok {
                        format!("Servico chrono-daemon.exe esta ONLINE em 127.0.0.1:{}!", actual_port)
                    } else {
                        "Nao foi possivel iniciar o chrono-daemon.exe. Verifique se ele esta na pasta.".to_string()
                    };
                    let msg_w = to_wide(&txt);
                    let cap_w = to_wide("Status do Servico");
                    MessageBoxW(hwnd, msg_w.as_ptr(), cap_w.as_ptr(), MB_OK | MB_ICONINFORMATION);
                }
                ID_BTN_OPEN_VAULT => {
                    let vault_dir = get_control_text(hwnd, ID_EDIT_VAULT);
                    let _ = fs::create_dir_all(&vault_dir);
                    let op = to_wide("open");
                    let path_w = to_wide(&vault_dir);
                    ShellExecuteW(
                        hwnd,
                        op.as_ptr(),
                        path_w.as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                }
                ID_BTN_OPEN_LOGS => {
                    let steam_dir = get_control_text(hwnd, ID_EDIT_STEAM);
                    let log_file = Path::new(&steam_dir).join("chrono_daemon.log");
                    let target = if log_file.exists() {
                        log_file
                    } else {
                        Path::new(&steam_dir).join("chronokeep_bridge.log")
                    };
                    let op = to_wide("open");
                    let path_w = to_wide(&target.to_string_lossy());
                    ShellExecuteW(
                        hwnd,
                        op.as_ptr(),
                        path_w.as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    for arg in &args[1..] {
        let a = arg.to_lowercase();
        if a == "/s"
            || a == "-s"
            || a == "/silent"
            || a == "--silent"
            || a == "/quiet"
            || a == "-q"
            || a == "/install"
        {
            let exit_code = run_silent_install();
            std::process::exit(exit_code);
        }
    }

    unsafe {
        let h_instance = GetModuleHandleW(null());
        let class_name = to_wide("ChronoKeepSetupWnd");
        let window_title = to_wide("ChronoKeep — Instalador e Configurador Cloud Save");

        let h_app_icon = LoadIconW(h_instance, 1 as *const u16);

        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: wnd_proc,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance,
            h_icon: h_app_icon,
            h_cursor: LoadCursorW(0, 32512 as *const u16), // IDC_ARROW
            hbr_background: (COLOR_WINDOW + 1) as HBRUSH,
            lpsz_menu_name: null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: h_app_icon,
        };

        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
            200,
            160,
            615,
            320,
            0,
            0,
            h_instance,
            null_mut(),
        );

        if hwnd == 0 {
            return;
        }

        if h_app_icon != 0 {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, h_app_icon as LPARAM);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, h_app_icon as LPARAM);
        }

        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
