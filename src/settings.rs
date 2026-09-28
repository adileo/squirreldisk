use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// A config folder chosen by the user instead of the system one:
/// `SQUIRRELDISK_CONFIG_DIR`, or a `portable` folder next to the executable
/// (next to the AppImage file, not inside its mount).
pub fn portable_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("SQUIRRELDISK_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file()).or_else(|| std::env::current_exe().ok())?;
    exe.parent().map(|d| d.join("portable")).filter(|d| d.is_dir())
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = portable_dir() {
        return Some(d);
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library/Application Support/SquirrelDisk"))
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("SquirrelDisk"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".config")))
            .map(|c| c.join("squirreldisk"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    /// "auto" (follow the system) or a language code.
    pub language: String,
    pub sound: bool,
    pub volume: f32,
    pub shader_fx: bool,
    pub auto_update: bool,
    /// Download new versions in the background; they're applied on restart.
    pub auto_install: bool,
    pub rings: usize,
    /// "sunburst" (default) or "treemap".
    pub chart_style: String,
    pub watch_fs: bool,
    pub ssh_history: Vec<String>,
    pub backup_folder: Option<String>,
    pub backup_remote: Option<String>,
    pub skipped_version: Option<String>,
    /// Choose sponsors from on-device signals (nothing is sent).
    pub personalized_sponsors: bool,
    /// Send anonymous daily view totals / count clicks.
    pub sponsor_measurement: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "Hazelnut".into(),
            language: "auto".into(),
            sound: false,
            volume: 0.6,
            shader_fx: true,
            auto_update: true,
            auto_install: true,
            rings: 6,
            chart_style: "sunburst".into(),
            watch_fs: true,
            ssh_history: Vec::new(),
            backup_folder: None,
            backup_remote: None,
            skipped_version: None,
            personalized_sponsors: true,
            sponsor_measurement: true,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    pub fn load() -> Settings {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if crate::scan::demo::enabled() {
            return; // demo recordings must not touch the user's settings
        }
        if let Some(p) = Self::path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }
}
