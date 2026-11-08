//! Native Windows notification balloon/toast delivery.

#[cfg(target_os = "windows")]
use std::ffi::c_void;
#[cfg(target_os = "windows")]
use std::ptr::null_mut;

#[cfg(target_os = "windows")]
#[repr(C)]
#[allow(non_snake_case)]
struct NOTIFYICONDATAW {
    cbSize: u32,
    hWnd: *mut c_void,
    uID: u32,
    uFlags: u32,
    uCallbackMessage: u32,
    hIcon: *mut c_void,
    szTip: [u16; 128],
    dwState: u32,
    dwStateMask: u32,
    szInfo: [u16; 256],
    uTimeoutOrVersion: u32,
    szInfoTitle: [u16; 64],
    dwInfoFlags: u32,
    guidItem: [u8; 16],
    hBalloonIcon: *mut c_void,
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn CreateWindowExW(
        dwExStyle: u32,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: u32,
        X: i32,
        Y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: *mut c_void,
        hMenu: *mut c_void,
        hInstance: *mut c_void,
        lpParam: *mut c_void,
    ) -> *mut c_void;
    fn DestroyWindow(hWnd: *mut c_void) -> i32;
    fn LoadIconW(hInstance: *mut c_void, lpIconName: *const u16) -> *mut c_void;
}

#[cfg(target_os = "windows")]
#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(dwMessage: u32, lpData: *const NOTIFYICONDATAW) -> i32;
}

/// Displays a native Windows notification balloon / Action Center toast.
/// Non-blocking: executes on a dedicated transient thread and unregisters cleanly.
pub fn show_balloon_notification(title: &str, message: &str, is_warning: bool) {
    #[cfg(target_os = "windows")]
    {
        let title_s = title.to_string();
        let msg_s = message.to_string();

        std::thread::spawn(move || {
            let title_w: Vec<u16> = title_s.encode_utf16().chain(std::iter::once(0)).collect();
            let msg_w: Vec<u16> = msg_s.encode_utf16().chain(std::iter::once(0)).collect();
            let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();

            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    std::ptr::null(),
                    0x80000000, // WS_POPUP (hidden top-level window)
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };

            if hwnd.is_null() {
                return;
            }

            let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1001;
            nid.uFlags = 0x00000002 | 0x00000010; // NIF_ICON | NIF_INFO

            // IDI_WARNING (32515) vs IDI_INFORMATION (32516)
            let icon_id = if is_warning {
                32515 as *const u16
            } else {
                32516 as *const u16
            };
            nid.hIcon = unsafe { LoadIconW(null_mut(), icon_id) };

            let title_len = (title_w.len() - 1).min(63);
            nid.szInfoTitle[..title_len].copy_from_slice(&title_w[..title_len]);

            let msg_len = (msg_w.len() - 1).min(255);
            nid.szInfo[..msg_len].copy_from_slice(&msg_w[..msg_len]);

            nid.dwInfoFlags = if is_warning {
                0x00000002 // NIIF_WARNING
            } else {
                0x00000001 // NIIF_INFO
            };

            unsafe {
                Shell_NotifyIconW(0 /* NIM_ADD */, &nid);
            }

            // Keep alive for 4.5 seconds for Windows Shell to complete toast presentation
            std::thread::sleep(std::time::Duration::from_millis(4500));

            unsafe {
                Shell_NotifyIconW(2 /* NIM_DELETE */, &nid);
                DestroyWindow(hwnd);
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (title, message, is_warning);
    }
}
