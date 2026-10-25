#![windows_subsystem = "windows"]
//! Native Win32 GUI Installer & Configurator (`NebulaSetup.exe`).
//! Installs the Bridge DLL (`nebula_core.dll` + optional host loader compatibility),
//! deploys `cr-service.exe`, configures the Vault path & Google OAuth credentials,
//! and manages the background service.

use cr_proto::cloud_messages::DEFAULT_SERVICE_PORT;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
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
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_PUSHBUTTON: u32 = 0x0000;
const BS_AUTOCHECKBOX: u32 = 0x0003;

const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_COMMAND: u32 = 0x0111;
const WM_SETFONT: u32 = 0x0030;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;

const COLOR_WINDOW: i32 = 5;
const DEFAULT_GUI_FONT: i32 = 17;
const SW_SHOWNORMAL: i32 = 1;
const MB_OK: u32 = 0x0000_0000;
const MB_ICONINFORMATION: u32 = 0x0000_0040;
const MB_ICONERROR: u32 = 0x0000_0010;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const ID_EDIT_STEAM: usize = 101;
const ID_EDIT_VAULT: usize = 102;
const ID_EDIT_PORT: usize = 103;
const ID_EDIT_DLL_NAME: usize = 104;
const ID_CHK_LOADER_COMPAT: usize = 105;
const ID_EDIT_GOOGLE_ID: usize = 106;
const ID_EDIT_GOOGLE_SECRET: usize = 107;
const ID_BTN_INSTALL: usize = 201;
const ID_BTN_START_SVC: usize = 202;
const ID_BTN_OPEN_VAULT: usize = 203;
const ID_BTN_OPEN_LOGS: usize = 204;
const ID_LBL_STATUS: usize = 301;

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
    fn GetMessageW(lp_msg: *mut MSG, h_wnd: HWND, w_msg_filter_min: u32, w_msg_filter_max: u32) -> i32;
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

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
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

unsafe fn is_checked(hwnd_parent: HWND, id: usize) -> bool {
    let h_ctrl = GetDlgItem(hwnd_parent, id as i32);
    if h_ctrl == 0 {
        return false;
    }
    SendMessageW(h_ctrl, BM_GETCHECK, 0, 0) as usize == BST_CHECKED
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
    let mut port = DEFAULT_SERVICE_PORT.to_string();
    let mut dll_name = "chronokeep.dll".to_string();
    let mut google_id = String::new();
    let mut google_secret = String::new();

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
                } else if k.eq_ignore_ascii_case("port") && !v.is_empty() {
                    port = v.to_string();
                } else if k.eq_ignore_ascii_case("dll_name") && !v.is_empty() {
                    dll_name = v.to_string();
                } else if k.eq_ignore_ascii_case("google_client_id") {
                    google_id = v.to_string();
                } else if k.eq_ignore_ascii_case("google_client_secret") {
                    google_secret = v.to_string();
                }
            }
        }
    }

    set_control_text(hwnd, ID_EDIT_STEAM, &steam_dir);
    set_control_text(hwnd, ID_EDIT_VAULT, &vault_dir);
    set_control_text(hwnd, ID_EDIT_PORT, &port);
    set_control_text(hwnd, ID_EDIT_DLL_NAME, &dll_name);
    set_control_text(hwnd, ID_EDIT_GOOGLE_ID, &google_id);
    set_control_text(hwnd, ID_EDIT_GOOGLE_SECRET, &google_secret);

    let chk = GetDlgItem(hwnd, ID_CHK_LOADER_COMPAT as i32);
    if chk != 0 {
        SendMessageW(chk, BM_SETCHECK, BST_CHECKED, 0);
    }

    refresh_status_label(hwnd);
}

unsafe fn refresh_status_label(hwnd: HWND) {
    let steam_dir = get_control_text(hwnd, ID_EDIT_STEAM);
    let dll_name = get_control_text(hwnd, ID_EDIT_DLL_NAME);
    let port: u16 = get_control_text(hwnd, ID_EDIT_PORT)
        .parse()
        .unwrap_or(DEFAULT_SERVICE_PORT);

    let dll_exists = Path::new(&steam_dir).join(&dll_name).exists()
        || Path::new(&steam_dir).join("cloud_redirect.dll").exists();
    let svc_online = check_service_online(port);

    let status_text = format!(
        "Status: DLL na Steam: [{}]   |   Servico Local (127.0.0.1:{}): [{}]",
        if dll_exists {
            "INSTALADA"
        } else {
            "NAO ENCONTRADA"
        },
        port,
        if svc_online { "ONLINE" } else { "OFFLINE" }
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
    if !content.contains("[cloud]") {
        let updated = format!("{}\r\n[cloud]\r\nenabled = true\r\n", content.trim_end());
        let _ = fs::write(&toml_path, updated);
    }
}

unsafe fn on_install_and_save(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);
    let port_str = get_control_text(hwnd, ID_EDIT_PORT);
    let mut dll_name = get_control_text(hwnd, ID_EDIT_DLL_NAME);
    let google_id = get_control_text(hwnd, ID_EDIT_GOOGLE_ID);
    let google_secret = get_control_text(hwnd, ID_EDIT_GOOGLE_SECRET);
    let compat_checked = is_checked(hwnd, ID_CHK_LOADER_COMPAT);

    if dll_name.is_empty() {
        dll_name = "chronokeep.dll".to_string();
    }
    if !dll_name.to_lowercase().ends_with(".dll") {
        dll_name.push_str(".dll");
    }

    let steam_dir = Path::new(&steam_dir_str);
    if !steam_dir.exists() {
        let msg = to_wide("A pasta da Steam informada nao existe. Verifique o caminho.");
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    let _ = fs::create_dir_all(&vault_dir_str);

    // 1. Write chronokeep.ini into Steam directory
    let port: u16 = port_str.parse().unwrap_or(DEFAULT_SERVICE_PORT);
    let ini_content = format!(
        "# Configuracao do ChronoKeep gerada pelo Instalador GUI\r\n\
         port={}\r\n\
         vault_dir={}\r\n\
         dll_name={}\r\n\
         google_client_id={}\r\n\
         google_client_secret={}\r\n",
        port, vault_dir_str, dll_name, google_id, google_secret
    );
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Err(e) = fs::write(&ini_path, ini_content) {
        let msg = to_wide(&format!("Falha ao gravar chronokeep.ini: {}", e));
        let cap = to_wide("Erro");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
        return;
    }

    // 2. Copy DLL and chrono-daemon.exe from the installer's folder to Steam folder
    let mut deployed_files = Vec::new();

    let src_dll = find_companion_binary(&dll_name)
        .or_else(|| find_companion_binary("chronokeep.dll"))
        .or_else(|| find_companion_binary("cloud_redirect.dll"));

    if let Some(src) = src_dll {
        let target_custom = steam_dir.join(&dll_name);
        if src != target_custom {
            let _ = fs::copy(&src, &target_custom);
        }
        deployed_files.push(dll_name.clone());

        if compat_checked && !dll_name.eq_ignore_ascii_case("cloud_redirect.dll") {
            let target_compat = steam_dir.join("cloud_redirect.dll");
            let _ = fs::copy(&src, &target_compat);
            deployed_files.push("cloud_redirect.dll (ponte loader)".to_string());
        }
    }

    if let Some(src_svc) = find_companion_binary("chrono-daemon.exe") {
        let target_svc = steam_dir.join("chrono-daemon.exe");
        if src_svc != target_svc {
            let _ = fs::copy(&src_svc, &target_svc);
        }
        deployed_files.push("chrono-daemon.exe".to_string());
    }

    ensure_opensteamtool_cloud_enabled(steam_dir);

    // 3. Start chrono-daemon.exe if not already online
    start_service_from_gui(steam_dir, port);
    refresh_status_label(hwnd);

    let summary = format!(
        "Configuracao e instalacao do ChronoKeep concluidas com sucesso!\r\n\r\n\
         Pasta da Steam: {}\r\n\
         Pasta de Saves (Vault): {}\r\n\
         Porta Local: 127.0.0.1:{}\r\n\
         Arquivos instalados: {}",
        steam_dir_str,
        vault_dir_str,
        port,
        if deployed_files.is_empty() {
            "chronokeep.ini".to_string()
        } else {
            deployed_files.join(", ")
        }
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
                "2. Pasta do Cofre de Saves (Local / Google Drive Desktop / OneDrive):",
                0,
                20,
                74,
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
                96,
                560,
                24,
                ID_EDIT_VAULT,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "3. Nome da DLL Principal:",
                0,
                20,
                134,
                260,
                20,
                0,
                font,
            );
            create_child(
                hwnd,
                "EDIT",
                "chronokeep.dll",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                20,
                156,
                260,
                24,
                ID_EDIT_DLL_NAME,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "Porta Local HTTP (127.0.0.1):",
                0,
                310,
                134,
                270,
                20,
                0,
                font,
            );
            create_child(
                hwnd,
                "EDIT",
                "39771",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                310,
                156,
                140,
                24,
                ID_EDIT_PORT,
                font,
            );

            create_child(
                hwnd,
                "BUTTON",
                "Criar tambem ponte de compatibilidade para OpenSteamTool / HubcapTools",
                BS_AUTOCHECKBOX | WS_TABSTOP,
                20,
                190,
                560,
                22,
                ID_CHK_LOADER_COMPAT,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "4. Google Drive API — OAuth Client ID (Opcional para sync direto via API):",
                0,
                20,
                224,
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
                246,
                560,
                24,
                ID_EDIT_GOOGLE_ID,
                font,
            );

            create_child(
                hwnd,
                "STATIC",
                "5. Google Drive API — OAuth Client Secret:",
                0,
                20,
                280,
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
                302,
                560,
                24,
                ID_EDIT_GOOGLE_SECRET,
                font,
            );

            create_child(
                hwnd,
                "BUTTON",
                "Instalar na Steam e Salvar Config",
                BS_PUSHBUTTON | WS_TABSTOP,
                20,
                344,
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
                272,
                344,
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
                396,
                344,
                90,
                34,
                ID_BTN_OPEN_VAULT,
                font,
            );
            create_child(
                hwnd,
                "BUTTON",
                "Ver Logs",
                BS_PUSHBUTTON | WS_TABSTOP,
                494,
                344,
                86,
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
                392,
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
                ID_BTN_INSTALL => {
                    on_install_and_save(hwnd);
                }
                ID_BTN_START_SVC => {
                    let steam_dir = get_control_text(hwnd, ID_EDIT_STEAM);
                    let port: u16 = get_control_text(hwnd, ID_EDIT_PORT)
                        .parse()
                        .unwrap_or(DEFAULT_SERVICE_PORT);
                    let ok = start_service_from_gui(Path::new(&steam_dir), port);
                    refresh_status_label(hwnd);
                    let txt = if ok {
                        format!("Servico chrono-daemon.exe esta ONLINE em 127.0.0.1:{}!", port)
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
                    ShellExecuteW(hwnd, op.as_ptr(), path_w.as_ptr(), null(), null(), SW_SHOWNORMAL);
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
                    ShellExecuteW(hwnd, op.as_ptr(), path_w.as_ptr(), null(), null(), SW_SHOWNORMAL);
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
    unsafe {
        let h_instance = GetModuleHandleW(null());
        let class_name = to_wide("ChronoKeepSetupWnd");
        let window_title = to_wide("ChronoKeep — Instalador e Configurador Cloud Save");

        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: wnd_proc,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance,
            h_icon: 0,
            h_cursor: LoadCursorW(0, 32512 as *const u16), // IDC_ARROW
            hbr_background: (COLOR_WINDOW + 1) as HBRUSH,
            lpsz_menu_name: null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: 0,
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
            465,
            0,
            0,
            h_instance,
            null_mut(),
        );

        if hwnd == 0 {
            return;
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

