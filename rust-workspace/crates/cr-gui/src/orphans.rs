#![allow(dead_code)]
//! Local save scanner and Google Drive batch upload orchestrator.

use crate::win32::*;
use cr_common::fs_utils::{collect_files_recursive, format_size};
use cr_common::gdrive::{
    clear_folder_cache, find_or_create_drive_folder, refresh_access_token, upload_file_multipart,
};
use cr_common::http::to_wide;
use cr_common::manifest::{parse_manifest, serialize_manifest, ManifestData, ManifestFileRecord};
use cr_proto::cloud_messages::split_root_token;
use cr_proto::sha1::sha1_digest;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct OrphanGameItem {
    pub account_id: u32,
    pub app_id: u32,
    pub file_count: usize,
    pub total_bytes: u64,
    pub source: String,
    pub files: Vec<(String, PathBuf)>,
}

pub static SCANNED_ORPHANS: Mutex<Vec<OrphanGameItem>> = Mutex::new(Vec::new());

fn upload_file_attempt_gui(
    token: &str,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
) -> bool {
    let Some(root_id) = find_or_create_drive_folder(token, "ChronoKeep", None) else {
        return false;
    };
    let Some(acct_id) = find_or_create_drive_folder(token, &account_id.to_string(), Some(&root_id)) else {
        return false;
    };
    let Some(mut parent_id) = find_or_create_drive_folder(token, &app_id.to_string(), Some(&acct_id)) else {
        return false;
    };

    let normalized = rel_filename.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return false;
    }
    for sub in &parts[..parts.len() - 1] {
        let Some(next_id) = find_or_create_drive_folder(token, sub, Some(&parent_id)) else {
            return false;
        };
        parent_id = next_id;
    }
    let leaf_name = parts[parts.len() - 1];

    upload_file_multipart(token, &parent_id, leaf_name, data)
}

fn sync_orphan_file_to_drive_with_retry(
    token: &str,
    account_id: u32,
    app_id: u32,
    rel_filename: &str,
    data: &[u8],
) -> bool {
    if upload_file_attempt_gui(token, account_id, app_id, rel_filename, data) {
        return true;
    }
    clear_folder_cache();
    upload_file_attempt_gui(token, account_id, app_id, rel_filename, data)
}

pub unsafe fn on_scan_orphans(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);
    let steam_dir = Path::new(&steam_dir_str);
    let vault_dir = Path::new(&vault_dir_str);

    let h_status = GetDlgItem(hwnd, ID_LBL_ORPHANS_STATUS as i32);
    if h_status != 0 {
        SetWindowTextW(h_status, to_wide("Escaneando pastas de saves locais no PC...").as_ptr());
    }

    let mut map: BTreeMap<(u32, u32), OrphanGameItem> = BTreeMap::new();

    // 1. Escanear Steam userdata
    let userdata_dir = steam_dir.join("userdata");
    if userdata_dir.exists() {
        if let Ok(acct_entries) = fs::read_dir(&userdata_dir) {
            for acct_entry in acct_entries.flatten() {
                let acct_path = acct_entry.path();
                if !acct_path.is_dir() {
                    continue;
                }
                let Some(acct_name) = acct_path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Ok(acct_id) = acct_name.parse::<u32>() else {
                    continue;
                };
                if acct_id == 0 {
                    continue;
                }

                if let Ok(app_entries) = fs::read_dir(&acct_path) {
                    for app_entry in app_entries.flatten() {
                        let app_path = app_entry.path();
                        if !app_path.is_dir() {
                            continue;
                        }
                        let Some(app_name) = app_path.file_name().and_then(|n| n.to_str()) else {
                            continue;
                        };
                        let Ok(app_id) = app_name.parse::<u32>() else {
                            continue;
                        };
                        if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
                            continue;
                        }

                        let remote_dir = app_path.join("remote");
                        if !remote_dir.exists() {
                            continue;
                        }

                        let mut found = Vec::new();
                        collect_files_recursive(&remote_dir, &remote_dir, &mut found);
                        let valid_files: Vec<(String, PathBuf)> = found
                            .into_iter()
                            .filter(|(rel, _)| {
                                let l = rel.to_lowercase();
                                !l.starts_with('.') && !l.ends_with(".tmp") && !l.contains(".cloudredirect")
                            })
                            .collect();

                        if !valid_files.is_empty() {
                            let entry = map.entry((acct_id, app_id)).or_insert_with(|| OrphanGameItem {
                                account_id: acct_id,
                                app_id,
                                file_count: 0,
                                total_bytes: 0,
                                source: "userdata".to_string(),
                                files: Vec::new(),
                            });
                            for (rel, path) in valid_files {
                                if !entry.files.iter().any(|(r, _)| r == &rel) {
                                    if let Ok(meta) = fs::metadata(&path) {
                                        entry.total_bytes += meta.len();
                                    }
                                    entry.files.push((rel, path));
                                }
                            }
                            entry.file_count = entry.files.len();
                        }
                    }
                }
            }
        }
    }

    // 2. Escanear Vault local
    if vault_dir.exists() {
        if let Ok(acct_entries) = fs::read_dir(vault_dir) {
            for acct_entry in acct_entries.flatten() {
                let acct_path = acct_entry.path();
                if !acct_path.is_dir() {
                    continue;
                }
                let Some(acct_name) = acct_path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Ok(acct_id) = acct_name.parse::<u32>() else {
                    continue;
                };

                if let Ok(app_entries) = fs::read_dir(&acct_path) {
                    for app_entry in app_entries.flatten() {
                        let app_path = app_entry.path();
                        if !app_path.is_dir() {
                            continue;
                        }
                        let Some(app_name) = app_path.file_name().and_then(|n| n.to_str()) else {
                            continue;
                        };
                        let Ok(app_id) = app_name.parse::<u32>() else {
                            continue;
                        };
                        if app_id == 0 || app_id == 7 || app_id == 760 || app_id == 241100 {
                            continue;
                        }

                        let files_dir = app_path.join("files");
                        if !files_dir.exists() {
                            continue;
                        }

                        let mut found = Vec::new();
                        collect_files_recursive(&files_dir, &files_dir, &mut found);
                        let valid_files: Vec<(String, PathBuf)> = found
                            .into_iter()
                            .filter(|(rel, _)| {
                                let l = rel.to_lowercase();
                                !l.starts_with('.') && !l.ends_with(".tmp") && !l.contains(".cloudredirect") && l != "manifest.tsv"
                            })
                            .collect();

                        if !valid_files.is_empty() {
                            let entry = map.entry((acct_id, app_id)).or_insert_with(|| OrphanGameItem {
                                account_id: acct_id,
                                app_id,
                                file_count: 0,
                                total_bytes: 0,
                                source: "Vault".to_string(),
                                files: Vec::new(),
                            });
                            for (rel, path) in valid_files {
                                if !entry.files.iter().any(|(r, _)| r == &rel) {
                                    if let Ok(meta) = fs::metadata(&path) {
                                        entry.total_bytes += meta.len();
                                    }
                                    entry.files.push((rel, path));
                                }
                            }
                            entry.file_count = entry.files.len();
                        }
                    }
                }
            }
        }
    }

    let items: Vec<OrphanGameItem> = map.into_values().collect();
    let count = items.len();
    if let Ok(mut guard) = SCANNED_ORPHANS.lock() {
        *guard = items.clone();
    }

    let h_list = GetDlgItem(hwnd, ID_LIST_ORPHANS as i32);
    if h_list != 0 {
        SendMessageW(h_list, LB_RESETCONTENT, 0, 0);
        for item in &items {
            let line = format!(
                "AppID: {:<8} | {:>2} arquivo(s) ({:>8}) | Conta: {:<9} | Local: {}",
                item.app_id,
                item.file_count,
                format_size(item.total_bytes),
                item.account_id,
                item.source
            );
            SendMessageW(h_list, LB_ADDSTRING, 0, to_wide(&line).as_ptr() as LPARAM);
        }
        SendMessageW(h_list, LB_SETSEL, 1, -1);
    }

    if h_status != 0 {
        let msg = format!(
            "{} jogos com saves locais encontrados no PC. Todos selecionados para envio.",
            count
        );
        SetWindowTextW(h_status, to_wide(&msg).as_ptr());
    }
}

pub unsafe fn on_select_all(hwnd: HWND, select: bool) {
    let h_list = GetDlgItem(hwnd, ID_LIST_ORPHANS as i32);
    if h_list != 0 {
        SendMessageW(h_list, LB_SETSEL, if select { 1 } else { 0 }, -1);
    }
    let h_status = GetDlgItem(hwnd, ID_LBL_ORPHANS_STATUS as i32);
    if h_status != 0 {
        let count = SCANNED_ORPHANS.lock().map(|g| g.len()).unwrap_or(0);
        let msg = if select {
            format!("Todos os {} jogos foram marcados.", count)
        } else {
            "Nenhum jogo selecionado. Clique na lista para selecionar ou em 'Marcar Todos'.".to_string()
        };
        SetWindowTextW(h_status, to_wide(&msg).as_ptr());
    }
}

pub unsafe fn on_upload_orphans(hwnd: HWND) {
    let steam_dir_str = get_control_text(hwnd, ID_EDIT_STEAM);
    let vault_dir_str = get_control_text(hwnd, ID_EDIT_VAULT);
    let steam_dir = PathBuf::from(&steam_dir_str);
    let vault_dir = PathBuf::from(&vault_dir_str);

    let mut token_path = steam_dir.join("chronokeep_gdrive.json");
    if !token_path.exists() {
        if let Ok(cur) = env::current_exe() {
            if let Some(parent) = cur.parent() {
                let cand = parent.join("chronokeep_gdrive.json");
                if cand.exists() {
                    token_path = cand;
                }
            }
        }
    }

    if !token_path.exists() {
        let msg = to_wide(
            "Conta Google Drive nao conectada!\r\n\r\n\
             Por favor, volte para a aba '1. Configuracao & Instalacao' e conecte a sua conta Google antes de enviar os saves.",
        );
        let cap = to_wide("Google Drive Desconectado");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONWARNING);
        return;
    }

    let h_list = GetDlgItem(hwnd, ID_LIST_ORPHANS as i32);
    if h_list == 0 {
        return;
    }
    let sel_count = SendMessageW(h_list, LB_GETSELCOUNT, 0, 0);
    if sel_count <= 0 {
        let msg = to_wide("Nenhum jogo selecionado na lista para envio.");
        let cap = to_wide("Aviso");
        MessageBoxW(hwnd, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONWARNING);
        return;
    }

    let mut indices: Vec<i32> = vec![0; sel_count as usize];
    SendMessageW(h_list, LB_GETSELITEMS, sel_count as WPARAM, indices.as_mut_ptr() as LPARAM);

    let all_scanned = match SCANNED_ORPHANS.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };

    let mut selected_games = Vec::new();
    for &idx in &indices {
        if let Some(item) = all_scanned.get(idx as usize) {
            selected_games.push(item.clone());
        }
    }

    let h_status = GetDlgItem(hwnd, ID_LBL_ORPHANS_STATUS as i32);
    if h_status != 0 {
        SetWindowTextW(h_status, to_wide("Iniciando sincronizacao com o Google Drive...").as_ptr());
    }

    let hwnd_val = hwnd;
    std::thread::spawn(move || {
        let Some(token) = refresh_access_token(&token_path) else {
            unsafe {
                if h_status != 0 {
                    SetWindowTextW(h_status, to_wide("Erro ao renovar token de acesso do Google Drive.").as_ptr());
                }
                let msg = to_wide("Falha ao autenticar com o Google Drive. Refaca o login na Aba 1.");
                let cap = to_wide("Erro Google Drive");
                MessageBoxW(hwnd_val, msg.as_ptr(), cap.as_ptr(), MB_OK | MB_ICONERROR);
            }
            return;
        };

        let total_games = selected_games.len();
        let mut total_files_uploaded = 0;
        let mut success_games = 0;

        for (i, game) in selected_games.iter().enumerate() {
            unsafe {
                if h_status != 0 {
                    let msg = format!("Enviando AppID {} ({}/{})...", game.app_id, i + 1, total_games);
                    SetWindowTextW(h_status, to_wide(&msg).as_ptr());
                }
            }

            let manifest_file = vault_dir
                .join(game.account_id.to_string())
                .join(game.app_id.to_string())
                .join("manifest.tsv");
            let mut manifest_data = if let Ok(prev) = fs::read_to_string(&manifest_file) {
                parse_manifest(&prev)
            } else {
                ManifestData::default()
            };
            if manifest_data.change_number == 0 {
                manifest_data.change_number = 1;
            }

            for (rel_name, full_path) in &game.files {
                if let Ok(bytes) = fs::read(full_path) {
                    let ok = sync_orphan_file_to_drive_with_retry(
                        &token,
                        game.account_id,
                        game.app_id,
                        rel_name,
                        &bytes,
                    );
                    if ok {
                        total_files_uploaded += 1;
                    }

                    // Salva copia local no Vault
                    let vault_file = vault_dir
                        .join(game.account_id.to_string())
                        .join(game.app_id.to_string())
                        .join("files")
                        .join(rel_name);
                    if let Some(parent) = vault_file.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&vault_file, &bytes);

                    let sha = sha1_digest(&bytes);
                    let file_ts = fs::metadata(full_path)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let (root_token, clean_filename) = split_root_token(rel_name);
                    manifest_data.files.insert(
                        clean_filename.clone(),
                        ManifestFileRecord {
                            filename: clean_filename,
                            root_token,
                            sha1: sha,
                            timestamp: file_ts,
                            raw_size: bytes.len() as u64,
                            platforms_to_sync: 0xFFFF_FFFF,
                        },
                    );
                }
            }

            let manifest_content = serialize_manifest(&manifest_data);
            if let Some(parent) = manifest_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&manifest_file, manifest_content.as_bytes());

            let _ = sync_orphan_file_to_drive_with_retry(
                &token,
                game.account_id,
                game.app_id,
                "manifest.tsv",
                manifest_content.as_bytes(),
            );

            success_games += 1;
        }

        unsafe {
            let done_text = format!(
                "Concluido com sucesso! {} jogos ({} arquivos) enviados ao Google Drive.",
                success_games, total_files_uploaded
            );
            if h_status != 0 {
                SetWindowTextW(h_status, to_wide(&done_text).as_ptr());
            }
            let cap = to_wide("Sincronizacao Concluida");
            MessageBoxW(hwnd_val, to_wide(&done_text).as_ptr(), cap.as_ptr(), MB_OK | MB_ICONINFORMATION);
        }
    });
}

pub fn find_or_create_gdrive_token_file(steam_dir: &Path) -> Option<PathBuf> {
    let cand1 = steam_dir.join("chronokeep_gdrive.json");
    if cand1.exists() {
        return Some(cand1);
    }
    if let Ok(cur) = env::current_exe() {
        if let Some(parent) = cur.parent() {
            let cand2 = parent.join("chronokeep_gdrive.json");
            if cand2.exists() {
                return Some(cand2);
            }
        }
    }
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for l in content.lines() {
            if let Some((k, v)) = l.trim().split_once('=') {
                if k.trim().eq_ignore_ascii_case("gdrive_refresh_token") && !v.trim().is_empty() {
                    let json = format!(r#"{{"refresh_token":"{}"}}"#, v.trim());
                    let _ = fs::write(&cand1, &json);
                    return Some(cand1);
                }
            }
        }
    }
    None
}

fn load_max_upload_mb(steam_dir: &Path) -> u64 {
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            if let Some((k, v)) = line.trim().split_once('=') {
                if k.trim().eq_ignore_ascii_case("max_upload_mb") {
                    if let Ok(mb) = v.trim().parse::<u64>() {
                        if mb > 0 {
                            return mb;
                        }
                    }
                }
            }
        }
    }
    512
}

pub fn upload_single_game_save(
    steam_dir: &Path,
    vault_dir: &Path,
    app_id: u32,
    account_id: u32,
) -> Result<usize, String> {
    let token_path = find_or_create_gdrive_token_file(steam_dir)
        .ok_or_else(|| "Conta Google Drive não conectada. Conecte sua conta antes de sincronizar.".to_string())?;

    let token = refresh_access_token(&token_path)
        .ok_or_else(|| "Falha ao renovar token de acesso do Google Drive. Refaça o login nas configurações.".to_string())?;

    let mut files_to_sync: Vec<(String, PathBuf)> = Vec::new();

    let userdata_app = steam_dir
        .join("userdata")
        .join(account_id.to_string())
        .join(app_id.to_string());

    if userdata_app.exists() {
        let remote_dir = userdata_app.join("remote");
        if remote_dir.exists() {
            let mut found = Vec::new();
            collect_files_recursive(&remote_dir, &remote_dir, &mut found);
            for (rel, p) in found {
                let norm = format!("%WinMyDocuments%/{}", rel.replace('\\', "/"));
                files_to_sync.push((norm, p));
            }
        }
        let files_dir = userdata_app.join("files");
        if files_dir.exists() {
            let mut found = Vec::new();
            collect_files_recursive(&files_dir, &files_dir, &mut found);
            for (rel, p) in found {
                let norm = rel.replace('\\', "/");
                if !files_to_sync.iter().any(|(r, _)| r == &norm) {
                    files_to_sync.push((norm, p));
                }
            }
        }
    }

    let vault_app_files = vault_dir
        .join(account_id.to_string())
        .join(app_id.to_string())
        .join("files");
    if vault_app_files.exists() {
        let mut found = Vec::new();
        collect_files_recursive(&vault_app_files, &vault_app_files, &mut found);
        for (rel, p) in found {
            let norm = rel.replace('\\', "/");
            if !files_to_sync.iter().any(|(r, _)| r == &norm) {
                files_to_sync.push((norm, p));
            }
        }
    }

    let valid_files: Vec<(String, PathBuf)> = files_to_sync
        .into_iter()
        .filter(|(rel, _)| {
            let l = rel.to_lowercase();
            !l.starts_with('.') && !l.ends_with(".tmp") && !l.contains(".cloudredirect") && l != "manifest.tsv"
        })
        .collect();

    if valid_files.is_empty() {
        return Ok(0);
    }

    let manifest_file = vault_dir
        .join(account_id.to_string())
        .join(app_id.to_string())
        .join("manifest.tsv");

    let mut manifest_data = if let Ok(prev) = fs::read_to_string(&manifest_file) {
        parse_manifest(&prev)
    } else {
        ManifestData::default()
    };
    if manifest_data.change_number == 0 {
        manifest_data.change_number = 1;
    } else {
        manifest_data.change_number += 1;
    }

    let max_mb = load_max_upload_mb(steam_dir);
    let max_bytes = max_mb * 1024 * 1024;
    let mut uploaded_count = 0;
    let mut oversized_count = 0;

    for (rel_name, full_path) in &valid_files {
        if let Ok(bytes) = fs::read(full_path) {
            // Guarda sempre no Cofre local para preservação do save
            let vault_dest = vault_app_files.join(rel_name);
            if let Some(parent) = vault_dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&vault_dest, &bytes);

            // Verifica o limite de upload configurado na UI para envio manual
            if (bytes.len() as u64) > max_bytes {
                oversized_count += 1;
                continue;
            }

            let ok = sync_orphan_file_to_drive_with_retry(&token, account_id, app_id, rel_name, &bytes);
            if ok {
                uploaded_count += 1;

                let sha = sha1_digest(&bytes);
                let file_ts = fs::metadata(full_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let (root_token, clean_filename) = split_root_token(rel_name);
                manifest_data.files.insert(
                    clean_filename.clone(),
                    ManifestFileRecord {
                        filename: clean_filename,
                        root_token,
                        sha1: sha,
                        timestamp: file_ts,
                        raw_size: bytes.len() as u64,
                        platforms_to_sync: 0xFFFF_FFFF,
                    },
                );
            }
        }
    }

    if uploaded_count == 0 {
        if oversized_count > 0 {
            return Err(format!(
                "Nenhum arquivo enviado à nuvem: todos os {} arquivo(s) do AppID {} excedem o limite de upload manual ({} MB) e foram preservados apenas no Cofre local.",
                valid_files.len(),
                app_id,
                max_mb
            ));
        } else {
            return Err(format!(
                "Falha ao enviar os {} arquivo(s) do AppID {} para o Google Drive. Verifique a conexão com a internet ou permissões da conta.",
                valid_files.len(),
                app_id
            ));
        }
    }

    let manifest_content = serialize_manifest(&manifest_data);

    let manifest_ok = sync_orphan_file_to_drive_with_retry(
        &token,
        account_id,
        app_id,
        "manifest.tsv",
        manifest_content.as_bytes(),
    );
    if !manifest_ok {
        return Err(format!(
            "Os arquivos de save do AppID {} foram enviados, mas houve falha ao sincronizar o índice 'manifest.tsv' com o Google Drive.",
            app_id
        ));
    }

    if let Some(parent) = manifest_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&manifest_file, manifest_content.as_bytes());

    let target_upload_count = valid_files.len().saturating_sub(oversized_count);
    if uploaded_count < target_upload_count {
        return Err(format!(
            "Sincronização parcial: apenas {} de {} arquivo(s) do AppID {} foram enviados ao Google Drive com sucesso.",
            uploaded_count,
            target_upload_count,
            app_id
        ));
    }

    Ok(uploaded_count)
}
