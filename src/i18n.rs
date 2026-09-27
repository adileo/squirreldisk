//! Translations.
//!
//! English source strings are the keys: `tr("Scan")`. Each language is a
//! JSON map `{ "English": "Translated" }` in `assets/i18n/<code>.json`,
//! embedded at compile time. Missing entries fall back to English.
//! Placeholders use `{name}` and are filled by [`trf`].

use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Latin,
    Cyrillic,
    Cjk,
    Hangul,
    Arabic,
    Devanagari,
    Bengali,
    Tamil,
    Telugu,
    Thai,
}

pub struct Lang {
    pub code: &'static str,
    /// Name in its own language, as shown in the picker.
    pub native: &'static str,
    pub script: Script,
    json: &'static str,
}

macro_rules! lang {
    ($code:literal, $native:literal, $script:ident) => {
        Lang { code: $code, native: $native, script: Script::$script, json: include_str!(concat!("../assets/i18n/", $code, ".json")) }
    };
}

/// The 25 most spoken languages (plus English as the source).
pub const LANGS: [Lang; 25] = [
    Lang { code: "en", native: "English", script: Script::Latin, json: "{}" },
    lang!("zh", "简体中文", Cjk),
    lang!("hi", "हिन्दी", Devanagari),
    lang!("es", "Español", Latin),
    lang!("fr", "Français", Latin),
    lang!("ar", "العربية", Arabic),
    lang!("bn", "বাংলা", Bengali),
    lang!("pt", "Português", Latin),
    lang!("ru", "Русский", Cyrillic),
    lang!("ur", "اردو", Arabic),
    lang!("id", "Bahasa Indonesia", Latin),
    lang!("de", "Deutsch", Latin),
    lang!("ja", "日本語", Cjk),
    lang!("mr", "मराठी", Devanagari),
    lang!("te", "తెలుగు", Telugu),
    lang!("tr", "Türkçe", Latin),
    lang!("ta", "தமிழ்", Tamil),
    lang!("vi", "Tiếng Việt", Latin),
    lang!("ko", "한국어", Hangul),
    lang!("it", "Italiano", Latin),
    lang!("fa", "فارسی", Arabic),
    lang!("pl", "Polski", Latin),
    lang!("uk", "Українська", Cyrillic),
    lang!("th", "ไทย", Thai),
    lang!("nl", "Nederlands", Latin),
];

struct Current {
    code: &'static str,
    map: HashMap<&'static str, &'static str>,
}

static CURRENT: RwLock<Option<Current>> = RwLock::new(None);

pub fn lang(code: &str) -> &'static Lang {
    LANGS.iter().find(|l| l.code == code).unwrap_or(&LANGS[0])
}

/// Two-letter language of the OS UI, if we support it.
pub fn system_language() -> &'static str {
    let from_env = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| v.len() >= 2 && v != "C" && v != "POSIX" && !v.starts_with("C."));
    #[allow(unused_mut)]
    let mut raw = from_env;
    #[cfg(target_os = "macos")]
    if raw.is_none() {
        // Apps launched from Finder have no LANG: ask for the preferred languages.
        raw = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).split('"').nth(1).map(|s| s.to_string()));
    }
    #[cfg(windows)]
    if raw.is_none() {
        raw = windows_ui_language();
    }
    let code = raw.map(|r| r.chars().take(2).collect::<String>().to_lowercase()).unwrap_or_default();
    lang(&code).code
}

#[cfg(windows)]
fn windows_ui_language() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..(n - 1) as usize]))
}

/// Resolves the setting ("auto" or a code) to a supported language code.
pub fn resolve(setting: &str) -> &'static str {
    if setting == "auto" || setting.is_empty() {
        system_language()
    } else {
        lang(setting).code
    }
}

/// Switches the UI language.
pub fn set_language(code: &str) {
    let l = lang(code);
    let map: HashMap<String, String> = serde_json::from_str(l.json).unwrap_or_default();
    // Leak once per switch: a few KB, and it lets `tr` hand out &'static str.
    let map = map
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (&*Box::leak(k.into_boxed_str()), &*Box::leak(v.into_boxed_str())))
        .collect();
    *CURRENT.write().unwrap() = Some(Current { code: l.code, map });
}

pub fn current() -> &'static str {
    CURRENT.read().unwrap().as_ref().map(|c| c.code).unwrap_or("en")
}

/// Translates an English UI string.
pub fn tr(key: &'static str) -> &'static str {
    match CURRENT.read().unwrap().as_ref() {
        Some(c) => c.map.get(key).copied().unwrap_or(key),
        None => key,
    }
}

/// Translates and fills `{name}` placeholders.
pub fn trf(key: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = tr(key).to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

/// Translates the progress status strings set by the scanners.
pub fn tr_status(status: &str) -> String {
    if let Some(secs) = status.strip_prefix("Scanned in ").and_then(|r| r.strip_suffix('s')) {
        return trf("Scanned in {seconds}s", &[("seconds", &secs)]);
    }
    match status {
        "Scanning" => tr("Scanning"),
        "Connecting" => tr("Connecting"),
        "Installing agent" => tr("Installing agent"),
        "Uploading agent" => tr("Uploading agent"),
        "Listing" => tr("Listing"),
        "Done" => tr("Done"),
        other => return other.to_string(),
    }
    .to_string()
}

/// Translates the deletion phases set by `delete.rs`.
pub fn tr_phase(phase: &str) -> String {
    match phase {
        "Backing up" => tr("Backing up"),
        "Uploading backup" => tr("Uploading backup"),
        "Moving to Trash" => tr("Moving to Trash"),
        "Deleting" => tr("Deleting"),
        "Securely erasing" => tr("Securely erasing"),
        "Deleting on server" => tr("Deleting on server"),
        "Deleting from cloud" => tr("Deleting from cloud"),
        other => return other.to_string(),
    }
    .to_string()
}

/// Translates the reasons given by the safety rules (`safety.rs`).
pub fn tr_reason(reason: &str) -> String {
    let inner = |prefix: &str| reason.strip_prefix(prefix).and_then(|r| r.strip_suffix(')'));
    if let Some(p) = inner("protected system location (") {
        return trf("protected system location ({path})", &[("path", &p)]);
    }
    if let Some(p) = inner("essential folder (") {
        return trf("essential folder ({path})", &[("path", &p)]);
    }
    if let Some(p) = reason.strip_prefix("inside ").and_then(|r| r.strip_suffix(": apps or system components may stop working")) {
        return trf("inside {path}: apps or system components may stop working", &[("path", &p)]);
    }
    match reason {
        "this is the root of a disk" => tr("this is the root of a disk"),
        "this is a mounted volume" => tr("this is a mounted volume"),
        "hidden configuration folder" => tr("hidden configuration folder"),
        other => return other.to_string(),
    }
    .to_string()
}

/// A system font that covers the script of `code`, as (bytes, index in collection).
/// Loaded at runtime so the app doesn't have to ship tens of MB of fonts.
pub fn system_font(code: &str) -> Option<(Vec<u8>, u32)> {
    let script = lang(code).script;
    if matches!(script, Script::Latin | Script::Cyrillic) {
        return None; // covered by the bundled fonts
    }
    let candidates: &[&str] = match (cfg!(target_os = "macos"), cfg!(windows), script) {
        (true, _, Script::Cjk) => &["/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"],
        (true, _, Script::Hangul) => &["/System/Library/Fonts/Supplemental/Arial Unicode.ttf", "/System/Library/Fonts/AppleSDGothicNeo.ttc"],
        (true, _, Script::Arabic) => &["/System/Library/Fonts/GeezaPro.ttc", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"],
        (true, _, Script::Devanagari) => &["/System/Library/Fonts/Kohinoor.ttc", "/System/Library/Fonts/Supplemental/Devanagari Sangam MN.ttc"],
        (true, _, Script::Bengali) => &["/System/Library/Fonts/KohinoorBangla.ttc", "/System/Library/Fonts/Supplemental/Bangla Sangam MN.ttc"],
        (true, _, Script::Telugu) => &["/System/Library/Fonts/KohinoorTelugu.ttc", "/System/Library/Fonts/Supplemental/Telugu Sangam MN.ttc"],
        (true, _, Script::Tamil) => &["/System/Library/Fonts/Supplemental/Tamil Sangam MN.ttc", "/System/Library/Fonts/Supplemental/Tamil MN.ttc"],
        (true, _, Script::Thai) => &["/System/Library/Fonts/ThonburiUI.ttc", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"],
        (_, true, Script::Cjk) if code == "ja" => &["C:\\Windows\\Fonts\\YuGothM.ttc", "C:\\Windows\\Fonts\\meiryo.ttc", "C:\\Windows\\Fonts\\msgothic.ttc"],
        (_, true, Script::Cjk) => &["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\simsun.ttc"],
        (_, true, Script::Hangul) => &["C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\gulim.ttc"],
        (_, true, Script::Arabic) => &["C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\tahoma.ttf"],
        (_, true, Script::Devanagari | Script::Bengali | Script::Tamil | Script::Telugu) => &["C:\\Windows\\Fonts\\Nirmala.ttf", "C:\\Windows\\Fonts\\NirmalaUI.ttf"],
        (_, true, Script::Thai) => &["C:\\Windows\\Fonts\\leelawui.ttf", "C:\\Windows\\Fonts\\tahoma.ttf"],
        _ => &[],
    };
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            return Some((bytes, 0));
        }
    }
    // Linux (and anything else): ask fontconfig.
    let fc_lang = match code {
        "zh" => "zh-cn",
        other => other,
    };
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}\n%{index}", &format!(":lang={fc_lang}")]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let file = lines.next()?.trim().to_string();
    let index = lines.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    std::fs::read(&file).ok().map(|b| (b, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_translation_parses_and_keeps_placeholders() {
        let english_keys: std::collections::HashSet<String> = {
            let src = [
                include_str!("ui/home.rs"),
                include_str!("ui/view.rs"),
                include_str!("ui/modals.rs"),
                include_str!("ui/fx.rs"),
                include_str!("ui/ads.rs"),
                include_str!("ui/app.rs"),
                include_str!("sponsor/mod.rs"),
                include_str!("sponsor/interests.rs"),
                include_str!("i18n.rs"),
            ]
            .join("\n");
            let mut keys = std::collections::HashSet::new();
            let bytes = src.as_bytes();
            for (i, _) in src.match_indices("tr(\"").chain(src.match_indices("trf(\"")) {
                let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
                if !prev_ok {
                    continue;
                }
                let start = src[i..].find('"').unwrap() + i + 1;
                let mut key = String::new();
                let mut chars = src[start..].chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                key.push(n);
                            }
                        }
                        '"' => break,
                        c => key.push(c),
                    }
                }
                keys.insert(key);
            }
            keys
        };
        for l in LANGS.iter().skip(1) {
            let map: HashMap<String, String> = serde_json::from_str(l.json).unwrap_or_else(|e| panic!("{}.json: {e}", l.code));
            for (k, v) in &map {
                for part in k.split('{').skip(1) {
                    let name = part.split('}').next().unwrap();
                    assert!(v.contains(&format!("{{{name}}}")), "{}: '{v}' lost placeholder {{{name}}}", l.code);
                }
            }
            let missing: Vec<&String> = english_keys.iter().filter(|k| !map.contains_key(*k)).collect();
            assert!(missing.len() <= english_keys.len() / 10, "{}: {} strings missing, e.g. {:?}", l.code, missing.len(), missing.iter().take(3).collect::<Vec<_>>());
        }
    }

    #[test]
    fn fallback_is_english() {
        assert_eq!(resolve("xx"), "en");
        assert_eq!(lang("it").native, "Italiano");
    }
}
