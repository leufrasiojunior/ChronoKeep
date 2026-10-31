//! Process supervision and daemon management for `ChronoKeep.exe`.

use crate::win32::CREATE_NO_WINDOW;
use cr_common::config::{check_daemon_health, find_companion_binary};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::time::Duration;

pub fn is_process_running(proc_name: &str) -> bool {
    let mut cmd = std::process::Command::new("tasklist");
    cmd.args(["/FI", &format!("IMAGENAME eq {}", proc_name)]);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output();
    if let Ok(output) = out {
        let text = String::from_utf8_lossy(&output.stdout);
        text.to_ascii_lowercase().contains(&proc_name.to_ascii_lowercase())
    } else {
        false
    }
}

pub fn kill_process_and_wait(proc_name: &str, max_wait_ms: u64) {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/IM", proc_name]);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd.status();
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < max_wait_ms as u128 {
        if !is_process_running(proc_name) {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

pub fn launch_steam(steam_dir: &Path) {
    let steam_exe = steam_dir.join("steam.exe");
    if steam_exe.exists() {
        let mut cmd = std::process::Command::new(&steam_exe);
        cmd.current_dir(steam_dir);
        let _ = cmd.spawn();
    }
}

pub fn check_service_online(port: u16) -> bool {
    check_daemon_health(port)
}

pub fn start_service_from_gui(steam_dir: &Path, port: u16) -> bool {
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
        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
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

pub fn stop_service_from_gui(port: u16) -> bool {
    use std::io::Write;
    use std::net::TcpStream;
    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
        let _ = stream.write_all(b"POST /v1/shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    kill_process_and_wait("chrono-daemon.exe", 1500);
    !check_service_online(port)
}
