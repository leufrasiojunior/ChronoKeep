//! Google Drive OAuth2 1-Click browser authentication flow for `ChronoKeepSetup.exe`.

use crate::installer::refresh_status_label;
use crate::win32::*;
use cr_common::gdrive::{
    exchange_code_for_tokens, DEFAULT_GDRIVE_CLIENT_ID as GDRIVE_CLIENT_ID,
    DEFAULT_GDRIVE_SCOPE as GDRIVE_SCOPE,
};
use cr_common::http::{to_wide, url_decode, url_encode};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::ptr::null;
use std::time::Duration;

pub unsafe fn on_google_browser_login(hwnd: HWND) {
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
        "3. Conta Google Drive: [AGUARDANDO LOGIN NO NAVEGADOR...]",
    );

    let op = to_wide("open");
    let url_w = to_wide(&auth_url);
    ShellExecuteW(hwnd, op.as_ptr(), url_w.as_ptr(), null(), null(), SW_SHOWNORMAL);

    // Espera callback do navegador
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
