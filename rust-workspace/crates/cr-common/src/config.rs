//! System configuration, port discovery, and directory detection utilities.

pub use cr_proto::cloud_messages::DEFAULT_SERVICE_PORT;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_service_port(steam_path: &str) -> u16 {
    let base = Path::new(steam_path);

    // 1. Tenta chrono_port.dat (gravado dinamicamente pelo daemon com a porta real)
    let port_dat = base.join("chrono_port.dat");
    if let Ok(content) = fs::read_to_string(&port_dat) {
        if let Ok(p) = content.trim().parse::<u16>() {
            if p > 0 {
                return p;
            }
        }
    }

    // 2. Tenta chronokeep.ini ou cr_clone_config.ini
    let candidates = ["chronokeep.ini", "cr_clone_config.ini"];
    for name in candidates {
        let ini_path = base.join(name);
        if let Ok(content) = fs::read_to_string(ini_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim().eq_ignore_ascii_case("port") {
                        if let Ok(p) = v.trim().parse::<u16>() {
                            if p > 0 {
                                return p;
                            }
                        }
                    }
                }
            }
        }
    }

    DEFAULT_SERVICE_PORT
}

pub fn detect_default_steam_dir() -> String {
    let candidates = [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
        r"D:\Steam",
        r"E:\Steam",
    ];
    for p in candidates {
        if Path::new(p).exists() {
            return p.to_string();
        }
    }
    r"C:\Program Files (x86)\Steam".to_string()
}

pub fn detect_default_vault_dir(exe_dir: Option<&Path>) -> PathBuf {
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(user_profile)
            .join("Saved Games")
            .join("ChronoKeepVault");
        return p;
    }
    exe_dir
        .map(|d| d.join("ChronoKeepVault"))
        .unwrap_or_else(|| PathBuf::from("ChronoKeepVault"))
}

pub fn is_gdrive_connected(steam_dir: &Path) -> bool {
    let token_file = steam_dir.join("chronokeep_gdrive.json");
    if token_file.exists() {
        return true;
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            if parent.join("chrono-daemon.exe").exists() && parent.join("chronokeep_gdrive.json").exists() {
                return true;
            }
        }
    }
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Ok(content) = std::fs::read_to_string(&ini_path) {
        for line in content.lines() {
            if let Some((k, v)) = line.trim().split_once('=') {
                if k.trim().eq_ignore_ascii_case("gdrive_refresh_token") && !v.trim().is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

pub fn load_debug_mode(steam_dir: &Path) -> bool {
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("debug") {
                    let val = v.trim();
                    return val.eq_ignore_ascii_case("true") || val == "1";
                }
            }
        }
    }
    false
}

pub fn save_debug_mode(steam_dir: &Path, enabled: bool) -> Result<(), std::io::Error> {
    let ini_path = steam_dir.join("chronokeep.ini");
    let content = fs::read_to_string(&ini_path).unwrap_or_default();
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut found = false;
    for line in &mut lines {
        let trimmed = line.trim();
        if let Some((k, _)) = trimmed.split_once('=') {
            if k.trim().eq_ignore_ascii_case("debug") {
                *line = format!("debug={}", enabled);
                found = true;
                break;
            }
        }
    }
    if !found {
        lines.push(format!("debug={}", enabled));
    }
    let updated = lines.join("\r\n") + "\r\n";
    fs::write(&ini_path, updated)
}

pub fn load_steam_api_key(steam_dir: &Path) -> Option<String> {
    let ini_path = steam_dir.join("chronokeep.ini");
    if let Ok(content) = fs::read_to_string(&ini_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("steam_api_key") || k.trim().eq_ignore_ascii_case("api_key") {
                    let val = v.trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn save_steam_api_key(steam_dir: &Path, key: &str) -> Result<(), std::io::Error> {
    let ini_path = steam_dir.join("chronokeep.ini");
    let content = fs::read_to_string(&ini_path).unwrap_or_default();
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut found = false;
    let clean_key = key.trim();
    for line in &mut lines {
        let trimmed = line.trim();
        if let Some((k, _)) = trimmed.split_once('=') {
            if k.trim().eq_ignore_ascii_case("steam_api_key") || k.trim().eq_ignore_ascii_case("api_key") {
                if clean_key.is_empty() {
                    *line = String::new();
                } else {
                    *line = format!("steam_api_key={}", clean_key);
                }
                found = true;
                break;
            }
        }
    }
    if !found && !clean_key.is_empty() {
        lines.push(format!("steam_api_key={}", clean_key));
    }
    lines.retain(|l| !l.trim().is_empty());
    let updated = lines.join("\r\n") + "\r\n";
    fs::write(&ini_path, updated)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppPlaytimeInfo {
    pub playtime_minutes: u64,
    pub last_played_ts: u64,
}

pub fn parse_localconfig_playtime(content: &str) -> HashMap<u32, AppPlaytimeInfo> {
    let mut apps: HashMap<u32, AppPlaytimeInfo> = HashMap::new();
    let mut current_app: Option<u32> = None;
    let mut depth: i32 = 0;
    let mut app_depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains('{') {
            depth += 1;
        }
        if trimmed.contains('}') {
            if current_app.is_some() && depth == app_depth {
                current_app = None;
            }
            depth = (depth - 1).max(0);
            continue;
        }

        // Split by quotes
        let parts: Vec<&str> = trimmed.split('"').collect();
        // format: "key" \t\t "val" -> parts: ["", key, " \t\t ", val, ""]
        if parts.len() >= 2 {
            let key = parts[1].trim();
            if parts.len() >= 4 {
                let val = parts[3].trim();
                if let Some(app_id) = current_app {
                    if key.eq_ignore_ascii_case("playtime") {
                        if let Ok(m) = val.parse::<u64>() {
                            apps.entry(app_id).or_default().playtime_minutes = m;
                        }
                    } else if key.eq_ignore_ascii_case("lastplayed") {
                        if let Ok(ts) = val.parse::<u64>() {
                            apps.entry(app_id).or_default().last_played_ts = ts;
                        }
                    }
                }
            } else if let Ok(app_id) = key.parse::<u32>() {
                if app_id > 0 {
                    current_app = Some(app_id);
                    app_depth = depth;
                    apps.entry(app_id).or_default();
                }
            }
        }
    }
    apps
}

pub fn load_all_local_playtime(
    steam_dir: &Path,
    account_id: Option<u32>,
) -> HashMap<u32, AppPlaytimeInfo> {
    let mut result: HashMap<u32, AppPlaytimeInfo> = HashMap::new();
    let userdata = steam_dir.join("userdata");
    if !userdata.exists() {
        return result;
    }

    let mut target_dirs = Vec::new();
    if let Some(acct) = account_id {
        if acct != 0 {
            target_dirs.push(userdata.join(acct.to_string()));
        }
    }

    if target_dirs.is_empty() {
        if let Ok(entries) = fs::read_dir(&userdata) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    target_dirs.push(p);
                }
            }
        }
    }

    for dir in target_dirs {
        let vdf = dir.join("config").join("localconfig.vdf");
        if let Ok(content) = fs::read_to_string(vdf) {
            let parsed = parse_localconfig_playtime(&content);
            for (app, info) in parsed {
                let entry = result.entry(app).or_default();
                entry.playtime_minutes = entry.playtime_minutes.max(info.playtime_minutes);
                entry.last_played_ts = entry.last_played_ts.max(info.last_played_ts);
            }
        }
    }
    result
}

pub fn check_daemon_health(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
    let req = format!(
        "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        port
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut resp = [0u8; 256];
    let Ok(n) = stream.read(&mut resp) else {
        return false;
    };
    let text = String::from_utf8_lossy(&resp[..n]);
    text.contains("200 OK") || text.starts_with("HTTP/1.1 200")
}

pub fn find_companion_binary(bin_name: &str) -> Option<PathBuf> {
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            let candidate = parent.join(bin_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join(bin_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let default_steam = PathBuf::from(detect_default_steam_dir());
    let candidate = default_steam.join(bin_name);
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_localconfig_playtime() {
        let vdf = r#"
"UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"2868840"
					{
						"LastPlayed"		"1793577105"
						"Playtime"		"78"
					}
					"1794680"
					{
						"LastPlayed"		"1792808878"
						"Playtime"		"64"
					}
				}
			}
		}
	}
}
"#;
        let map = parse_localconfig_playtime(vdf);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&2868840).unwrap().playtime_minutes, 78);
        assert_eq!(map.get(&2868840).unwrap().last_played_ts, 1793577105);
        assert_eq!(map.get(&1794680).unwrap().playtime_minutes, 64);
        assert_eq!(map.get(&1794680).unwrap().last_played_ts, 1792808878);
    }

    #[test]
    fn test_api_key_save_and_load() {
        let dir = std::env::temp_dir().join(format!("ck_test_key_{}", now_unix()));
        let _ = fs::create_dir_all(&dir);
        assert_eq!(load_steam_api_key(&dir), None);
        assert!(save_steam_api_key(&dir, "ABC123456789XYZ").is_ok());
        assert_eq!(load_steam_api_key(&dir), Some("ABC123456789XYZ".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    fn now_unix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}
