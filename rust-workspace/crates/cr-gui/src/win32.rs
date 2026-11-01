#![allow(dead_code)]
//! Win32 FFI bindings, window message constants, control IDs, and dialog helpers.

use cr_common::http::to_wide;
use std::ptr::{null, null_mut};

pub type HWND = usize;
pub type HINSTANCE = usize;
pub type HMENU = usize;
pub type HFONT = usize;
pub type HBRUSH = usize;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;

pub const WS_OVERLAPPED: u32 = 0x0000_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_TABSTOP: u32 = 0x0001_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_VSCROLL: u32 = 0x0020_0000;
pub const ES_AUTOHSCROLL: u32 = 0x0080;
pub const BS_PUSHBUTTON: u32 = 0x0000;

pub const LBS_NOTIFY: u32 = 0x0001;
pub const LBS_MULTIPLESEL: u32 = 0x0008;
pub const LB_ADDSTRING: u32 = 0x0180;
pub const LB_RESETCONTENT: u32 = 0x0184;
pub const LB_SETSEL: u32 = 0x0185;
pub const LB_GETSELCOUNT: u32 = 0x0190;
pub const LB_GETSELITEMS: u32 = 0x0191;

pub const SW_HIDE: i32 = 0;
pub const SW_SHOW: i32 = 5;
pub const SW_SHOWNORMAL: i32 = 1;

pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub const WM_SETICON: u32 = 0x0080;
pub const ICON_SMALL: usize = 0;
pub const ICON_BIG: usize = 1;

pub const COLOR_WINDOW: i32 = 5;
pub const DEFAULT_GUI_FONT: i32 = 17;

pub const MB_OK: u32 = 0x0000_0000;
pub const MB_YESNO: u32 = 0x0000_0004;
pub const MB_ICONINFORMATION: u32 = 0x0000_0040;
pub const MB_ICONQUESTION: u32 = 0x0000_0020;
pub const MB_ICONWARNING: u32 = 0x0000_0030;
pub const MB_ICONERROR: u32 = 0x0000_0010;
pub const IDYES: i32 = 6;
#[allow(dead_code)]
pub const IDNO: i32 = 7;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// Embedded companion assets
pub const EMBEDDED_CHRONOKEEP_DLL: &[u8] = include_bytes!("../embedded/chronokeep.dll");
pub const EMBEDDED_CHRONO_DAEMON: &[u8] = include_bytes!("../embedded/chrono-daemon.exe");

// Control IDs — Tab 1 (Configuração & Instalação)
pub const ID_EDIT_STEAM: usize = 101;
pub const ID_EDIT_VAULT: usize = 102;
pub const ID_LBL_STEAM: usize = 103;
pub const ID_LBL_VAULT: usize = 104;
pub const ID_BTN_INSTALL: usize = 201;
pub const ID_BTN_START_SVC: usize = 202;
pub const ID_BTN_OPEN_VAULT: usize = 203;
pub const ID_BTN_OPEN_LOGS: usize = 204;
pub const ID_BTN_GOOGLE_LOGIN: usize = 205;
pub const ID_LBL_STATUS: usize = 301;
pub const ID_LBL_GDRIVE_STATUS: usize = 302;

// Navigation Tabs
pub const ID_TAB_BTN_CONFIG: usize = 401;
pub const ID_TAB_BTN_ORPHANS: usize = 402;

// Control IDs — Tab 2 (Saves Órfãos / Nuvem)
pub const ID_LBL_ORPHANS_TITLE: usize = 501;
pub const ID_LIST_ORPHANS: usize = 502;
pub const ID_BTN_SCAN_ORPHANS: usize = 503;
pub const ID_BTN_SELECT_ALL: usize = 504;
pub const ID_BTN_DESELECT_ALL: usize = 505;
pub const ID_BTN_UPLOAD_ORPHANS: usize = 506;
pub const ID_LBL_ORPHANS_STATUS: usize = 507;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cb_size: u32,
    pub style: u32,
    pub lpfn_wnd_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_instance: HINSTANCE,
    pub h_icon: usize,
    pub h_cursor: usize,
    pub hbr_background: HBRUSH,
    pub lpsz_menu_name: *const u16,
    pub lpsz_class_name: *const u16,
    pub h_icon_sm: usize,
}

#[repr(C)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub w_param: WPARAM,
    pub l_param: LPARAM,
    pub time: u32,
    pub pt: POINT,
}

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
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
    pub fn ShowWindow(h_wnd: HWND, n_cmd_show: i32) -> i32;
    pub fn UpdateWindow(h_wnd: HWND) -> i32;
    pub fn GetMessageW(
        lp_msg: *mut MSG,
        h_wnd: HWND,
        w_msg_filter_min: u32,
        w_msg_filter_max: u32,
    ) -> i32;
    pub fn TranslateMessage(lp_msg: *const MSG) -> i32;
    pub fn DispatchMessageW(lp_msg: *const MSG) -> LRESULT;
    pub fn DefWindowProcW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    pub fn PostQuitMessage(n_exit_code: i32);
    pub fn SendMessageW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    pub fn GetDlgItem(h_dlg: HWND, n_id_dlg_item: i32) -> HWND;
    pub fn SetWindowTextW(h_wnd: HWND, lp_string: *const u16) -> i32;
    pub fn GetWindowTextW(h_wnd: HWND, lp_string: *mut u16, n_max_count: i32) -> i32;
    pub fn MessageBoxW(
        h_wnd: HWND,
        lp_text: *const u16,
        lp_caption: *const u16,
        u_type: u32,
    ) -> i32;
    pub fn LoadCursorW(h_instance: HINSTANCE, lp_cursor_name: *const u16) -> usize;
    pub fn LoadIconW(h_instance: HINSTANCE, lp_icon_name: *const u16) -> usize;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn GetStockObject(i: i32) -> usize;
    pub fn SetBkMode(hdc: usize, mode: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(lp_module_name: *const u16) -> HINSTANCE;
    pub fn GetUserDefaultLocaleName(lp_locale_name: *mut u16, cch_locale_name: i32) -> i32;
    pub fn GetUserDefaultUILanguage() -> u16;
}

#[link(name = "shell32")]
extern "system" {
    pub fn ShellExecuteW(
        hwnd: HWND,
        lp_operation: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show_cmd: i32,
    ) -> usize;
}

pub unsafe fn create_child(
    parent: HWND,
    class_name: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
    font: HFONT,
) -> HWND {
    let cls = to_wide(class_name);
    let title = to_wide(text);
    let h_instance = GetModuleHandleW(null());
    let h_wnd = CreateWindowExW(
        0,
        cls.as_ptr(),
        title.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as HMENU,
        h_instance,
        null_mut(),
    );
    if h_wnd != 0 && font != 0 {
        SendMessageW(h_wnd, WM_SETFONT, font, 1);
    }
    h_wnd
}

pub unsafe fn get_control_text(hwnd_parent: HWND, id: usize) -> String {
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

pub unsafe fn set_control_text(hwnd_parent: HWND, id: usize, text: &str) {
    let h_ctrl = GetDlgItem(hwnd_parent, id as i32);
    if h_ctrl != 0 {
        let w = to_wide(text);
        SetWindowTextW(h_ctrl, w.as_ptr());
    }
}
