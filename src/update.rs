//! Self-update from GitHub releases.
//!
//! Release assets are expected to be named `squirreldisk-<target-triple>` (plus
//! `.exe` on Windows), as produced by `.github/workflows/release.yml`.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use crate::GITHUB_REPO;

pub fn current_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(windows) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    #[allow(dead_code)]
    pub notes: String,
    pub asset_url: Option<String>,
}

#[derive(Clone, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading,
    Ready(String),
    Failed(String),
}

pub struct Updater {
    pub state: Arc<Mutex<State>>,
    pub downloaded: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Compares dotted versions numerically ("2.10.0" > "2.9.3").
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v').split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

impl Updater {
    pub fn new() -> Self {
        Updater {
            state: Arc::new(Mutex::new(State::Idle)),
            downloaded: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn check(&self, ctx: eframe::egui::Context) {
        let st = self.state.clone();
        *st.lock().unwrap() = State::Checking;
        std::thread::spawn(move || {
            let r = (|| -> Result<State, String> {
                let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
                let mut resp = ureq::get(&url)
                    .header("User-Agent", concat!("SquirrelDisk/", env!("CARGO_PKG_VERSION")))
                    .header("Accept", "application/vnd.github+json")
                    .call()
                    .map_err(|e| e.to_string())?;
                let rel: GhRelease = resp.body_mut().read_json().map_err(|e| e.to_string())?;
                if rel.draft || rel.prerelease || !is_newer(&rel.tag_name, env!("CARGO_PKG_VERSION")) {
                    return Ok(State::UpToDate);
                }
                let want = format!("squirreldisk-{}", current_target());
                let asset = rel
                    .assets
                    .iter()
                    .find(|a| a.name == want || a.name == format!("{want}.exe"))
                    .map(|a| a.browser_download_url.clone());
                Ok(State::Available(Release {
                    version: rel.tag_name.trim_start_matches('v').to_string(),
                    notes: rel.body,
                    asset_url: asset,
                }))
            })();
            *st.lock().unwrap() = r.unwrap_or_else(State::Failed);
            ctx.request_repaint();
        });
    }

    pub fn install(&self, rel: Release, ctx: eframe::egui::Context) {
        let Some(url) = rel.asset_url.clone() else {
            *self.state.lock().unwrap() = State::Failed("no build for this platform in the release".into());
            return;
        };
        let (st, dl, total) = (self.state.clone(), self.downloaded.clone(), self.total.clone());
        *st.lock().unwrap() = State::Downloading;
        std::thread::spawn(move || {
            let r = (|| -> Result<(), String> {
                let resp = ureq::get(&url)
                    .header("User-Agent", concat!("SquirrelDisk/", env!("CARGO_PKG_VERSION")))
                    .call()
                    .map_err(|e| e.to_string())?;
                total.store(resp.body().content_length().unwrap_or(0), Ordering::Relaxed);
                let mut reader = resp.into_body().into_reader();
                let tmp = std::env::temp_dir().join(format!("squirreldisk-update-{}", std::process::id()));
                let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut f, &buf[..n]).map_err(|e| e.to_string())?;
                    dl.fetch_add(n as u64, Ordering::Relaxed);
                    ctx.request_repaint();
                }
                drop(f);
                if std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0) < 1_000_000 {
                    return Err("downloaded file looks broken".into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
                }
                self_replace::self_replace(&tmp).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&tmp);
                Ok(())
            })();
            *st.lock().unwrap() = match r {
                Ok(()) => State::Ready(rel.version.clone()),
                Err(e) => State::Failed(e),
            };
            ctx.request_repaint();
        });
    }
}

/// Relaunches the (updated) executable and exits.
pub fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::is_newer("v2.1.0", "2.0.9"));
        assert!(super::is_newer("2.10.0", "2.9.0"));
        assert!(!super::is_newer("2.0.0", "2.0.0"));
        assert!(!super::is_newer("1.9", "2.0.0"));
    }
}
