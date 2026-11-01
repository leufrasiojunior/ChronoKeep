//! ChronoKeep Bridge DLL (`chronokeep.dll`).
//! Features:
//! 1. Autonomous `DllMain` initialization thread (auto-discovers Steam path, `stplug-in` / `applist`
//!    AppIDs, spawns `chrono-daemon.exe`, and hooks `steamclient64.dll` without requiring external calls).
//! 2. Native `CK_*` API exports (`CK_AttachRuntime`, `CK_InstallTransportHooks`, `CK_DispatchCloudRpc`, etc.)
//!    plus host-loader compatibility aliases so existing proxy DLLs work seamlessly.

pub mod ipc;
pub mod scanner;
pub mod vtable;

use cr_proto::cloud_messages::DEFAULT_SERVICE_PORT;
use std::collections::HashSet;
use std::ffi::CStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[no_mangle]
#[used]
pub static CHRONOKEEP_BRIDGE_VERSION_MARKER: &[u8] =
    concat!("CK_BRIDGE_VER:", env!("CARGO_PKG_VERSION"), "\0").as_bytes();

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

#[link(name = "kernel32")]
extern "system" {
    fn DisableThreadLibraryCalls(hLibModule: usize) -> i32;
    fn CreateThread(
        lpThreadAttributes: *mut std::ffi::c_void,
        dwStackSize: usize,
        lpStartAddress: Option<unsafe extern "system" fn(*mut std::ffi::c_void) -> u32>,
        lpParameter: *mut std::ffi::c_void,
        dwCreationFlags: u32,
        lpThreadId: *mut u32,
    ) -> usize;
    fn GetModuleFileNameW(hModule: usize, lpFilename: *mut u16, nSize: u32) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
}

pub fn get_local_timestamp_str() -> String {
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

const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_PROCESS_DETACH: u32 = 0;

static DLL_HMODULE: AtomicUsize = AtomicUsize::new(0);

pub fn get_dll_directory() -> Option<PathBuf> {
    let hmod = DLL_HMODULE.load(Ordering::Acquire);
    let mut buf = [0u16; 1024];
    unsafe {
        let len = GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32);
        if len > 0 {
            let path_str = String::from_utf16_lossy(&buf[..len as usize]);
            let p = PathBuf::from(path_str);
            return p.parent().map(|d| d.to_path_buf());
        }
    }
    None
}

#[repr(C)]
pub struct CK_PlaytimeRecord {
    pub minutes_forever: u32,
    pub minutes_last_two_weeks: u32,
    pub last_played_time: u32,
    pub playtime_windows: u32,
    pub playtime_mac: u32,
    pub playtime_linux: u32,
}

#[repr(C)]
pub struct CK_AchievementRecord {
    pub stat_id: u32,
    pub bits: u32,
    pub unlock_times: [u32; 32],
}

#[allow(non_camel_case_types)]
pub type CK_NotifyCallback =
    Option<unsafe extern "C" fn(level: i32, message: *const c_char, duration_ms: i32)>;

static INIT_DONE: AtomicBool = AtomicBool::new(false);
static ACCOUNT_ID: AtomicU32 = AtomicU32::new(0);
static SERVICE_PORT: AtomicU16 = AtomicU16::new(DEFAULT_SERVICE_PORT);

static STEAM_PATH: OnceLock<Mutex<String>> = OnceLock::new();
static NAMESPACE_APPS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
static LAST_PUSHED_SESSION: OnceLock<Mutex<(String, u32, Vec<u32>, std::time::Instant)>> =
    OnceLock::new();
static APPLIST_LOGGED_ONCE: AtomicBool = AtomicBool::new(false);

fn last_session_store() -> &'static Mutex<(String, u32, Vec<u32>, std::time::Instant)> {
    LAST_PUSHED_SESSION.get_or_init(|| {
        Mutex::new((
            String::new(),
            0,
            Vec::new(),
            std::time::Instant::now() - Duration::from_secs(3600),
        ))
    })
}

fn steam_path_store() -> &'static Mutex<String> {
    STEAM_PATH.get_or_init(|| Mutex::new(String::new()))
}

fn apps_store() -> &'static Mutex<HashSet<u32>> {
    NAMESPACE_APPS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn get_steam_path() -> String {
    steam_path_store()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn get_account_id() -> u32 {
    ACCOUNT_ID.load(Ordering::Acquire)
}

pub fn set_account_id(id: u32) {
    if id != 0 {
        ACCOUNT_ID.store(id, Ordering::Release);
    }
}

pub fn get_service_port() -> u16 {
    SERVICE_PORT.load(Ordering::Acquire)
}

pub fn is_namespace_app(app_id: u32) -> bool {
    // Aplicativos internos da Steam nunca devem ser interceptados pelo ChronoKeep
    // (7 = Steam Client, 760 = Screenshots, 241100/241300 = Steam Controller / Entrada Steam)
    if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 || app_id == 241300 {
        return false;
    }

    if let Ok(guard) = apps_store().lock() {
        if guard.contains(&app_id) {
            return true;
        }
    }

    // Auto-detecção dinâmica caso o jogo tenha sido adicionado enquanto a Steam já estava rodando
    let steam_path = get_steam_path();
    if !steam_path.is_empty() {
        let base = Path::new(&steam_path);

        // 1. Verifica se existe config\stplug-in\<app_id>.lua ou .st, ou config\lua\<app_id>.lua
        let lua_file1 = base.join("config").join("stplug-in").join(format!("{}.lua", app_id));
        let st_file = base.join("config").join("stplug-in").join(format!("{}.st", app_id));
        let lua_file2 = base.join("config").join("lua").join(format!("{}.lua", app_id));
        if lua_file1.exists() || st_file.exists() || lua_file2.exists() {
            if let Ok(mut guard) = apps_store().lock() {
                guard.insert(app_id);
            }
            log_line(&format!(
                "[AutoDetect] AppID {} detectado em stplug-in/lua (*.lua) -> Registrado dinamicamente!",
                app_id
            ));
            sync_session_with_service();
            return true;
        }

        // 2. Verifica se existe AppList\<app_id>.txt ou se consta nos arquivos do AppList
        let applist_dir = base.join("AppList");
        if applist_dir.is_dir() {
            if applist_dir.join(format!("{}.txt", app_id)).exists() {
                if let Ok(mut guard) = apps_store().lock() {
                    guard.insert(app_id);
                }
                log_line(&format!(
                    "[AutoDetect] AppID {} detectado em AppList/{}.txt -> Registrado dinamicamente!",
                    app_id, app_id
                ));
                sync_session_with_service();
                return true;
            }
            if let Ok(entries) = fs::read_dir(&applist_dir) {
                for entry in entries.flatten() {
                    if let Ok(content) = fs::read_to_string(entry.path()) {
                        if let Ok(id) = content.trim().parse::<u32>() {
                            if id == app_id {
                                if let Ok(mut guard) = apps_store().lock() {
                                    guard.insert(app_id);
                                }
                                log_line(&format!(
                                    "[AutoDetect] AppID {} detectado dentro do AppList -> Registrado dinamicamente!",
                                    app_id
                                ));
                                sync_session_with_service();
                                return true;
                            }
                        }
                    }
                }
            }
        }

        // NOTA: Jogos legítimos da Steam (sem stplug-in/AppList) NÃO são interceptados aqui.
        // Eles passam direto para a Valve Steam Cloud nativa (Passthrough em Slot 4),
        // garantindo que jogos oficiais mantenham a nuvem oficial da Valve como primária.
    }

    false
}

static DEBUG_MODE: AtomicBool = AtomicBool::new(false);

pub fn is_debug_mode() -> bool {
    DEBUG_MODE.load(Ordering::Relaxed)
}

pub fn set_debug_mode(enabled: bool) {
    DEBUG_MODE.store(enabled, Ordering::Relaxed);
}

pub fn log_debug(msg: &str) {
    if is_debug_mode() {
        log_line(&format!("[DEBUG] {}", msg));
    }
}

pub fn log_line(msg: &str) {
    let steam_dir = get_steam_path();
    let base_dir = if !steam_dir.is_empty() {
        PathBuf::from(steam_dir)
    } else if let Some(dll_dir) = get_dll_directory() {
        dll_dir
    } else {
        PathBuf::from(".")
    };

    let ts = get_local_timestamp_str();
    let formatted = format!("[{}][chronokeep-bridge] {}\r\n", ts, msg);

    // Grava tanto em chronokeep_bridge.log quanto em cloud_redirect.log para total visibilidade
    let p1 = base_dir.join("chronokeep_bridge.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p1) {
        let _ = f.write_all(formatted.as_bytes());
        let _ = f.flush();
    }

    let p2 = base_dir.join("cloud_redirect.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p2) {
        let _ = f.write_all(formatted.as_bytes());
        let _ = f.flush();
    }
}

fn auto_seed_local_apps(steam_path: &str) {
    let mut discovered = Vec::new();

    // 1. Scan <SteamDir>\config\stplug-in\ for <appId>.lua or <appId>.st
    let st_dir = Path::new(steam_path).join("config").join("stplug-in");
    if let Ok(entries) = fs::read_dir(st_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(app_id) = stem.parse::<u32>() {
                    if app_id > 0 {
                        discovered.push(app_id);
                    }
                }
            }
        }
    }

    // 2. Scan <SteamDir>\config\lua\ for <appId>.lua
    let lua_dir = Path::new(steam_path).join("config").join("lua");
    if let Ok(entries) = fs::read_dir(lua_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(app_id) = stem.parse::<u32>() {
                    if app_id > 0 {
                        discovered.push(app_id);
                    }
                }
            }
        }
    }

    // 3. Scan <SteamDir>\AppList\ for GreenLuma-style <index>.txt files containing an AppID
    let applist_dir = Path::new(steam_path).join("AppList");
    if let Ok(entries) = fs::read_dir(applist_dir) {
        for entry in entries.flatten() {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if let Ok(app_id) = content.trim().parse::<u32>() {
                    if app_id > 0 {
                        discovered.push(app_id);
                    }
                }
            }
        }
    }

    if !discovered.is_empty() {
        if let Ok(mut guard) = apps_store().lock() {
            for &id in &discovered {
                guard.insert(id);
            }
        }
        log_line(&format!(
            "[Init] Auto-descobertos {} AppIDs locais (stplug-in / AppList)",
            discovered.len()
        ));
    }
}

fn sync_session_with_service() {
    sync_session_with_service_opts(false);
}

fn sync_session_with_service_opts(force: bool) {
    let steam_path = get_steam_path();
    let port = get_service_port();
    let acct = get_account_id();
    let mut apps: Vec<u32> = apps_store()
        .lock()
        .map(|g| g.iter().copied().collect())
        .unwrap_or_default();
    apps.sort_unstable();

    if !force {
        if let Ok(guard) = last_session_store().lock() {
            if guard.0 == steam_path
                && guard.1 == acct
                && guard.2 == apps
                && guard.3.elapsed() < Duration::from_secs(60)
            {
                return;
            }
        }
    }

    if ipc::push_session_update(port, &steam_path, acct, &apps) {
        if let Ok(mut guard) = last_session_store().lock() {
            *guard = (steam_path, acct, apps, std::time::Instant::now());
        }
    }
}

unsafe extern "system" fn worker_thread_proc(_param: *mut std::ffi::c_void) -> u32 {
    bootstrap_autonomous_worker();
    0
}

fn bootstrap_autonomous_worker() {
    // Wait briefly for host process initialization outside the Windows Loader Lock
    std::thread::sleep(Duration::from_millis(500));

    if !INIT_DONE.load(Ordering::Acquire) {
        let dir_opt = get_dll_directory().or_else(|| {
            std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf()))
        });
        if let Some(dir) = dir_opt {
            let mut s = dir.to_string_lossy().into_owned();
            if !s.ends_with('\\') && !s.ends_with('/') {
                s.push('\\');
            }
            init_internal(&s, "DllMain-Autonomous");
        }
    }

    // Poll up to 45 seconds for steamclient64.dll to be loaded, then hook vtable
    for _ in 0..90 {
        if vtable::is_vtable_hook_installed() {
            break;
        }
        if vtable::install_vtable_hooks() {
            log_line("[Bootstrap] Vtable hooks instalados com sucesso via worker!");
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn init_internal(normalized_path: &str, source_tag: &str) -> bool {
    if INIT_DONE.swap(true, Ordering::AcqRel) {
        return true;
    }

    if let Ok(mut guard) = steam_path_store().lock() {
        *guard = normalized_path.to_string();
    }

    let port = ipc::resolve_service_port(normalized_path);
    SERVICE_PORT.store(port, Ordering::Release);

    let ini_path = Path::new(normalized_path).join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            if let Some((k, v)) = line.trim().split_once('=') {
                if k.trim().eq_ignore_ascii_case("debug") {
                    let d = v.trim().eq_ignore_ascii_case("true") || v.trim() == "1";
                    DEBUG_MODE.store(d, Ordering::Relaxed);
                }
            }
        }
    }

    log_line(&format!(
        "=== Bridge DLL v{} inicializada [{}] (porta={}, debug={}) ===",
        env!("CARGO_PKG_VERSION"),
        source_tag,
        port,
        if is_debug_mode() { "ON" } else { "OFF" }
    ));
    log_line(&format!("Steam Path: {}", normalized_path));

    auto_seed_local_apps(normalized_path);

    let service_ok = ipc::ensure_service_running(normalized_path, port);
    log_line(&format!(
        "Conexao com chrono-daemon.exe em 127.0.0.1:{}: {}",
        port,
        if service_ok { "ONLINE" } else { "AGUARDANDO" }
    ));

    sync_session_with_service();
    true
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    hinst_dll: usize,
    fdw_reason: u32,
    _lpv_reserved: *mut std::ffi::c_void,
) -> i32 {
    if fdw_reason == DLL_PROCESS_ATTACH {
        DLL_HMODULE.store(hinst_dll, Ordering::Release);
        let _ = DisableThreadLibraryCalls(hinst_dll);

        // Pre-resolve Steam path immediately from the DLL location itself
        if let Some(dll_dir) = get_dll_directory() {
            let mut s = dll_dir.to_string_lossy().into_owned();
            if !s.ends_with('\\') && !s.ends_with('/') {
                s.push('\\');
            }
            if let Ok(mut guard) = steam_path_store().lock() {
                if guard.is_empty() {
                    *guard = s;
                }
            }
        }

        log_line(&format!(
            "DLL carregada com sucesso no processo (PID={})",
            GetCurrentProcessId()
        ));

        // Use raw Win32 CreateThread to avoid Rust std::thread runtime deadlock inside DllMain
        CreateThread(
            std::ptr::null_mut(),
            0,
            Some(worker_thread_proc),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );
    } else if fdw_reason == DLL_PROCESS_DETACH {
        log_line("[DllMain] Processo encerrando (DLL_PROCESS_DETACH) -> notificando chrono-daemon");
        let port = get_service_port();
        ipc::notify_shutdown(port);
    }
    1
}

// ============================================================================
// PRIMARY NATIVE API (`CK_*` — ChronoKeep Custom Function Names)
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn CK_AttachRuntime(
    steam_path: *const c_char,
    _notify: CK_NotifyCallback,
) -> bool {
    if steam_path.is_null() {
        return false;
    }
    let raw_path = CStr::from_ptr(steam_path).to_string_lossy().into_owned();
    let mut normalized = raw_path;
    if !normalized.is_empty() && !normalized.ends_with('\\') && !normalized.ends_with('/') {
        normalized.push('\\');
    }
    init_internal(&normalized, "CK_AttachRuntime")
}

#[no_mangle]
pub unsafe extern "C" fn CK_InstallTransportHooks() -> bool {
    if !INIT_DONE.load(Ordering::Acquire) {
        return false;
    }
    vtable::install_vtable_hooks()
}

#[no_mangle]
pub unsafe extern "C" fn CK_DispatchCloudRpc(
    method: *const c_char,
    app_id: u32,
    account_id: u32,
    req_body: *const u8,
    req_len: u32,
    resp_buf: *mut u8,
    resp_max_len: u32,
    resp_len: *mut u32,
    eresult: *mut i32,
) -> bool {
    if resp_len.is_null() || eresult.is_null() {
        return false;
    }
    *resp_len = 0;
    *eresult = 2;

    if !INIT_DONE.load(Ordering::Acquire) || method.is_null() || resp_buf.is_null() {
        return false;
    }
    if !is_namespace_app(app_id) {
        return false;
    }

    if account_id != 0 && get_account_id() == 0 {
        set_account_id(account_id);
    }

    let Ok(method_str) = CStr::from_ptr(method).to_str() else {
        return false;
    };

    let req_slice = if req_body.is_null() || req_len == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(req_body, req_len as usize)
    };

    let steam_path = get_steam_path();
    let port = get_service_port();
    let eff_acct = if account_id != 0 {
        account_id
    } else {
        get_account_id()
    };

    let ipc::RpcForwardResult::Handled(res_code, resp_bytes) =
        ipc::forward_rpc(port, &steam_path, method_str, app_id, eff_acct, req_slice)
    else {
        return false;
    };

    if resp_bytes.len() > resp_max_len as usize {
        return false;
    }

    if !resp_bytes.is_empty() {
        std::ptr::copy_nonoverlapping(resp_bytes.as_ptr(), resp_buf, resp_bytes.len());
    }
    *resp_len = resp_bytes.len() as u32;
    *eresult = res_code;

    log_line(&format!(
        "[CK_DispatchCloudRpc] {} app={} -> eresult={} ({}B)",
        method_str,
        app_id,
        res_code,
        resp_bytes.len()
    ));
    true
}

#[no_mangle]
pub unsafe extern "C" fn CK_RegisterAppList(app_ids: *const u32, count: u32) {
    let slice = if app_ids.is_null() || count == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(app_ids, count as usize)
    };

    let (changed, total) = if let Ok(mut guard) = apps_store().lock() {
        let before = guard.len();
        for &id in slice {
            if id != 0 {
                guard.insert(id);
            }
        }
        (guard.len() != before, guard.len())
    } else {
        (false, 0)
    };

    let first_log = !APPLIST_LOGGED_ONCE.swap(true, Ordering::AcqRel);
    if changed || first_log {
        log_line(&format!(
            "[CK_API] AppList atualizada: +{} apps recebidos do loader (total gerenciados={})",
            slice.len(),
            total
        ));
    }
    sync_session_with_service_opts(changed);
}

#[no_mangle]
pub unsafe extern "C" fn CK_RegisterApp(app_id: u32) {
    if app_id == 0 {
        return;
    }
    if let Ok(mut guard) = apps_store().lock() {
        guard.insert(app_id);
    }
    log_line(&format!("[CK_API] App registrado: {}", app_id));
    sync_session_with_service();
}

#[no_mangle]
pub unsafe extern "C" fn CK_UnregisterApp(app_id: u32) {
    if let Ok(mut guard) = apps_store().lock() {
        guard.remove(&app_id);
    }
    log_line(&format!("[CK_API] App removido: {}", app_id));
    sync_session_with_service();
}

#[no_mangle]
pub unsafe extern "C" fn CK_QueryAppRegistered(app_id: u32) -> bool {
    is_namespace_app(app_id)
}

#[no_mangle]
pub unsafe extern "C" fn CK_BindAccount(account_id: u32) {
    if account_id != 0 {
        set_account_id(account_id);
        log_line(&format!("[CK_API] AccountId vinculado: {}", account_id));
        sync_session_with_service();
    }
}

#[no_mangle]
pub unsafe extern "C" fn CK_NotifyGameState(app_id: u32, running: bool) {
    log_line(&format!(
        "[CK_API] Estado do jogo: app={} running={}",
        app_id, running
    ));
    if !running && app_id != 0 {
        let port = get_service_port();
        let acct = get_account_id();
        let _ = ipc::push_game_state(port, app_id, acct, false);
    }
}

#[no_mangle]
pub unsafe extern "C" fn CK_DetachRuntime() {
    log_line("[CK_API] Runtime desconectado");
}

// ============================================================================
// HOST LOADER COMPATIBILITY BRIDGES (For OpenSteamTool / HubcapTools / SteamTools)
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn CR_InitCloudSave(
    steam_path: *const c_char,
    notify: CK_NotifyCallback,
) -> bool {
    CK_AttachRuntime(steam_path, notify)
}

#[no_mangle]
pub unsafe extern "C" fn CR_InstallVtableHooks() -> bool {
    CK_InstallTransportHooks()
}

#[no_mangle]
pub unsafe extern "C" fn CR_HandleCloudRpc(
    method: *const c_char,
    app_id: u32,
    account_id: u32,
    req_body: *const u8,
    req_len: u32,
    resp_buf: *mut u8,
    resp_max_len: u32,
    resp_len: *mut u32,
    eresult: *mut i32,
) -> bool {
    CK_DispatchCloudRpc(
        method,
        app_id,
        account_id,
        req_body,
        req_len,
        resp_buf,
        resp_max_len,
        resp_len,
        eresult,
    )
}

#[no_mangle]
pub unsafe extern "C" fn CR_SetApps(app_ids: *const u32, count: u32) {
    CK_RegisterAppList(app_ids, count)
}

#[no_mangle]
pub unsafe extern "C" fn CR_AddApp(app_id: u32) {
    CK_RegisterApp(app_id)
}

#[no_mangle]
pub unsafe extern "C" fn CR_RemoveApp(app_id: u32) {
    CK_UnregisterApp(app_id)
}

#[no_mangle]
pub unsafe extern "C" fn CR_IsApp(app_id: u32) -> bool {
    CK_QueryAppRegistered(app_id)
}

#[no_mangle]
pub unsafe extern "C" fn CR_SetAccountId(account_id: u32) {
    CK_BindAccount(account_id)
}

#[no_mangle]
pub unsafe extern "C" fn CR_NotifyAppRunning(app_id: u32, running: bool) {
    CK_NotifyGameState(app_id, running)
}

#[no_mangle]
pub unsafe extern "C" fn CR_EnableStatsSync(_achievements: bool, _playtime: bool) {}

#[no_mangle]
pub unsafe extern "C" fn CR_NotifyStatsStored(_app_id: u32) {}

#[no_mangle]
pub unsafe extern "C" fn CR_NotifyStatsStoredFrom(_app_id: u32, _native_app_id: u32) {}

#[no_mangle]
pub unsafe extern "C" fn CR_GetPlaytime(_app_id: u32, _out: *mut CK_PlaytimeRecord) -> bool {
    false
}

#[no_mangle]
pub unsafe extern "C" fn CR_GetAchievements(
    _app_id: u32,
    _out: *mut CK_AchievementRecord,
    _max_blocks: u32,
) -> u32 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn CR_DrainPlaytimeUpdates() {}

#[no_mangle]
pub unsafe extern "C" fn CR_Shutdown() {
    CK_DetachRuntime()
}

#[no_mangle]
pub unsafe extern "C" fn CloudOnSendPkt(
    _this_ptr: *mut std::ffi::c_void,
    _data: *const u8,
    _size: u32,
    _recv_pkt_fn: *mut std::ffi::c_void,
) -> i32 {
    if !INIT_DONE.load(Ordering::Acquire) {
        let dir_opt = get_dll_directory().or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|p| p.to_path_buf()))
        });
        if let Some(dir) = dir_opt {
            let mut s = dir.to_string_lossy().into_owned();
            if !s.ends_with('\\') && !s.ends_with('/') {
                s.push('\\');
            }
            init_internal(&s, "CloudOnSendPkt");
            vtable::install_vtable_hooks();
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn CloudRedirect_CliMain(_argc: i32, _argv: *mut *mut c_char) -> i32 {
    0
}
