#![windows_subsystem = "windows"]
//! Native Win32 GUI Installer & Configurator (`ChronoKeepSetup.exe`).
//! Installs the Bridge DLL (`chronokeep.dll` + optional host loader compatibility),
//! deploys `chrono-daemon.exe`, configures the Vault path, and provides 1-Click
//! Google Drive OAuth2 Browser Login using the pre-verified Desktop OAuth client.

mod installer;
mod oauth_flow;
mod orphans;
mod service_mgr;
mod win32;
mod window;

use cr_common::http::to_wide;
use installer::run_silent_install;
use std::env;
use std::ptr::{null, null_mut};
use win32::*;
use window::wnd_proc;

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
