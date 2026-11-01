//! Lifecycle management, Steam process monitoring, and structured logging.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[repr(C)]
#[allow(non_snake_case)]
struct SYSTEMTIME {
    wYear: u16,
    wMonth: u16,
    wDayOfWeek: u16,
    wDay: u16,
    wHour: u16,
    wMinute: u16,
    wSecond: u16,
    wMilliseconds: u16,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
}

pub fn get_local_timestamp_str() -> String {
    #[cfg(target_os = "windows")]
    {
        let mut st = SYSTEMTIME {
            wYear: 0,
            wMonth: 0,
            wDayOfWeek: 0,
            wDay: 0,
            wHour: 0,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        unsafe { GetLocalTime(&mut st) };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        "1970-01-01 00:00:00".to_string()
    }
}

static WATCHING_PID: AtomicU32 = AtomicU32::new(0);

pub fn ensure_parent_watcher(pid: u32, log_path: Arc<PathBuf>) {
    if pid == 0 {
        return;
    }
    if WATCHING_PID
        .compare_exchange(0, pid, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        std::thread::spawn(move || {
            type HANDLE = *mut std::ffi::c_void;
            #[link(name = "kernel32")]
            extern "system" {
                fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> HANDLE;
                fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) -> u32;
                fn CloseHandle(hObject: HANDLE) -> i32;
            }
            const SYNCHRONIZE: u32 = 0x0010_0000;
            const INFINITE: u32 = 0xFFFF_FFFF;

            let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                log_msg(
                    &log_path,
                    &format!(
                        "[Lifecycle] Nao foi possivel abrir handle de sincronizacao para Steam PID={}",
                        pid
                    ),
                );
                return;
            }

            log_msg(
                &log_path,
                &format!(
                    "[Lifecycle] Monitorando encerramento do processo Steam (PID={})...",
                    pid
                ),
            );

            unsafe {
                WaitForSingleObject(handle, INFINITE);
                CloseHandle(handle);
            }

            log_msg(
                &log_path,
                &format!(
                    "[Lifecycle] Processo Steam (PID={}) encerrou. Drenando uploads pendentes antes de finalizar chrono-daemon.exe...",
                    pid
                ),
            );

            crate::gdrive::wait_for_all_uploads(
                std::time::Duration::from_secs(25),
                Some(log_path.as_ref()),
            );
            std::thread::sleep(std::time::Duration::from_millis(150));
            std::process::exit(0);
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = log_path;
    }
}

pub static DEBUG_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_debug_mode() -> bool {
    DEBUG_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_debug_mode(enabled: bool) {
    DEBUG_MODE.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub fn log_msg(log_path: &Path, msg: &str) {
    let line = format!("[{}][chrono-daemon] {}\r\n", get_local_timestamp_str(), msg);
    print!("{}", line);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

pub fn log_debug(log_path: &Path, msg: &str) {
    if is_debug_mode() {
        let line = format!("[{}][chrono-daemon][DEBUG] {}\r\n", get_local_timestamp_str(), msg);
        print!("{}", line);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path) {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}
