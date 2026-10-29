//! Win32 window procedure, navigation tabs, and control message routing.

use crate::installer::{on_install_and_save, populate_existing_config, refresh_status_label};
use crate::oauth_flow::on_google_browser_login;
use crate::orphans::{on_scan_orphans, on_select_all, on_upload_orphans, SCANNED_ORPHANS};
use crate::service_mgr::start_service_from_gui;
use crate::win32::*;
use cr_common::config::resolve_service_port;
use cr_common::http::to_wide;
use std::fs;
use std::path::Path;
use std::ptr::null;

pub const TAB_1_CONTROLS: &[usize] = &[
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

pub const TAB_2_CONTROLS: &[usize] = &[
    ID_LBL_ORPHANS_TITLE,
    ID_LIST_ORPHANS,
    ID_BTN_SCAN_ORPHANS,
    ID_BTN_SELECT_ALL,
    ID_BTN_DESELECT_ALL,
    ID_BTN_UPLOAD_ORPHANS,
    ID_LBL_ORPHANS_STATUS,
];

pub unsafe fn switch_tab(hwnd: HWND, tab_idx: usize) {
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

pub unsafe extern "system" fn wnd_proc(
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
                    let port = resolve_service_port(&steam_dir);
                    let ok = start_service_from_gui(Path::new(&steam_dir), port);
                    refresh_status_label(hwnd);
                    let actual_port = resolve_service_port(&steam_dir);
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
