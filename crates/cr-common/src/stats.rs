//! Statistics and achievements synchronization for managed and non-Steam games.
//! Handles serialization, monotonic merge, Steam native BKV export/import,
//! localconfig.vdf updating, and librarycache.json reconciliation.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AchievementUnlockRecord {
    pub id: String,
    #[serde(default)]
    pub stat_id: u32,
    #[serde(default)]
    pub bit: u32,
    pub unlocked: bool,
    #[serde(default)]
    pub unlock_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StatRecord {
    pub stat_id: u32,
    pub value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameStatsData {
    pub app_id: u32,
    pub account_id: u32,
    #[serde(default)]
    pub playtime_minutes: u64,
    #[serde(default)]
    pub playtime_windows: u64,
    #[serde(default)]
    pub last_played_ts: u64,
    #[serde(default)]
    pub stats: Vec<StatRecord>,
    #[serde(default)]
    pub achievements: Vec<AchievementUnlockRecord>,
    #[serde(default)]
    pub schema_b64: Option<String>,
    #[serde(default)]
    pub raw_user_game_stats_b64: Option<String>,
    #[serde(default)]
    pub librarycache_achievements_b64: Option<String>,
    #[serde(default)]
    pub updated_at: u64,
}

impl GameStatsData {
    pub fn merge_with(&mut self, other: &GameStatsData) {
        self.playtime_minutes = self.playtime_minutes.max(other.playtime_minutes);
        self.playtime_windows = self.playtime_windows.max(other.playtime_windows);
        self.last_played_ts = self.last_played_ts.max(other.last_played_ts);
        self.updated_at = self.updated_at.max(other.updated_at);

        // Merge achievements cumulatively: once unlocked, never locked
        let mut ach_map: HashMap<String, AchievementUnlockRecord> = HashMap::new();
        for a in &self.achievements {
            ach_map.insert(a.id.clone(), a.clone());
        }
        for a in &other.achievements {
            match ach_map.get_mut(&a.id) {
                Some(existing) => {
                    if a.unlocked && !existing.unlocked {
                        existing.unlocked = true;
                        existing.unlock_time = a.unlock_time;
                    } else if a.unlocked && existing.unlocked {
                        if existing.unlock_time == 0 {
                            existing.unlock_time = a.unlock_time;
                        } else if a.unlock_time > 0 {
                            existing.unlock_time = existing.unlock_time.min(a.unlock_time);
                        }
                    }
                    if existing.stat_id == 0 && a.stat_id > 0 {
                        existing.stat_id = a.stat_id;
                    }
                    if existing.bit == 0 && a.bit > 0 {
                        existing.bit = a.bit;
                    }
                }
                None => {
                    ach_map.insert(a.id.clone(), a.clone());
                }
            }
        }
        self.achievements = ach_map.into_values().collect();
        self.achievements.sort_by(|a, b| a.id.cmp(&b.id));

        // Merge stats (max value per stat_id)
        let mut stat_map: HashMap<u32, u32> = HashMap::new();
        for s in &self.stats {
            stat_map.insert(s.stat_id, s.value);
        }
        for s in &other.stats {
            let entry = stat_map.entry(s.stat_id).or_insert(0);
            *entry = (*entry).max(s.value);
        }
        self.stats = stat_map.into_iter().map(|(stat_id, value)| StatRecord { stat_id, value }).collect();
        self.stats.sort_by_key(|s| s.stat_id);

        if self.schema_b64.is_none() && other.schema_b64.is_some() {
            self.schema_b64 = other.schema_b64.clone();
        }
        if self.raw_user_game_stats_b64.is_none() && other.raw_user_game_stats_b64.is_some() {
            self.raw_user_game_stats_b64 = other.raw_user_game_stats_b64.clone();
        }
        if self.librarycache_achievements_b64.is_none() && other.librarycache_achievements_b64.is_some() {
            self.librarycache_achievements_b64 = other.librarycache_achievements_b64.clone();
        }
    }
}

pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) & 0x3F] as char);
        out.push(TABLE[(triple >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(triple >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[triple & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        if b.is_ascii_whitespace() || b == b'=' {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Valve Binary KeyValues (BKV / KV1) Writer
pub struct BkvWriter {
    buf: Vec<u8>,
}

impl BkvWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn begin_section(&mut self, name: &str) {
        self.buf.push(0x00);
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.push(0x00);
    }

    pub fn end_section(&mut self) {
        self.buf.push(0x08);
    }

    pub fn write_int(&mut self, name: &str, val: u32) {
        self.buf.push(0x02);
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.push(0x00);
        self.buf.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_str(&mut self, name: &str, val: &str) {
        self.buf.push(0x01);
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.push(0x00);
        self.buf.extend_from_slice(val.as_bytes());
        self.buf.push(0x00);
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// Generates a binary `UserGameStats_<account>_<app>.bin` formatted for Steam's native engine.
pub fn export_user_game_stats_bin(stats: &GameStatsData) -> Vec<u8> {
    let mut bkv = BkvWriter::new();
    bkv.begin_section("cache");
    bkv.write_int("crc", 0);
    bkv.write_int("PendingChanges", 0);

    let mut stat_ach_map: HashMap<u32, Vec<&AchievementUnlockRecord>> = HashMap::new();
    for a in &stats.achievements {
        let sid = if a.stat_id != 0 { a.stat_id } else { 1 };
        stat_ach_map.entry(sid).or_default().push(a);
    }

    let mut emitted_stats = HashSet::new();
    for s in &stats.stats {
        emitted_stats.insert(s.stat_id);
        let stat_name = s.stat_id.to_string();
        bkv.begin_section(&stat_name);
        bkv.write_int("data", s.value);

        if let Some(achs) = stat_ach_map.get(&s.stat_id) {
            bkv.begin_section("AchievementTimes");
            for a in achs {
                if a.unlocked && a.unlock_time > 0 {
                    let bit_name = a.bit.to_string();
                    bkv.write_int(&bit_name, a.unlock_time as u32);
                }
            }
            bkv.end_section();
        }
        bkv.end_section();
    }

    for (&stat_id, achs) in &stat_ach_map {
        if emitted_stats.contains(&stat_id) {
            continue;
        }
        let stat_name = stat_id.to_string();
        bkv.begin_section(&stat_name);
        let mut bits = 0u32;
        for a in achs {
            if a.unlocked && a.bit < 32 {
                bits |= 1 << a.bit;
            }
        }
        bkv.write_int("data", bits);
        bkv.begin_section("AchievementTimes");
        for a in achs {
            if a.unlocked && a.unlock_time > 0 {
                let bit_name = a.bit.to_string();
                bkv.write_int(&bit_name, a.unlock_time as u32);
            }
        }
        bkv.end_section();
        bkv.end_section();
    }

    bkv.end_section(); // cache
    bkv.end_section(); // root terminator (0x08)
    bkv.into_bytes()
}

/// Updates or creates the AppID playtime entry inside Steam's `localconfig.vdf`.
pub fn update_localconfig_playtime(
    content: &str,
    app_id: u32,
    playtime_minutes: u64,
    last_played_ts: u64,
) -> String {
    let app_str = app_id.to_string();

    // Look for existing app block: "<app_id>"
    if let Some(app_idx) = find_vdf_key_case_insensitive(content, &app_str) {
        // App exists, update its keys inside its block
        if let Some(open_brace) = content[app_idx..].find('{') {
            let block_start = app_idx + open_brace;
            if let Some(block_len) = find_matching_brace(&content[block_start..]) {
                let block_end = block_start + block_len;
                let app_block = &content[block_start + 1..block_end - 1];

                let mut updated_block = app_block.to_string();

                // 1. Playtime
                updated_block = update_or_append_vdf_prop(&updated_block, "Playtime", &playtime_minutes.to_string());
                // 2. PlaytimeWindows
                updated_block = update_or_append_vdf_prop(&updated_block, "PlaytimeWindows", &playtime_minutes.to_string());
                // 3. LastPlayed
                if last_played_ts > 0 {
                    updated_block = update_or_append_vdf_prop(&updated_block, "LastPlayed", &last_played_ts.to_string());
                }

                let mut result = String::with_capacity(content.len() + 128);
                result.push_str(&content[..block_start + 1]);
                result.push_str(&updated_block);
                result.push_str(&content[block_end - 1..]);
                return result;
            }
        }
    }

    // If app doesn't exist, insert into apps / Apps section
    let apps_key = if let Some(idx) = find_vdf_key_case_insensitive(content, "apps") {
        Some((idx, "apps"))
    } else if let Some(idx) = find_vdf_key_case_insensitive(content, "Apps") {
        Some((idx, "Apps"))
    } else {
        None
    };

    if let Some((apps_idx, _)) = apps_key {
        if let Some(open_brace) = content[apps_idx..].find('{') {
            let insert_pos = apps_idx + open_brace + 1;
            let mut result = String::with_capacity(content.len() + 256);
            result.push_str(&content[..insert_pos]);
            result.push_str(&format!(
                "\n\t\t\t\t\t\"{}\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"Playtime\"\t\t\"{}\"\n\t\t\t\t\t\t\"PlaytimeWindows\"\t\t\"{}\"\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"{}\"\n\t\t\t\t\t}}",
                app_id, playtime_minutes, playtime_minutes, last_played_ts
            ));
            result.push_str(&content[insert_pos..]);
            return result;
        }
    }

    // Fallback: create complete default structure if file was minimal or empty
    format!(
        "\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n\t\t\t\t\t\"{}\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"Playtime\"\t\t\"{}\"\n\t\t\t\t\t\t\"PlaytimeWindows\"\t\t\"{}\"\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"{}\"\n\t\t\t\t\t}}\n\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n}}\n",
        app_id, playtime_minutes, playtime_minutes, last_played_ts
    )
}

fn update_or_append_vdf_prop(block: &str, prop_name: &str, new_val: &str) -> String {
    if let Some(idx) = find_vdf_key_case_insensitive(block, prop_name) {
        // Find quotes for value after prop_name
        let after_key = &block[idx + prop_name.len() + 2..]; // skip past key quotes
        if let Some(first_quote) = after_key.find('"') {
            let val_start = idx + prop_name.len() + 2 + first_quote + 1;
            if let Some(second_quote) = block[val_start..].find('"') {
                let val_end = val_start + second_quote;
                let old_val_str = &block[val_start..val_end];
                let old_num: u64 = old_val_str.parse().unwrap_or(0);
                let new_num: u64 = new_val.parse().unwrap_or(0);
                let final_val = old_num.max(new_num).to_string();

                let mut out = String::with_capacity(block.len() + 16);
                out.push_str(&block[..val_start]);
                out.push_str(&final_val);
                out.push_str(&block[val_end..]);
                return out;
            }
        }
    }
    // Property missing: append it
    format!("{}\n\t\t\t\t\t\t\"{}\"\t\t\"{}\"", block.trim_end(), prop_name, new_val)
}

fn find_vdf_key_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let lower_h = haystack.to_lowercase();
    let lower_n = format!("\"{}\"", needle.to_lowercase());
    lower_h.find(&lower_n)
}

fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Updates or creates the `Steam/userdata/<account_id>/config/librarycache/<app_id>.json` file.
pub fn update_librarycache_json(
    json_content: &str,
    achievements: &[AchievementUnlockRecord],
) -> Option<String> {
    let Ok(mut val) = serde_json::from_str::<serde_json::Value>(json_content) else {
        return None;
    };

    let Some(arr) = val.as_array_mut() else {
        return None;
    };

    let ach_map: HashMap<&str, &AchievementUnlockRecord> = achievements
        .iter()
        .filter(|a| a.unlocked)
        .map(|a| (a.id.as_str(), a))
        .collect();

    let mut updated_any = false;

    for item in arr {
        let Some(sec) = item.as_array_mut() else { continue };
        if sec.len() >= 2 && sec[0].as_str() == Some("achievements") {
            if let Some(data) = sec[1].get_mut("data") {
                let mut unlocked_count = 0usize;

                let lists = ["vecHighlight", "vecAchievedHidden", "vecUnachieved"];
                for list_name in lists {
                    if let Some(list) = data.get_mut(list_name).and_then(|v| v.as_array_mut()) {
                        for entry in list {
                            if let Some(id_str) = entry.get("strID").and_then(|v| v.as_str()) {
                                if let Some(rec) = ach_map.get(id_str) {
                                    entry["bAchieved"] = serde_json::Value::Bool(true);
                                    if rec.unlock_time > 0 {
                                        entry["rtUnlocked"] = serde_json::Value::Number(rec.unlock_time.into());
                                    }
                                    updated_any = true;
                                }
                            }
                            if entry.get("bAchieved").and_then(|v| v.as_bool()) == Some(true) {
                                unlocked_count += 1;
                            }
                        }
                    }
                }
                data["nAchieved"] = serde_json::Value::Number(unlocked_count.into());
            }
        }
    }

    if updated_any {
        serde_json::to_string(&val).ok()
    } else {
        None
    }
}

/// Captures all local game stats (playtime, achievements, schema, raw bins) for an app from Steam.
pub fn capture_local_game_stats(
    steam_path: &Path,
    account_id: u32,
    app_id: u32,
) -> Option<GameStatsData> {
    let mut data = GameStatsData {
        app_id,
        account_id,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        ..Default::default()
    };

    // 1. Playtime from localconfig.vdf
    let userdata_acct = steam_path.join("userdata").join(account_id.to_string());
    let lc_path = userdata_acct.join("config").join("localconfig.vdf");
    if let Ok(content) = fs::read_to_string(&lc_path) {
        let playtime_map = crate::config::parse_localconfig_playtime(&content);
        if let Some(info) = playtime_map.get(&app_id) {
            data.playtime_minutes = info.playtime_minutes;
            data.playtime_windows = info.playtime_minutes;
            data.last_played_ts = info.last_played_ts;
        }
    }

    // 2. Achievements from UserGameStats_<account_id>_<app_id>.bin
    let stats_bin_path = steam_path
        .join("appcache")
        .join("stats")
        .join(format!("UserGameStats_{}_{}.bin", account_id, app_id));
    if let Ok(bytes) = fs::read(&stats_bin_path) {
        data.raw_user_game_stats_b64 = Some(base64_encode(&bytes));
    }

    // 3. Schema from UserGameStatsSchema_<app_id>.bin
    let schema_bin_path = steam_path
        .join("appcache")
        .join("stats")
        .join(format!("UserGameStatsSchema_{}.bin", app_id));
    if let Ok(bytes) = fs::read(&schema_bin_path) {
        data.schema_b64 = Some(base64_encode(&bytes));
    }

    // 4. Library cache JSON from userdata/<account_id>/config/librarycache/<app_id>.json
    let lc_json_path = userdata_acct.join("config").join("librarycache").join(format!("{}.json", app_id));
    if let Ok(content) = fs::read_to_string(&lc_json_path) {
        data.librarycache_achievements_b64 = Some(base64_encode(content.as_bytes()));

        // Parse achievements from json
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(arr) = val.as_array() {
                for item in arr {
                    if let Some(sec) = item.as_array() {
                        if sec.len() >= 2 && sec[0].as_str() == Some("achievements") {
                            if let Some(d) = sec[1].get("data") {
                                for list_name in ["vecHighlight", "vecAchievedHidden", "vecUnachieved"] {
                                    if let Some(list) = d.get(list_name).and_then(|v| v.as_array()) {
                                        for entry in list {
                                            if let Some(id) = entry.get("strID").and_then(|v| v.as_str()) {
                                                let unlocked = entry.get("bAchieved").and_then(|v| v.as_bool()).unwrap_or(false);
                                                let ts = entry.get("rtUnlocked").and_then(|v| v.as_u64()).unwrap_or(0);
                                                data.achievements.push(AchievementUnlockRecord {
                                                    id: id.to_string(),
                                                    stat_id: 1,
                                                    bit: 0,
                                                    unlocked,
                                                    unlock_time: ts,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if data.playtime_minutes == 0
        && data.last_played_ts == 0
        && data.achievements.is_empty()
        && data.raw_user_game_stats_b64.is_none()
    {
        return None;
    }

    Some(data)
}

/// Restores game stats (playtime, achievements, native BKV, and library cache) to the local Steam install.
pub fn restore_local_game_stats(
    steam_path: &Path,
    account_id: u32,
    app_id: u32,
    stats: &GameStatsData,
) -> Result<(), String> {
    let userdata_acct = steam_path.join("userdata").join(account_id.to_string());
    let stats_dir = steam_path.join("appcache").join("stats");
    let _ = fs::create_dir_all(&stats_dir);

    // 1. Restore localconfig.vdf (Playtime & LastPlayed)
    if stats.playtime_minutes > 0 || stats.last_played_ts > 0 {
        let lc_path = userdata_acct.join("config").join("localconfig.vdf");
        if let Some(parent) = lc_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let current_content = fs::read_to_string(&lc_path).unwrap_or_default();
        let updated_vdf = update_localconfig_playtime(
            &current_content,
            app_id,
            stats.playtime_minutes,
            stats.last_played_ts,
        );
        let _ = fs::write(&lc_path, updated_vdf);
    }

    // 2. Restore UserGameStatsSchema_<app_id>.bin if available
    let schema_bin_path = stats_dir.join(format!("UserGameStatsSchema_{}.bin", app_id));
    if !schema_bin_path.exists() {
        if let Some(ref b64) = stats.schema_b64 {
            if let Some(bytes) = base64_decode(b64) {
                let _ = fs::write(&schema_bin_path, bytes);
            }
        }
    }

    // 3. Restore UserGameStats_<account_id>_<app_id>.bin
    let user_stats_bin_path = stats_dir.join(format!("UserGameStats_{}_{}.bin", account_id, app_id));
    let bin_bytes = if let Some(ref b64) = stats.raw_user_game_stats_b64 {
        base64_decode(b64)
    } else {
        None
    };

    let final_bytes = bin_bytes.unwrap_or_else(|| export_user_game_stats_bin(stats));
    if !final_bytes.is_empty() {
        let _ = fs::write(&user_stats_bin_path, final_bytes);
    }

    // 4. Restore librarycache/<app_id>.json
    let libcache_dir = userdata_acct.join("config").join("librarycache");
    let _ = fs::create_dir_all(&libcache_dir);
    let libcache_file = libcache_dir.join(format!("{}.json", app_id));

    if libcache_file.exists() {
        if let Ok(content) = fs::read_to_string(&libcache_file) {
            if let Some(updated) = update_librarycache_json(&content, &stats.achievements) {
                let _ = fs::write(&libcache_file, updated);
            }
        }
    } else if let Some(ref b64) = stats.librarycache_achievements_b64 {
        if let Some(bytes) = base64_decode(b64) {
            let _ = fs::write(&libcache_file, bytes);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_roundtrip() {
        let sample = b"ChronoKeep Achievements Sync Test Payload \x00\x01\x02\xFF";
        let encoded = base64_encode(sample);
        let decoded = base64_decode(&encoded).expect("failed decode");
        assert_eq!(sample.as_slice(), decoded.as_slice());
    }

    #[test]
    fn test_monotonic_merge() {
        let mut d1 = GameStatsData {
            app_id: 3010850,
            account_id: 12345,
            playtime_minutes: 50,
            last_played_ts: 1000,
            achievements: vec![
                AchievementUnlockRecord {
                    id: "ACH_1".into(),
                    stat_id: 1,
                    bit: 0,
                    unlocked: true,
                    unlock_time: 500,
                },
                AchievementUnlockRecord {
                    id: "ACH_2".into(),
                    stat_id: 1,
                    bit: 1,
                    unlocked: false,
                    unlock_time: 0,
                },
            ],
            ..Default::default()
        };

        let d2 = GameStatsData {
            app_id: 3010850,
            account_id: 12345,
            playtime_minutes: 120,
            last_played_ts: 2000,
            achievements: vec![
                AchievementUnlockRecord {
                    id: "ACH_2".into(),
                    stat_id: 1,
                    bit: 1,
                    unlocked: true,
                    unlock_time: 1500,
                },
            ],
            ..Default::default()
        };

        d1.merge_with(&d2);

        assert_eq!(d1.playtime_minutes, 120);
        assert_eq!(d1.last_played_ts, 2000);
        assert_eq!(d1.achievements.len(), 2);
        assert!(d1.achievements.iter().all(|a| a.unlocked));
    }

    #[test]
    fn test_update_localconfig_playtime_insert_and_update() {
        let vdf = r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"100"
					{
						"Playtime"		"10"
						"LastPlayed"		"500"
					}
				}
			}
		}
	}
}
"#;
        // 1. Update existing app 100 with higher playtime
        let updated = update_localconfig_playtime(vdf, 100, 45, 1200);
        assert!(updated.contains("\"Playtime\"\t\t\"45\""));
        assert!(updated.contains("\"LastPlayed\"\t\t\"1200\""));

        // 2. Insert new app 200
        let inserted = update_localconfig_playtime(&updated, 200, 90, 1500);
        assert!(inserted.contains("\"200\""));
        assert!(inserted.contains("\"Playtime\"\t\t\"90\""));
    }

    #[test]
    fn test_export_bkv_structure() {
        let stats = GameStatsData {
            app_id: 3010850,
            account_id: 12345,
            achievements: vec![
                AchievementUnlockRecord {
                    id: "ACH_1".into(),
                    stat_id: 1,
                    bit: 0,
                    unlocked: true,
                    unlock_time: 1730000000,
                },
            ],
            ..Default::default()
        };

        let bin = export_user_game_stats_bin(&stats);
        assert!(!bin.is_empty());
        assert_eq!(bin[0], 0x00); // Section start
        assert_eq!(bin[bin.len() - 1], 0x08); // End
    }

    #[test]
    fn test_stats_json_serialization_and_roundtrip() {
        let stats = GameStatsData {
            app_id: 3010850,
            account_id: 99999,
            playtime_minutes: 185,
            playtime_windows: 185,
            last_played_ts: 1730500000,
            stats: vec![StatRecord { stat_id: 1, value: 42 }],
            achievements: vec![
                AchievementUnlockRecord {
                    id: "ACH_WIN_GAME".into(),
                    stat_id: 1,
                    bit: 0,
                    unlocked: true,
                    unlock_time: 1730500100,
                },
                AchievementUnlockRecord {
                    id: "ACH_COLLECT_ALL".into(),
                    stat_id: 1,
                    bit: 1,
                    unlocked: false,
                    unlock_time: 0,
                },
            ],
            schema_b64: Some("U0NIRU1BX1RFU1Q=".into()),
            raw_user_game_stats_b64: Some("QktWX1RFU1Q=".into()),
            librarycache_achievements_b64: None,
            updated_at: 1730500200,
        };

        let json_str = serde_json::to_string_pretty(&stats).expect("failed serialize");
        assert!(json_str.contains("\"app_id\": 3010850"));
        assert!(json_str.contains("\"ACH_WIN_GAME\""));

        let restored: GameStatsData = serde_json::from_str(&json_str).expect("failed deserialize");
        assert_eq!(restored.app_id, 3010850);
        assert_eq!(restored.account_id, 99999);
        assert_eq!(restored.playtime_minutes, 185);
        assert_eq!(restored.achievements.len(), 2);
        assert!(restored.achievements[0].unlocked);
        assert!(!restored.achievements[1].unlocked);
        assert_eq!(restored.schema_b64.as_deref(), Some("U0NIRU1BX1RFU1Q="));
    }

    #[test]
    fn test_librarycache_json_update() {
        let mock_json = r#"[
            ["appinfo", {}],
            ["achievements", {
                "data": {
                    "vecHighlight": [
                        { "strID": "ACH_01", "bAchieved": false, "rtUnlocked": 0 }
                    ],
                    "vecAchievedHidden": [],
                    "vecUnachieved": [
                        { "strID": "ACH_02", "bAchieved": false, "rtUnlocked": 0 }
                    ],
                    "nAchieved": 0
                }
            }]
        ]"#;

        let achs = vec![
            AchievementUnlockRecord {
                id: "ACH_01".into(),
                stat_id: 1,
                bit: 0,
                unlocked: true,
                unlock_time: 1730500000,
            },
        ];

        let updated = update_librarycache_json(mock_json, &achs).expect("failed to update librarycache");
        assert!(updated.contains("\"bAchieved\":true"));
        assert!(updated.contains("1730500000"));
        assert!(updated.contains("\"nAchieved\":1"));
    }
}
