//! Multi-language (i18n) support for ChronoKeep GUI & Backend.
//! Supports automatic OS language detection (`pt-BR` if Portuguese, `en-US` fallback)
//! and persistent user preference across `gui_prefs.ini` and `chronokeep.ini`.

use std::fs;
use std::path::Path;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lang {
    EnUs,
    PtBr,
}

impl Lang {
    pub fn as_code(self) -> &'static str {
        match self {
            Lang::EnUs => "en-US",
            Lang::PtBr => "pt-BR",
        }
    }

    pub fn is_pt(self) -> bool {
        matches!(self, Lang::PtBr)
    }

    /// Detects the operating system UI language.
    /// Returns `PtBr` if the OS locale is Portuguese (`pt*`), otherwise defaults to `EnUs`.
    pub fn detect_os_locale() -> Lang {
        #[cfg(target_os = "windows")]
        unsafe {
            let mut buf = [0u16; 85];
            let len = crate::win32::GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32);
            if len > 1 {
                let s = String::from_utf16_lossy(&buf[..(len - 1) as usize])
                    .trim()
                    .to_ascii_lowercase();
                if s.starts_with("pt") {
                    return Lang::PtBr;
                }
                if !s.is_empty() {
                    return Lang::EnUs;
                }
            }
            // Fallback to LANGID primary language (0x16 == LANG_PORTUGUESE)
            let lang_id = crate::win32::GetUserDefaultUILanguage();
            if (lang_id & 0x03ff) == 0x16 {
                return Lang::PtBr;
            }
            Lang::EnUs
        }

        #[cfg(not(target_os = "windows"))]
        {
            for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
                if let Ok(val) = std::env::var(var) {
                    let clean = val.trim().to_ascii_lowercase();
                    if !clean.is_empty() && clean != "c" && clean != "posix" {
                        if clean.starts_with("pt") {
                            return Lang::PtBr;
                        }
                        return Lang::EnUs;
                    }
                }
            }
            Lang::EnUs
        }
    }

    /// Normalizes a user preference string to `"system"`, `"pt-BR"`, or `"en-US"`.
    pub fn normalize_pref(pref: &str) -> String {
        let norm = pref.trim().to_ascii_lowercase().replace('_', "-");
        match norm.as_str() {
            "pt-br" | "pt" => "pt-BR".to_string(),
            "en-us" | "en" => "en-US".to_string(),
            _ => "system".to_string(),
        }
    }

    /// Resolves a preference (`"system"`, `"pt-BR"`, `"en-US"`) into a concrete `Lang`.
    pub fn resolve_preference(pref: &str) -> Lang {
        let norm = pref.trim().to_ascii_lowercase().replace('_', "-");
        match norm.as_str() {
            "pt-br" | "pt" => Lang::PtBr,
            "en-us" | "en" => Lang::EnUs,
            _ => Self::detect_os_locale(),
        }
    }
}

/// Loads saved language preference (`"system"`, `"pt-BR"`, or `"en-US"`).
/// Checks `chronokeep.ini` (if available) and `gui_prefs.ini`, defaulting to `"system"`.
pub fn load_saved_language_pref(steam_dir: Option<&Path>) -> String {
    // 1. Check gui_prefs.ini first (always writable per-user in %APPDATA%\ChronoKeep)
    let prefs_file = crate::commands::get_gui_prefs_path();
    if let Ok(content) = fs::read_to_string(&prefs_file) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case("language") && !v.trim().is_empty() {
                    return Lang::normalize_pref(v);
                }
            }
        }
    }

    // 2. Fallback to chronokeep.ini in Steam directory
    if let Some(s_path) = steam_dir {
        let ini_path = s_path.join("chronokeep.ini");
        if let Ok(content) = fs::read_to_string(&ini_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once('=') {
                    if k.trim().eq_ignore_ascii_case("language") && !v.trim().is_empty() {
                        return Lang::normalize_pref(v);
                    }
                }
            }
        }
    }

    "system".to_string()
}

/// Persists language preference into `gui_prefs.ini` and (if present) `chronokeep.ini`.
pub fn save_language_pref(pref: &str, steam_dir: Option<&Path>) {
    let canonical = Lang::normalize_pref(pref);

    // 1. Update gui_prefs.ini
    let prefs_file = crate::commands::get_gui_prefs_path();
    if let Some(parent) = prefs_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut lines = Vec::new();
    let mut updated = false;
    if let Ok(content) = fs::read_to_string(&prefs_file) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case("language") {
                    lines.push(format!("language={}", canonical));
                    updated = true;
                    continue;
                }
            }
            if !trimmed.is_empty() {
                lines.push(trimmed.to_string());
            }
        }
    }
    if !updated {
        lines.push(format!("language={}", canonical));
    }
    let _ = fs::write(&prefs_file, lines.join("\r\n") + "\r\n");

    // 2. Also update chronokeep.ini if it already exists in Steam dir
    if let Some(s_path) = steam_dir {
        let ini_path = s_path.join("chronokeep.ini");
        if ini_path.exists() {
            if let Ok(content) = fs::read_to_string(&ini_path) {
                let mut ini_lines = Vec::new();
                let mut ini_updated = false;
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some((k, _)) = trimmed.split_once('=') {
                        if k.trim().eq_ignore_ascii_case("language") {
                            ini_lines.push(format!("language={}", canonical));
                            ini_updated = true;
                            continue;
                        }
                    }
                    ini_lines.push(line.to_string());
                }
                if !ini_updated {
                    ini_lines.push(format!("language={}", canonical));
                }
                let _ = fs::write(&ini_path, ini_lines.join("\r\n") + "\r\n");
            }
        }
    }
}

/// Resolves the active `Lang` right now from saved preferences or OS locale.
pub fn current_lang(steam_dir: Option<&Path>) -> Lang {
    let pref = load_saved_language_pref(steam_dir);
    Lang::resolve_preference(&pref)
}
