//! Vtable Hook Manager for `CClientUnifiedServiceTransport` inside `steamclient64.dll`.
//! Intercepts Slots 4, 5, 7, and 8 to redirect `Cloud.*` RPCs for registered namespace AppIDs
//! to `cr-service.exe`.

use crate::ipc::{self, RpcForwardResult};
use crate::scanner::{resolve_steamclient_symbols, SteamClientSymbols};
use crate::{get_account_id, get_service_port, get_steam_path, is_namespace_app, log_line, set_account_id};
use cr_proto::cloud_messages::{
    extract_app_id, is_handled_cloud_rpc, ERESULT_FAIL, RPC_EXIT_SYNC,
};
use cr_proto::wire::{find_field, parse_message};

#[inline]
fn is_valve_system_app(app_id: u32) -> bool {
    matches!(app_id, 0 | 7 | 760 | 241100 | 241300)
}
use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(lpModuleName: *const u8) -> usize;
    fn VirtualProtect(
        lpAddress: *mut std::ffi::c_void,
        dwSize: usize,
        flNewProtect: u32,
        lpflOldProtect: *mut u32,
    ) -> i32;
}

const PAGE_EXECUTE_READWRITE: u32 = 0x40;

type Slot4Fn = unsafe extern "C" fn(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    req_body: *mut std::ffi::c_void,
    resp_body: *mut std::ffi::c_void,
    flags: *mut i32,
) -> bool;

type Slot5Fn = unsafe extern "C" fn(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    request_msg: *mut std::ffi::c_void,
    response_msg: *mut std::ffi::c_void,
    flags: *mut i64,
) -> bool;

type Slot7Fn = unsafe extern "C" fn(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    body_obj: *mut std::ffi::c_void,
    flags: *mut i32,
) -> bool;

type Slot8Fn = unsafe extern "C" fn(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    request_msg: *mut std::ffi::c_void,
) -> bool;

type ByteSizeFn = unsafe extern "C" fn(body_obj: *mut std::ffi::c_void) -> u64;
type SerializeToArrayFn =
    unsafe extern "C" fn(body_obj: *mut std::ffi::c_void, buf: *mut u8, size: i32) -> u8;
type ParseFromArrayFn =
    unsafe extern "C" fn(body_obj: *mut std::ffi::c_void, data: *const u8, size: i32) -> u8;

static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
static ORIG_SLOT4: AtomicUsize = AtomicUsize::new(0);
static ORIG_SLOT5: AtomicUsize = AtomicUsize::new(0);
static ORIG_SLOT7: AtomicUsize = AtomicUsize::new(0);
static ORIG_SLOT8: AtomicUsize = AtomicUsize::new(0);
static FN_SERIALIZE: AtomicUsize = AtomicUsize::new(0);
static FN_PARSE: AtomicUsize = AtomicUsize::new(0);

pub fn is_vtable_hook_installed() -> bool {
    HOOK_INSTALLED.load(Ordering::Acquire)
}

pub fn install_vtable_hooks() -> bool {
    if HOOK_INSTALLED.load(Ordering::Acquire) {
        return true;
    }

    unsafe {
        let sc_base = GetModuleHandleA(b"steamclient64.dll\0".as_ptr());
        if sc_base == 0 {
            log_line("[VtHook] steamclient64.dll ainda nao carregada no processo");
            return false;
        }

        let Some(syms) = resolve_steamclient_symbols(sc_base) else {
            log_line("[VtHook] ERRO: Falha ao resolver simbolos RTTI/Protobuf em steamclient64.dll");
            return false;
        };

        log_resolved(&syms);

        FN_SERIALIZE.store(syms.serialize_to_array, Ordering::Release);
        FN_PARSE.store(syms.parse_from_array, Ordering::Release);

        let vtable_ptr = syms.transport_vtable as *mut usize;
        let mut old_prot = 0u32;
        if VirtualProtect(
            vtable_ptr.add(4) as *mut std::ffi::c_void,
            5 * std::mem::size_of::<usize>(),
            PAGE_EXECUTE_READWRITE,
            &mut old_prot,
        ) == 0
        {
            log_line("[VtHook] ERRO: VirtualProtect na vtable falhou");
            return false;
        }

        ORIG_SLOT4.store(*vtable_ptr.add(4), Ordering::Release);
        ORIG_SLOT5.store(*vtable_ptr.add(5), Ordering::Release);
        ORIG_SLOT7.store(*vtable_ptr.add(7), Ordering::Release);
        ORIG_SLOT8.store(*vtable_ptr.add(8), Ordering::Release);

        *vtable_ptr.add(4) = hooked_slot4 as *const () as usize;
        *vtable_ptr.add(5) = hooked_slot5 as *const () as usize;
        *vtable_ptr.add(7) = hooked_slot7 as *const () as usize;
        *vtable_ptr.add(8) = hooked_slot8 as *const () as usize;

        let mut dummy = 0u32;
        let _ = VirtualProtect(
            vtable_ptr.add(4) as *mut std::ffi::c_void,
            5 * std::mem::size_of::<usize>(),
            old_prot,
            &mut dummy,
        );

        HOOK_INSTALLED.store(true, Ordering::Release);
        log_line("[VtHook] Hooks instalados com sucesso nos Slots 4, 5, 7 e 8!");
        true
    }
}

fn log_resolved(syms: &SteamClientSymbols) {
    log_line(&format!(
        "[Resolver] steamclient64=0x{:X} | TransportVT=0x{:X} (RVA 0x{:X}) | Serialize=0x{:X} (RVA 0x{:X}) | Parse=0x{:X} (RVA 0x{:X})",
        syms.module_base,
        syms.transport_vtable,
        syms.transport_vtable - syms.module_base,
        syms.serialize_to_array,
        syms.serialize_to_array - syms.module_base,
        syms.parse_from_array,
        syms.parse_from_array - syms.module_base
    ));
}

unsafe fn serialize_body(body_obj: *mut std::ffi::c_void) -> Option<Vec<u8>> {
    if body_obj.is_null() {
        return None;
    }
    let ser_addr = FN_SERIALIZE.load(Ordering::Acquire);
    if ser_addr == 0 {
        return None;
    }

    // Call protobuf MessageLite::ByteSizeLong at vtable slot 8 (offset +64)
    let vtable = *(body_obj as *const usize);
    if vtable == 0 {
        return None;
    }
    let byte_size_fn: ByteSizeFn = std::mem::transmute(*((vtable + 64) as *const usize));
    let expected_size = byte_size_fn(body_obj);
    if expected_size == 0 {
        return Some(Vec::new());
    }
    if expected_size > 16 * 1024 * 1024 {
        return None;
    }

    let mut buf = vec![0u8; expected_size as usize];
    let ser_fn: SerializeToArrayFn = std::mem::transmute(ser_addr);
    if ser_fn(body_obj, buf.as_mut_ptr(), expected_size as i32) == 0 {
        return None;
    }
    Some(buf)
}

unsafe fn parse_into_body(body_obj: *mut std::ffi::c_void, bytes: &[u8]) -> bool {
    if body_obj.is_null() {
        return false;
    }
    if bytes.is_empty() {
        return true;
    }
    let parse_addr = FN_PARSE.load(Ordering::Acquire);
    if parse_addr == 0 {
        return false;
    }
    let parse_fn: ParseFromArrayFn = std::mem::transmute(parse_addr);
    parse_fn(body_obj, bytes.as_ptr(), bytes.len() as i32) != 0
}

unsafe extern "C" fn hooked_slot4(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    req_body: *mut std::ffi::c_void,
    resp_body: *mut std::ffi::c_void,
    flags: *mut i32,
) -> bool {
    let orig: Slot4Fn = std::mem::transmute(ORIG_SLOT4.load(Ordering::Acquire));
    if method_name.is_null() {
        return orig(thisptr, method_name, req_body, resp_body, flags);
    }

    let Ok(method) = CStr::from_ptr(method_name).to_str() else {
        return orig(thisptr, method_name, req_body, resp_body, flags);
    };

    if !method.starts_with("Cloud.") || !is_handled_cloud_rpc(method) {
        return orig(thisptr, method_name, req_body, resp_body, flags);
    }

    let Some(req_bytes) = serialize_body(req_body) else {
        return orig(thisptr, method_name, req_body, resp_body, flags);
    };

    let fields = parse_message(&req_bytes);
    let app_id = extract_app_id(method, &fields);
    if is_valve_system_app(app_id) || !is_namespace_app(app_id) {
        if !is_valve_system_app(app_id) {
            log_line(&format!(
                "[Slot4] PASSTHROUGH {} app={} (não é namespace app -> enviado para a Valve)",
                method, app_id
            ));
        }
        return orig(thisptr, method_name, req_body, resp_body, flags);
    }

    let steam_path = get_steam_path();
    let port = get_service_port();
    let account_id = get_account_id();

    crate::log_debug(&format!(
        "[Slot4] RPC interceptada: method={} app={} acct={} req_bytes={}B",
        method, app_id, account_id, req_bytes.len()
    ));

    let (eresult, resp_bytes) =
        match ipc::forward_rpc(port, &steam_path, method, app_id, account_id, &req_bytes) {
            RpcForwardResult::Handled(er, bytes) => (er, bytes),
            RpcForwardResult::UnhandledPassthrough => {
                return orig(thisptr, method_name, req_body, resp_body, flags);
            }
            RpcForwardResult::TransportError => {
                // Isolamento estrito de namespace: nunca encaminhar AppID gerenciado (stplug-in/lua) à Valve!
                log_line(&format!(
                    "[Slot4] ERRO IPC com chrono-daemon em {} app={} -> retornando ERESULT_FAIL local",
                    method, app_id
                ));
                if !flags.is_null() {
                    *flags.add(2) = 1;
                    *flags.add(3) = ERESULT_FAIL;
                    *flags.add(4) = 0;
                }
                return true;
            }
        };

    if !resp_body.is_null() && !resp_bytes.is_empty() {
        if !parse_into_body(resp_body, &resp_bytes) {
            log_line(&format!(
                "[Slot4] ERRO: ParseFromArray falhou em {} app={}",
                method, app_id
            ));
            return false;
        }
    }

    if !flags.is_null() {
        *flags.add(2) = 1; // transport_ok = 1
        *flags.add(3) = eresult;
        *flags.add(4) = 0; // empty error message
    }

    log_line(&format!(
        "[Slot4] INTERCEPT {} app={} -> eresult={} ({}B)",
        method,
        app_id,
        eresult,
        resp_bytes.len()
    ));
    true
}

unsafe extern "C" fn hooked_slot5(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    request_msg: *mut std::ffi::c_void,
    response_msg: *mut std::ffi::c_void,
    flags: *mut i64,
) -> bool {
    let orig: Slot5Fn = std::mem::transmute(ORIG_SLOT5.load(Ordering::Acquire));
    if method_name.is_null() || request_msg.is_null() || response_msg.is_null() {
        return orig(thisptr, method_name, request_msg, response_msg, flags);
    }

    let Ok(method) = CStr::from_ptr(method_name).to_str() else {
        return orig(thisptr, method_name, request_msg, response_msg, flags);
    };

    if !method.starts_with("Cloud.") || !is_handled_cloud_rpc(method) {
        return orig(thisptr, method_name, request_msg, response_msg, flags);
    }

    // CProtoBufMsg layout: +40 = header*, +48 = body*
    let req_body = *((request_msg as usize + 48) as *const *mut std::ffi::c_void);
    let Some(req_bytes) = serialize_body(req_body) else {
        return orig(thisptr, method_name, request_msg, response_msg, flags);
    };

    let fields = parse_message(&req_bytes);
    let app_id = extract_app_id(method, &fields);
    if is_valve_system_app(app_id) || !is_namespace_app(app_id) {
        if !is_valve_system_app(app_id) {
            log_line(&format!(
                "[Slot5] PASSTHROUGH {} app={} (não é namespace app -> enviado para a Valve)",
                method, app_id
            ));
        }
        return orig(thisptr, method_name, request_msg, response_msg, flags);
    }

    // Extract account_id from request CMsgProtoBufHeader (+40), field 1 (fixed64 steamid)
    if get_account_id() == 0 {
        let req_hdr = *((request_msg as usize + 40) as *const *mut std::ffi::c_void);
        if let Some(hdr_bytes) = serialize_body(req_hdr) {
            let hdr_fields = parse_message(&hdr_bytes);
            if let Some(sid) = find_field(&hdr_fields, 1).and_then(|f| f.as_u64()) {
                let acct = (sid & 0xFFFF_FFFF) as u32;
                if acct != 0 {
                    set_account_id(acct);
                    log_line(&format!(
                        "[Slot5] SteamID capturado do header: {} (accountId={})",
                        sid, acct
                    ));
                }
            }
        }
    }

    let steam_path = get_steam_path();
    let port = get_service_port();
    let account_id = get_account_id();

    crate::log_debug(&format!(
        "[Slot5] RPC interceptada: method={} app={} acct={} req_bytes={}B",
        method, app_id, account_id, req_bytes.len()
    ));

    let resp_hdr = *((response_msg as usize + 40) as *const *mut std::ffi::c_void);
    let resp_body = *((response_msg as usize + 48) as *const *mut std::ffi::c_void);
    if resp_hdr.is_null() || resp_body.is_null() {
        return false;
    }

    let (eresult, resp_bytes) =
        match ipc::forward_rpc(port, &steam_path, method, app_id, account_id, &req_bytes) {
            RpcForwardResult::Handled(er, bytes) => (er, bytes),
            RpcForwardResult::UnhandledPassthrough => {
                return orig(thisptr, method_name, request_msg, response_msg, flags);
            }
            RpcForwardResult::TransportError => {
                // Isolamento estrito de namespace: nunca vazar AppID gerenciado para a Valve!
                log_line(&format!(
                    "[Slot5] ERRO IPC com chrono-daemon em {} app={} -> retornando ERESULT_FAIL local",
                    method, app_id
                ));
                (ERESULT_FAIL, Vec::new())
            }
        };

    if !resp_bytes.is_empty() && !parse_into_body(resp_body, &resp_bytes) {
        log_line(&format!(
            "[Slot5] ERRO: ParseFromArray falhou em {} app={}",
            method, app_id
        ));
        return false;
    }

    // Populate CMsgProtoBufHeader eresult (+216) and error_code (+220) + has_bits (+16)
    let hdr_base = resp_hdr as usize;
    *((hdr_base + 16) as *mut u32) |= 0x6000_0000;
    *((hdr_base + 216) as *mut i32) = eresult;
    *((hdr_base + 220) as *mut i32) = 0;

    if !flags.is_null() {
        let f32 = flags as *mut i32;
        *f32.add(0) = 0;
        *f32.add(2) = 0;
        *f32.add(3) = eresult;
    }

    log_line(&format!(
        "[Slot5] INTERCEPT {} app={} -> eresult={} ({}B)",
        method,
        app_id,
        eresult,
        resp_bytes.len()
    ));
    true
}

unsafe extern "C" fn hooked_slot7(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    body_obj: *mut std::ffi::c_void,
    flags: *mut i32,
) -> bool {
    let orig: Slot7Fn = std::mem::transmute(ORIG_SLOT7.load(Ordering::Acquire));
    if method_name.is_null() {
        return orig(thisptr, method_name, body_obj, flags);
    }
    let Ok(method) = CStr::from_ptr(method_name).to_str() else {
        return orig(thisptr, method_name, body_obj, flags);
    };
    if !method.starts_with("Cloud.") {
        return orig(thisptr, method_name, body_obj, flags);
    }
    if let Some(bytes) = serialize_body(body_obj) {
        let fields = parse_message(&bytes);
        let app_id = extract_app_id(method, &fields);
        if !is_valve_system_app(app_id) && is_namespace_app(app_id) {
            crate::log_debug(&format!("[Slot7] Cloud RPC bloqueada para a Valve: method={} app={}", method, app_id));
            if method == RPC_EXIT_SYNC {
                return orig(thisptr, method_name, body_obj, flags);
            }
            return true;
        }
    }
    orig(thisptr, method_name, body_obj, flags)
}

unsafe extern "C" fn hooked_slot8(
    thisptr: *mut std::ffi::c_void,
    method_name: *const c_char,
    request_msg: *mut std::ffi::c_void,
) -> bool {
    let orig: Slot8Fn = std::mem::transmute(ORIG_SLOT8.load(Ordering::Acquire));
    if method_name.is_null() || request_msg.is_null() {
        return orig(thisptr, method_name, request_msg);
    }
    let Ok(method) = CStr::from_ptr(method_name).to_str() else {
        return orig(thisptr, method_name, request_msg);
    };
    if !method.starts_with("Cloud.") {
        return orig(thisptr, method_name, request_msg);
    }
    let body_obj = *((request_msg as usize + 48) as *const *mut std::ffi::c_void);
    if let Some(bytes) = serialize_body(body_obj) {
        let fields = parse_message(&bytes);
        let app_id = extract_app_id(method, &fields);
        if !is_valve_system_app(app_id) && is_namespace_app(app_id) {
            crate::log_debug(&format!("[Slot8] Cloud RPC bloqueada para a Valve: method={} app={}", method, app_id));
            if method == RPC_EXIT_SYNC {
                return orig(thisptr, method_name, request_msg);
            }
            return true;
        }
    }
    orig(thisptr, method_name, request_msg)
}
