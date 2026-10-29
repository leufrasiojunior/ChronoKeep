#![windows_subsystem = "windows"]
//! Native Win32 GUI Installer & Configurator (`ChronoKeepSetup.exe`).
//! Installs the Bridge DLL (`chronokeep.dll` + optional host loader compatibility),
//! deploys `chrono-daemon.exe`, configures the Vault path, and provides 1-Click
//! Google Drive OAuth2 Browser Login using the pre-verified Desktop OAuth client.

use cr_common::config::{detect_default_steam_dir, DEFAULT_SERVICE_PORT};
use cr_common::fs_utils::{collect_files_recursive, format_size};
use cr_common::gdrive::{
    clear_folder_cache, exchange_code_for_tokens, find_or_create_drive_folder,
    refresh_access_token, upload_file_multipart, DEFAULT_GDRIVE_CLIENT_ID as GDRIVE_CLIENT_ID,
    DEFAULT_GDRIVE_SCOPE as GDRIVE_SCOPE,
};
use cr_common::http::{to_wide, url_decode, url_encode};
use cr_common::manifest::{parse_manifest, serialize_manifest, ManifestData, ManifestFileRecord};
use cr_proto::cloud_messages::split_root_token;
use cr_proto::sha1::sha1_digest;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Duration;

type HWND = usize;
type HINSTANCE = usize;
type HMENU = usize;
type HFONT = usize;
type HBRUSH = usize;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_PUSHBUTTON: u32 = 0x0000;

const LBS_NOTIFY: u32 = 0x0001;
const LBS_MULTIPLESEL: u32 = 0x0008;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_SETSEL: u32 = 0x0185;
const LB_GETSELCOUNT: u32 = 0x0190;
const LB_GETSELITEMS: u32 = 0x0191;

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;

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

const ID_EDIT_STEAM: usize = 101;
const ID_EDIT_VAULT: usize = 102;
const ID_LBL_STEAM: usize = 103;
const ID_LBL_VAULT: usize = 104;
const ID_BTN_INSTALL: usize = 201;
const ID_BTN_START_SVC: usize = 202;
const ID_BTN_OPEN_VAULT: usize = 203;
const ID_BTN_OPEN_LOGS: usize = 204;
const ID_BTN_GOOGLE_LOGIN: usize = 205;
const ID_LBL_STATUS: usize = 301;
const ID_LBL_GDRIVE_STATUS: usize = 302;

// Abas de navegacao
const ID_TAB_BTN_CONFIG: usize = 401;
const ID_TAB_BTN_ORPHANS: usize = 402;

// Controles da Aba 2: Saves Orfaos / Nuvem
const ID_LBL_ORPHANS_TITLE: usize = 501;
const ID_LIST_ORPHANS: usize = 502;
const ID_BTN_SCAN_ORPHANS: usize = 503;
const ID_BTN_SELECT_ALL: usize = 504;
const ID_BTN_DESELECT_ALL: usize = 505;
const ID_BTN_UPLOAD_ORPHANS: usize = 506;
const ID_LBL_ORPHANS_STATUS: usize = 507;

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

fn detect_default_vault_dir() -> String {
    cr_common::config::detect_default_vault_dir(None)
        .to_string_lossy()
        .into_owned()
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

fn upload_file_attempt_gui(
    token: &str,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
) -> bool {
    let Some(root_id) = find_or_create_drive_folder(token, "ChronoKeep", None) else {
        return false;
    };
    let Some(acct_id) = find_or_create_drive_folder(token, &account_id.to_string(), Some(&root_id)) else {
        return false;
    };
    let Some(mut parent_id) = find_or_create_drive_folder(token, &app_id.to_string(), Some(&acct_id)) else {
        return false;
    };

    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return false;
    }
    for sub in &parts[..parts.len() - 1] {
        let Some(next_id) = find_or_create_drive_folder(token, sub, Some(&parent_id)) else {
            return false;
        };
        parent_id = next_id;
    }
    let leaf_name = parts[parts.len() - 1];

    upload_file_multipart(token, &parent_id, leaf_name, data)
}

fn sync_orphan_file_to_drive_with_retry(
    token: &str,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
) -> bool {
    if upload_file_attempt_gui(token, account_id, app_id, rel_filename, data) {
        return true;
    }
    clear_folder_cache();
    upload_file_attempt_gui(token, account_id, app_id, rel_filename, data)
}

#[derive(Clone, Debug)]
struct OrphanGameItem {
    account_id: u32,
    app_id: u32,
    file_count: usize,
    total_bytes: u64,
    source: String,
    files: Vec<(String, PathBuf)>,
}

static SCANNED_ORPHANS: Mutex<Vec<OrphanGameItem>> = Mutex::new(Vec::new());

const TAB_1_CONTROLS: &[usize] = &[
    ID_LBL_STEAM,
    ID_EDIT_STEAM,
    ID_LBL_VAULT,
    ID_EDIT_VAULT,
    ID_LBL_GDRIVE_STATUS,
    ID_BTN_GOOGLE_LOGIN,
    ID_BTN_INSTALL,
    ID_BTN_START_SVC,
    ID_BTN_OPEN_VAULT,
    ID_BTN_OPEN_LOGS,
    ID_LBL_STATUS,
];

const TAB_2_CONTROLS: &[usize] = &[
    ID_LBL_ORPHANS_TITLE,
    ID_LIST_ORPHANS,
    ID_BTN_SCAN_ORPHANS,
    ID_BTN_SELECT_ALL,
    ID_BTN_DESELECT_ALL,
    ID_BTN_UPLOAD_ORPHANS,
    ID_LBL_ORPHANS_STATUS,
];

unsafe fn switch_tab(hwnd: HWND, tab_idx: usize) {
    let show_tab1 = if tab_idx == 0 { SW_SHOW } else { SW_HIDE };
    let show_tab2 = if tab_idx == 1 { SW_SHOW } else { SW_HIDE };

    for &id in TAB_1_CONTROLS {
        let h = GetDlgItem(hwnd, id as i32);
        if h != 0 {
            ShowWindow(h, show_tab1);
        }
    }
    for &id in TAB_2_CONTROLS {
        let h = GetDlgItem(hwnd, id as i32);
        if h != 0 {
            ShowWindow(h, show_tab2);
        }
    }
}

unsafe fn on_scan_orphans(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);
    let steam_dir = Path::new(&steam_dir_str);
    let vault_dir = Path::new(&vault_dir_str);

    let h_status = GetDlgItem(hwnd, ID_LBL_ORPHANS_STATUS as i32);
    if h_status != 0 {
        SetWindowTextW(h_status, to_wide("Escaneando pastas de saves locais no PC...").as_ptr());
    }

    let mut map: BTreeMap<(u32, u32), OrphanGameItem> = BTreeMap::new();

    // 1. Escanear Steam userdata
    let userdata_dir = steam_dir.join("userdata");
    if userdata_dir.exists() {
        if let Ok(acct_entries) = fs::read_dir(&userdata_dir) {
            for acct_entry in acct_entries.flatten() {
                let acct_path = acct_entry.path();
                if !acct_path.is_dir() {
                    continue;
                }
                let Some(acct_name) = acct_path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Ok(acct_id) = acct_name.parse::<u32>() else {
                    continue;
                };
                if acct_id == 0 {
                    continue;
                }

                if let Ok(app_entries) = fs::read_dir(&acct_path) {
                    for app_entry in app_entries.flatten() {
                        let app_path = app_entry.path();
                        if !app_path.is_dir() {
                            continue;
                        }
                        let Some(app_name) = app_path.file_name().and_then(|n| n.to_str()) else {
                            continue;
                        };
                        let Ok(app_id) = app_name.parse::<u32>() else {
                            continue;
                        };
                        if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
                            continue;
                        }

                        let remote_dir = app_path.join("remote");
                        if !remote_dir.exists() {
                            continue;
                        }

                        let mut found = Vec::new();
                        collect_files_recursive(&remote_dir, &remote_dir, &mut found);
                        let valid_files: Vec<(String, PathBuf)> = found
                            .into_iter()
                            .filter(|(rel, _)| {
                                let l = rel.to_lowercase();
                                !l.starts_with('.') && !l.ends_with(".tmp") && !l.contains(".cloudredirect")
                            })
                            .collect();

                        if !valid_files.is_empty() {
                            let entry = map.entry((acct_id, app_id)).or_insert_with(|| OrphanGameItem {
                                account_id: acct_id,
                                app_id,
                                file_count: 0,
                                total_bytes: 0,
                                source: "userdata".to_string(),
                                files: Vec::new(),
                            });
                            for (rel, path) in valid_files {
                                if !entry.files.iter().any(|(r, _)| r == &rel) {
                                    if let Ok(meta) = fs::metadata(&path) {
                                        entry.total_bytes += meta.len();
                                    }
                                    entry.files.push((rel, path));
                                }
                            }
                            entry.file_count = entry.files.len();
                        }
                    }
                }
            }
        }
    }

    // 2. Escanear Vault local
    if vault_dir.exists() {
        if let Ok(acct_entries) = fs::read_dir(vault_dir) {
            for acct_entry in acct_entries.flatten() {
                let acct_path = acct_entry.path();
                if !acct_path.is_dir() {
                    continue;
                }
                let Some(acct_name) = acct_path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Ok(acct_id) = acct_name.parse::<u32>() else {
                    continue;
                };

                if let Ok(app_entries) = fs::read_dir(&acct_path) {
                    for app_entry in app_entries.flatten() {
                        let app_path = app_entry.path();
                        if !app_path.is_dir() {
                            continue;
                        }
                        let Some(app_name) = app_path.file_name().and_then(|n| n.to_str()) else {
                            continue;
                        };
                        let Ok(app_id) = app_name.parse::<u32>() else {
                            continue;
                        };
                        if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
                            continue;
                        }

                        let files_dir = app_path.join("files");
                        if !files_dir.exists() {
                            continue;
                        }

                        let mut found = Vec::new();
                        collect_files_recursive(&files_dir, &files_dir, &mut found);
                        let valid_files: Vec<(String, PathBuf)> = found
                            .into_iter()
                            .filter(|(rel, _)| {
                                let l = rel.to_lowercase();
                                !l.starts_with('.') && !l.ends_with(".tmp") && !l.contains(".cloudredirect") && l != "manifest.tsv"
                            })
                            .collect();

                        if !valid_files.is_empty() {
                            let entry = map.entry((acct_id, app_id)).or_insert_with(|| OrphanGameItem {
                                account_id: acct_id,
                                app_id,
                                file_count: 0,
                                total_bytes: 0,
                                source: "Vault".to_string(),
                                files: Vec::new(),
                            });
                            for (rel, path) in valid_files {
                                if !entry.files.iter().any(|(r, _)| r == &rel) {
                                    if let Ok(meta) = fs::metadata(&path) {
                                        entry.total_bytes += meta.len();
                                    }
                                    entry.files.push((rel, path));
                                }
                            }
                            entry.file_count = entry.files.len();
                        }
                    }
                }
            }
        }
    }

    let items: Vec<OrphanGameItem> = map.into_values().collect();
    let count = items.len();
    if let Ok(mut guard) = SCANNED_ORPHANS.lock() {
        *guard = items.clone();
    }

    let h_list = GetDlgItem(hwnd, ID_LIST_ORPHANS as i32);
    if h_list != 0 {
        SendMessageW(h_list, LB_RESETCONTENT, 0, 0);
        for item in &items {
            let line = format!(
                "AppID: {:<8} | {:>2} arquivo(s) ({:>8}) | Conta: {:<9} | Local: {}",
                item.app_id,
                item.file_count,
                format_size(item.total_bytes),
                item.account_id,
                item.source
            );
            SendMessageW(h_list, LB_ADDSTRING, 0, to_wide(&line).as_ptr() as LPARAM);
        }
        SendMessageW(h_list, LB_SETSEL, 1, -1);
    }

    if h_status != 0 {
        let msg = format!(
            "{} jogos com saves locais encontrados no PC. Todos selecionados para envio.",
            count
        );
        SetWindowTextW(h_status, to_wide(&msg).as_ptr());
    }
}

unsafe fn on_select_all(hwnd: HWND, select: bool) {
    let h_list = GetDlgItem(hwnd, ID_LIST_ORPHANS as i32);
    if h_list != 0 {
        SendMessageW(h_list, LB_SETSEL, if select { 1 } else { 0 }, -1);
    }
    let h_status = GetDlgItem(hwnd, ID_LBL_ORPHANS_STATUS as i32);
    if h_status != 0 {
        let count = SCANNED_ORPHANS.lock().map(|g| g.len()).unwrap_or(0);
        let msg = if select {
            format!("Todos os {} jogos foram marcados.", count)
        } else {
            "Nenhum jogo selecionado. Clique na lista para selecionar ou em 'Marcar Todos'.".to_string()
        };
        SetWindowTextW(h_status, to_wide(&msg).as_ptr());
    }
}

unsafe fn on_upload_orphans(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);
    let steam_dir = PathBuf::from(&steam_dir_str);
    let vault_dir = PathBuf::from(&vault_dir_str);

    let mut token_path = steam_dir.join("chronokeep_gdrive.json");
    if !token_path.exists() {
        if let Ok(cur) = env::current_exe() {
            if let Some(parent) = cur.parent() {
                let cand = parent.join("chronokeep_gdrive.json");
                if cand.exists() {
                    token_path = cand;
                }
            }
        }
    }

    if !token_path.exists() {
        let msg = to_wide(
            "Conta Google Drive nao conectada!\r\n\r\n\
             Por favor, volte para a aba '1. Configuracao & Instalacao' e conecte a sua conta Google antes de enviar os saves.",
        );
        let cap = to_wide("Google Drive Desconectado");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONWARNING);
        return;
    }

    let h_list = GetDlgItem(hwnd, ID_LIST_ORPHANS as i32);
    if h_list == 0 {
        return;
    }
    let sel_count = SendMessageW(h_list, LB_GETSELCOUNT, 0, 0);
    if sel_count <= 0 {
        let msg = to_wide("Nenhum jogo selecionado na lista para envio.");
        let cap = to_wide("Aviso");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONWARNING);
        return;
    }

    let mut indices: Vec<i32> = vec![0; sel_count as usize];
    SendMessageW(h_list, LB_GETSELITEMS, sel_count as WPARAM, indices.as_mut_ptr() as LPARAM);

    let all_scanned = match SCANNED_ORPHANS.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };

    let mut selected_games = Vec::new();
    for &idx in &indices {
        if let Some(item) = all_scanned.get(idx as usize) {
            selected_games.push(item.clone());
        }
    }

    let h_status = GetDlgItem(hwnd, ID_LBL_ORPHANS_STATUS as i32);
    if h_status != 0 {
        SetWindowTextW(h_status, to_wide("Iniciando sincronizacao com o Google Drive...").as_ptr());
    }

    let hwnd_val = hwnd;
    std::thread::spawn(move || {
        let Some(token) = refresh_access_token(&token_path) else {
            unsafe {
                if h_status != 0 {
                    SetWindowTextW(h_status, to_wide("Erro ao renovar token de acesso do Google Drive.").as_ptr());
                }
                let msg = to_wide("Falha ao autenticar com o Google Drive. Refaca o login na Aba 1.");
                let cap = to_wide("Erro Google Drive");
                MessageBoxW(hwnd_val, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
            }
            return;
        };

        let total_games = selected_games.len();
        let mut total_files_uploaded = 0;
        let mut success_games = 0;

        for (i, game) in selected_games.iter().enumerate() {
            unsafe {
                if h_status != 0 {
                    let msg = format!("Enviando AppID {} ({}/{})...", game.app_id, i + 1, total_games);
                    SetWindowTextW(h_status, to_wide(&msg).as_ptr());
                }
            }

            let manifest_file = vault_dir
                .join(game.account_id.to_string())
                .join(game.app_id.to_string())
                .join("manifest.tsv");
            let mut manifest_data = if let Ok(prev) = fs::read_to_string(&manifest_file) {
                parse_manifest(&prev)
            } else {
                ManifestData::default()
            };
            if manifest_data.change_number == 0 {
                manifest_data.change_number = 1;
            }

            for (rel_name, full_path) in &game.files {
                if let Ok(bytes) = fs::read(full_path) {
                    let ok = sync_orphan_file_to_drive_with_retry(
                        &token,
                        game.account_id,
                        game.app_id,
                        rel_name,
                        &bytes,
                    );
                    if ok {
                        total_files_uploaded += 1;
                    }

                    // Salva copia local no Vault
                    let vault_file = vault_dir
                        .join(game.account_id.to_string())
                        .join(game.app_id.to_string())
                        .join("files")
                        .join(rel_name);
                    if let Some(parent) = vault_file.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&vault_file, &bytes);

                    let sha = sha1_digest(&bytes);
                    let file_ts = fs::metadata(full_path)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let (root_token, clean_filename) = split_root_token(rel_name);
                    manifest_data.files.insert(
                        clean_filename.clone(),
                        ManifestFileRecord {
                            filename: clean_filename,
                            root_token,
                            sha1: sha,
                            timestamp: file_ts,
                            raw_size: bytes.len() as u64,
                            platforms_to_sync: 0xFFFF_FFFF,
                        },
                    );
                }
            }

            let manifest_content = serialize_manifest(&manifest_data);
            if let Some(parent) = manifest_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&manifest_file, manifest_content.as_bytes());

            let _ = sync_orphan_file_to_drive_with_retry(
                &token,
                game.account_id,
                game.app_id,
                "manifest.tsv",
                manifest_content.as_bytes(),
            );

            success_games += 1;
        }

        unsafe {
            let done_text = format!(
                "Concluido com sucesso! {} jogos ({} arquivos) enviados ao Google Drive.",
                success_games, total_files_uploaded
            );
            if h_status != 0 {
                SetWindowTextW(h_status, to_wide(&done_text).as_ptr());
            }
            let cap = to_wide("Sincronizacao Concluida");
            MessageBoxW(hwnd_val, to_wide(&done_text).as_ptr(), cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
        }
    });
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
    cr_common::config::is_gdrive_connected(Path::new(steam_dir))
}

fn resolve_service_port_for_gui(steam_path: &str) -> u16 {
    cr_common::config::resolve_service_port(steam_path)
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

            // Abas de navegacao no topo
            create_child(
                hwnd,
                "BUTTON",
                "1. Configuracao & Instalacao",
                BS_PUSHBUTTON | WS_TABSTOP,
                20,
                10,
                285,
                30,
                ID_TAB_BTN_CONFIG,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "2. Saves Orfaos & Nuvem",
                BS_PUSHBUTTON | WS_TABSTOP,
                315,
                10,
                285,
                30,
                ID_TAB_BTN_ORPHANS,
                font,
            );

            // --- CONTROLES DA ABA 1 (Instalacao & Status) ---
            create_child(
                hwnd,
                "STATIC",
                "1. Pasta da Steam (onde a DLL e o servico serao instalados):",
                0,
                20,
                48,
                580,
                18,
                ID_LBL_STEAM,
                font,
            );
            create_child(
                hwnd,
                "EDIT",
                "",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                20,
                68,
                580,
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
                98,
                580,
                18,
                ID_LBL_VAULT,
                font,
            );
            create_child(
                hwnd,
                "EDIT",
                "",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                20,
                118,
                580,
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
                150,
                580,
                18,
                ID_LBL_GDRIVE_STATUS,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Conectar Conta Google Drive (Login de 1 Clique no Navegador)",
                BS_PUSHBUTTON | WS_TABSTOP,
                20,
                170,
                580,
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
                216,
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
                216,
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
                216,
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
                216,
                100,
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
                262,
                580,
                22,
                ID_LBL_STATUS,
                font,
            );

            // --- CONTROLES DA ABA 2 (Saves Orfaos & Nuvem) ---
            create_child(
                hwnd,
                "STATIC",
                "Saves locais no computador (Steam / Vault). Selecione para enviar ao Google Drive:",
                0,
                20,
                48,
                580,
                18,
                ID_LBL_ORPHANS_TITLE,
                font,
            );
            create_child(
                hwnd,
                "LISTBOX",
                "",
                WS_BORDER | WS_TABSTOP | WS_VSCROLL | LBS_NOTIFY | LBS_MULTIPLESEL,
                20,
                70,
                580,
                230,
                ID_LIST_ORPHANS,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "🔍 Escanear Saves",
                BS_PUSHBUTTON | WS_TABSTOP,
                20,
                310,
                150,
                34,
                ID_BTN_SCAN_ORPHANS,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Marcar Todos",
                BS_PUSHBUTTON | WS_TABSTOP,
                180,
                310,
                105,
                34,
                ID_BTN_SELECT_ALL,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Limpar",
                BS_PUSHBUTTON | WS_TABSTOP,
                295,
                310,
                75,
                34,
                ID_BTN_DESELECT_ALL,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "☁️ Enviar para o Google Drive",
                BS_PUSHBUTTON | WS_TABSTOP,
                380,
                310,
                220,
                34,
                ID_BTN_UPLOAD_ORPHANS,
                font,
            );
            create_child(
                hwnd,
                "STATIC",
                "Pronto. Clique em 'Escanear Saves' ou selecione os jogos e clique em 'Enviar para o Google Drive'.",
                0,
                20,
                354,
                580,
                32,
                ID_LBL_ORPHANS_STATUS,
                font,
            );

            populate_existing_config(hwnd);
            switch_tab(hwnd, 0);
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
                ID_TAB_BTN_CONFIG => {
                    switch_tab(hwnd, 0);
                }
                ID_TAB_BTN_ORPHANS => {
                    switch_tab(hwnd, 1);
                    let is_empty = SCANNED_ORPHANS.lock().map(|g| g.is_empty()).unwrap_or(true);
                    if is_empty {
                        on_scan_orphans(hwnd);
                    }
                }
                ID_BTN_SCAN_ORPHANS => {
                    on_scan_orphans(hwnd);
                }
                ID_BTN_SELECT_ALL => {
                    on_select_all(hwnd, true);
                }
                ID_BTN_DESELECT_ALL => {
                    on_select_all(hwnd, false);
                }
                ID_BTN_UPLOAD_ORPHANS => {
                    on_upload_orphans(hwnd);
                }
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
            120,
            635,
            445,
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
