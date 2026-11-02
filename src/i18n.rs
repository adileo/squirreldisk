//! Translations.
//!
//! English source strings are the keys: `tr("Scan")`. Each language is a
//! JSON map `{ "English": "Translated" }` in `assets/i18n/<code>.json`,
//! embedded at compile time. Missing entries fall back to English.
//! Placeholders use `{name}` and are filled by [`trf`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
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

impl Script {
    /// The script of `c`, among the ones the bundled fonts don't cover.
    fn of(c: char) -> Option<Script> {
        Some(match c {
            '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}' => Script::Arabic,
            '\u{0900}'..='\u{097F}' | '\u{A8E0}'..='\u{A8FF}' => Script::Devanagari,
            '\u{0980}'..='\u{09FF}' => Script::Bengali,
            '\u{0B80}'..='\u{0BFF}' => Script::Tamil,
            '\u{0C00}'..='\u{0C7F}' => Script::Telugu,
            '\u{0E00}'..='\u{0E7F}' => Script::Thai,
            '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}' | '\u{AC00}'..='\u{D7FF}' => Script::Hangul,
            // ideographs, kana, bopomofo, CJK punctuation and full-width forms
            '\u{2E80}'..='\u{2FDF}' | '\u{3000}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFF}' => Script::Cjk,
            _ => return None,
        })
    }

    pub fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// Scripts used by the file and volume names on screen, one [`Script::bit`]
/// each. Set by the UI from the open sessions, read by the font installer.
static NAME_SCRIPTS: AtomicU32 = AtomicU32::new(0);

/// The scripts `name` is written in, as [`Script::bit`]s. Names can be in any
/// language, whatever the UI language, and each script needs its own font.
pub fn scripts_of(name: &str) -> u32 {
    if name.is_ascii() {
        return 0;
    }
    name.chars().filter_map(Script::of).fold(0, |bits, s| bits | s.bit())
}

pub fn set_name_scripts(bits: u32) {
    NAME_SCRIPTS.store(bits, Ordering::Relaxed);
}

pub fn name_scripts() -> u32 {
    NAME_SCRIPTS.load(Ordering::Relaxed)
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

/// Optional context, e.g. `Clear [collector]`; other languages keep `Clear`.
pub fn trc(key: &'static str, context: &str) -> &'static str {
    translation_variant(key, Some(context))
}

fn translation_variant(key: &'static str, variant: Option<&str>) -> &'static str {
    let current = CURRENT.read().unwrap();
    let Some(c) = current.as_ref() else { return key };
    lookup_variant(&c.map, key, variant)
}

fn lookup_variant<'a>(map: &HashMap<&str, &'a str>, key: &'a str, variant: Option<&str>) -> &'a str {
    variant.and_then(|v| map.get(format!("{key} [{v}]").as_str()).copied()).or_else(|| map.get(key).copied()).unwrap_or(key)
}

/// Russian cardinal forms. The unsuffixed entry is the many/default form;
/// optional `[one]` and `[few]` entries keep existing language files valid.
pub fn trn(key: &'static str, count: u64, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(translation_variant(key, plural_variant(current(), count)), args)
}

fn plural_variant(code: &str, count: u64) -> Option<&'static str> {
    if code != "ru" || (11..=14).contains(&(count % 100)) {
        return None;
    }
    match count % 10 {
        1 => Some("one"),
        2..=4 => Some("few"),
        _ => None,
    }
}

/// Translates and fills `{name}` placeholders.
pub fn trf(key: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(tr(key), args)
}

fn fill(template: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = template.to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

/// UI-only formatting: CLI output and non-Russian locales keep their format.
pub fn fmt_size(bytes: u64) -> String {
    localized_size(bytes, current())
}

fn localized_size(bytes: u64, code: &str) -> String {
    let text = crate::tree::fmt_size(bytes);
    if code != "ru" {
        return text;
    }
    let (number, unit) = text.split_once(' ').unwrap();
    let unit = match unit {
        "B" => "Б", "KB" => "КБ", "MB" => "МБ", "GB" => "ГБ", "TB" => "ТБ", "PB" => "ПБ", other => other,
    };
    format!("{}\u{a0}{unit}", number.replace('.', ","))
}

pub fn fmt_count(n: u64) -> String {
    localized_count(n, current())
}

fn localized_count(n: u64, code: &str) -> String {
    let text = crate::tree::fmt_count(n);
    if code == "ru" { text.replace(',', "\u{a0}") } else { text }
}

pub fn fmt_count_compact(n: u64) -> String {
    localized_compact(n, current())
}

fn localized_compact(n: u64, code: &str) -> String {
    let text = crate::tree::fmt_count_compact(n);
    if code == "ru" { text.replace('.', ",").replace('k', "\u{a0}тыс.").replace('M', "\u{a0}млн") } else { text }
}

pub fn fmt_seconds(seconds: f32) -> String {
    let number = localized_decimal(&format!("{seconds:.1}"), current());
    trf("{seconds} s", &[("seconds", &number)])
}

fn localized_decimal(number: &str, code: &str) -> String {
    if code == "ru" { number.replace('.', ",") } else { number.to_string() }
}

pub fn fmt_date((y, m, d): (u16, u8, u8)) -> String {
    localized_date((y, m, d), current())
}

fn localized_date((y, m, d): (u16, u8, u8), code: &str) -> String {
    if code == "ru" { format!("{d:02}.{m:02}.{y:04}") } else { format!("{y}-{m:02}-{d:02}") }
}

/// Translate application-owned removal reasons, preserving the path and any
/// external diagnostic text verbatim. Workers do not depend on the UI language.
pub fn tr_error(error: &str) -> String {
    translate_error(error, tr)
}

fn translate_error<'a>(error: &str, translate: impl Fn(&'static str) -> &'a str) -> String {
    const REASONS: [&str; 5] = [
        "the copy to keep is missing, nothing removed",
        "protected system location",
        "changed since the comparison, left alone",
        "same file as the copy kept, left alone",
        "rclone could not delete it",
    ];
    for reason in REASONS {
        if error == reason {
            return translate(reason).to_string();
        }
        if let Some(path) = error.strip_suffix(reason).and_then(|prefix| prefix.strip_suffix(": ")) {
            return format!("{path}: {}", translate(reason));
        }
    }
    error.to_string()
}

/// Translates the progress status strings set by the scanners.
pub fn tr_status(status: &str) -> String {
    if let Some(secs) = status.strip_prefix("Scanned in ").and_then(|r| r.strip_suffix('s')) {
        let secs = localized_decimal(secs, current());
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
        (_, true, Script::Devanagari | Script::Bengali | Script::Tamil | Script::Telugu) => &["C:\\Windows\\Fonts\\Nirmala.ttf", "C:\\Windows\\Fonts\\NirmalaUI.ttf", "C:\\Windows\\Fonts\\Nirmala.ttc"],
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
                include_str!("ui/dupes.rs"),
                include_str!("ui/view.rs"),
                include_str!("ui/treemap.rs"),
                include_str!("ui/modals.rs"),
                include_str!("ui/cloud.rs"),
                include_str!("ui/fx.rs"),
                include_str!("ui/ads.rs"),
                include_str!("ui/app.rs"),
                include_str!("sponsor/mod.rs"),
                include_str!("sponsor/interests.rs"),
                include_str!("i18n.rs"),
            ]
            .iter().map(|s| s.split("\n#[cfg(test)]\nmod tests").next().unwrap()).collect::<Vec<_>>().join("\n");
            let mut keys = std::collections::HashSet::new();
            let bytes = src.as_bytes();
            for (i, call) in src.match_indices("tr(").chain(src.match_indices("trf(")).chain(src.match_indices("trn(")).chain(src.match_indices("trc(")) {
                let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
                if !prev_ok {
                    continue;
                }
                let rest = &src[i + call.len()..];
                let trimmed = rest.trim_start();
                if !trimmed.starts_with('"') {
                    continue; // dynamic keys are checked through their dictionaries
                }
                let start = i + call.len() + rest.len() - trimmed.len() + 1;
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
            if l.code == "ru" {
                assert!(missing.is_empty(), "ru: missing translations: {missing:?}");
                let placeholders = |s: &str| -> std::collections::HashSet<String> {
                    s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(name, _)| name.to_string())).collect()
                };
                for (key, value) in &map {
                    assert_eq!(placeholders(key), placeholders(value), "ru: placeholders differ for {key}");
                }
            }
            assert!(missing.len() <= english_keys.len() / 10, "{}: {} strings missing, e.g. {:?}", l.code, missing.len(), missing.iter().take(3).collect::<Vec<_>>());
        }
    }

    #[test]
    fn russian_numbers_sizes_and_dates() {
        assert_eq!(localized_size(0, "ru"), "0\u{a0}Б");
        assert_eq!(localized_size(999, "ru"), "999\u{a0}Б");
        assert_eq!(localized_size(1_000, "ru"), "1,0\u{a0}КБ");
        assert_eq!(localized_size(17_400_000_000, "ru"), "17,4\u{a0}ГБ");
        assert_eq!(localized_count(1_234_567, "ru"), "1\u{a0}234\u{a0}567");
        assert_eq!(localized_compact(999, "ru"), "999");
        assert_eq!(localized_compact(2_345, "ru"), "2\u{a0}тыс.");
        assert_eq!(localized_compact(1_758_000, "ru"), "1,75\u{a0}млн");
        assert_eq!(localized_decimal("3.5", "ru"), "3,5");
        assert_eq!(localized_date((2026, 11, 2), "ru"), "02.11.2026");
        // Changing the UI format must not change CLI or other locales.
        for code in ["en", "de", "uk"] {
            assert_eq!(localized_size(17_400_000_000, code), "17.4 GB");
            assert_eq!(localized_count(1_234_567, code), "1,234,567");
            assert_eq!(localized_compact(1_758_000, code), "1.75M");
            assert_eq!(localized_date((2026, 11, 2), code), "2026-11-02");
        }
    }

    #[test]
    fn russian_plural_forms_and_context_use_the_dictionary() {
        let owned: HashMap<String, String> = serde_json::from_str(include_str!("../assets/i18n/ru.json")).unwrap();
        let map: HashMap<&str, &str> = owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        for (n, expected) in [(0, "0 копий"), (1, "1 копия"), (2, "2 копии"), (5, "5 копий"), (11, "11 копий"), (12, "12 копий"), (14, "14 копий"), (21, "21 копия"), (22, "22 копии"), (25, "25 копий"), (101, "101 копия"), (111, "111 копий"), (112, "112 копий")] {
            let template = lookup_variant(&map, "{n} copies", plural_variant("ru", n));
            assert_eq!(fill(template, &[("n", &n)]), expected);
        }
        assert_eq!(lookup_variant(&map, "Clear", Some("collector")), "Очистить список");
        assert_eq!(lookup_variant(&map, "Clear", Some("duplicates")), "Снять отметки");
        assert_eq!(lookup_variant(&map, "Clear", Some("unknown")), "Очистить");
        let english = HashMap::new();
        assert_eq!(lookup_variant(&english, "Clear", Some("collector")), "Clear");
        assert_eq!(plural_variant("en", 21), None);
        // Existing language dictionaries still fall back to their base translation.
        let french = HashMap::from([("{n} copies", "Copies : {n}")]);
        assert_eq!(lookup_variant(&french, "{n} copies", Some("one")), "Copies : {n}");
    }

    #[test]
    fn removal_reasons_are_translated_without_changing_paths_or_external_errors() {
        let owned: HashMap<String, String> = serde_json::from_str(include_str!("../assets/i18n/ru.json")).unwrap();
        let map: HashMap<&str, &str> = owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let translate = |key| map.get(key).copied().unwrap_or(key);
        assert_eq!(translate_error("/Фото/a.jpg: the copy to keep is missing, nothing removed", translate), "/Фото/a.jpg: сохраняемая копия недоступна; удаление отменено");
        assert_eq!(translate_error("C:\\Фото\\a.jpg: changed since the comparison, left alone", translate), "C:\\Фото\\a.jpg: файл изменился после сравнения и не был удалён");
        assert_eq!(translate_error("remote:folder/a.jpg: same file as the copy kept, left alone", translate), "remote:folder/a.jpg: это сохраняемая копия; файл не был удалён");
        assert_eq!(translate_error("/tmp/protected system location", translate), "/tmp/protected system location");
        assert_eq!(translate_error("/Фото/a.jpg: Permission denied (os error 13)", translate), "/Фото/a.jpg: Permission denied (os error 13)");
    }

    #[test]
    fn fallback_is_english() {
        assert_eq!(resolve("xx"), "en");
        assert_eq!(lang("it").native, "Italiano");
    }

    #[test]
    fn scripts_of_names() {
        assert_eq!(Script::of('照'), Some(Script::Cjk));
        assert_eq!(Script::of('の'), Some(Script::Cjk));
        assert_eq!(Script::of('（'), Some(Script::Cjk));
        assert_eq!(Script::of('한'), Some(Script::Hangul));
        assert_eq!(Script::of('ก'), Some(Script::Thai));
        assert_eq!(Script::of('ع'), Some(Script::Arabic));
        assert_eq!(Script::of('é'), None);
        assert_eq!(Script::of('Ж'), None);
        assert_eq!(scripts_of("照片（2024）"), Script::Cjk.bit());
        assert_eq!(scripts_of("사진 ภาพ"), Script::Hangul.bit() | Script::Thai.bit());
        assert_eq!(scripts_of("Photos été"), 0);
    }
}
