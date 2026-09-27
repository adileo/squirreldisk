//! # Sponsors, the privacy-respecting way
//!
//! SquirrelDisk shows one slim sponsor banner. Choosing *which* one is done
//! **entirely on your computer**:
//!
//! 1. We download the **whole** catalog (`wire::fetch_catalog`). Every user
//!    makes the exact same request, so the server learns nothing about you.
//! 2. From data the app already has in memory after a scan, we derive a handful
//!    of **coarse signals** ([`Signals`]): OS, language, how full the disk is,
//!    and up to 8 broad interest categories ([`interests`]). They never leave
//!    the device and are never written to disk.
//! 3. Each ad carries targeting rules ([`Target`]); we match them locally and
//!    pick one, weighted, per launch.
//!
//! Measurement is aggregate-only (see [`wire`] for the complete list of what is
//! ever sent): a click goes through a counting redirect that only knows the ad
//! id, and view counts are sent as daily totals with no identifier of any kind.
//! Both personalization and measurement can be switched off in Settings.

pub mod interests;
pub mod wire;

pub use interests::Interest;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub const SPONSORS_URL: &str = "https://squirreldisk.com/sponsors";

/// Targeting rules attached to an ad. Every listed condition must hold;
/// empty lists / `None` mean "any".
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct Target {
    /// Any-of: the user must show at least one of these interests.
    pub interests: Vec<Interest>,
    /// Any-of: "macos", "windows", "linux".
    pub os: Vec<String>,
    /// Any-of: two-letter language codes ("en", "it", …).
    pub lang: Vec<String>,
    /// Startup disk at least this full (0.0 – 1.0).
    pub disk_full_above: Option<f32>,
    /// Has (or hasn't) an external drive attached.
    pub external_drive: Option<bool>,
}

impl Target {
    /// Rules that depend on what we inferred about the user (as opposed to the
    /// plain technical context) only apply when personalization is on.
    fn is_personal(&self) -> bool {
        !self.interests.is_empty() || self.disk_full_above.is_some() || self.external_drive.is_some()
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Ad {
    /// Stable identifier, used for the click redirect and view counts.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// Final destination (the redirect endpoint sends people here).
    pub url: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "one")]
    pub weight: u32,
    #[serde(default)]
    pub target: Target,
}

fn one() -> u32 {
    1
}

impl Ad {
    /// Shown when there is nothing to serve (offline, empty catalog, no match).
    pub fn house() -> Ad {
        Ad {
            id: String::new(),
            title: "Your brand here".into(),
            text: "Reach people who care about their disks.".into(),
            url: SPONSORS_URL.into(),
            icon: "acorn".into(),
            weight: 1,
            target: Target::default(),
        }
    }

    pub fn is_house(&self) -> bool {
        self.id.is_empty()
    }
}

/// Coarse, on-device context used for matching. Lives in memory only.
#[derive(Clone, Debug, Default)]
pub struct Signals {
    pub os: &'static str,
    pub lang: String,
    /// Fraction of the startup disk in use, if known.
    pub disk_full: Option<f32>,
    pub external_drive: bool,
    pub interests: HashSet<Interest>,
}

impl Signals {
    fn detect() -> Signals {
        let os = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        };
        Signals { os, lang: detect_language(), ..Default::default() }
    }
}

/// Two-letter UI language of the OS.
fn detect_language() -> String {
    let from_env = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| v.len() >= 2 && v != "C" && v != "POSIX");
    #[allow(unused_mut)]
    let mut raw = from_env;
    #[cfg(target_os = "macos")]
    if raw.is_none() {
        // Apps launched from Finder have no LANG.
        raw = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    }
    raw.map(|r| r.chars().take(2).collect::<String>().to_lowercase()).filter(|l| l.len() == 2).unwrap_or_else(|| "en".into())
}

/// The ad to show plus the human-readable reasons it was picked.
#[derive(Clone, Debug)]
pub struct Choice {
    pub ad: Ad,
    pub reasons: Vec<String>,
}

/// Local evaluation of an ad's rules. Returns the reasons when it matches.
pub fn evaluate(target: &Target, s: &Signals, personalized: bool) -> Option<Vec<String>> {
    if target.is_personal() && !personalized {
        return None;
    }
    let mut why = Vec::new();
    if !target.os.is_empty() {
        if !target.os.iter().any(|o| o == s.os) {
            return None;
        }
        why.push(format!("you use {}", match s.os { "macos" => "macOS", "windows" => "Windows", _ => "Linux" }));
    }
    if !target.lang.is_empty() {
        if !target.lang.iter().any(|l| *l == s.lang) {
            return None;
        }
        why.push(format!("your system language is \"{}\"", s.lang));
    }
    if !target.interests.is_empty() {
        let hit: Vec<&Interest> = target.interests.iter().filter(|i| s.interests.contains(i)).collect();
        if hit.is_empty() {
            return None;
        }
        why.push(format!("your disk suggests: {}", hit.iter().map(|i| i.label()).collect::<Vec<_>>().join(", ")));
    }
    if let Some(min) = target.disk_full_above {
        match s.disk_full {
            Some(f) if f >= min => why.push(format!("your startup disk is {:.0}% full", f * 100.0)),
            _ => return None,
        }
    }
    if let Some(want) = target.external_drive {
        if want != s.external_drive {
            return None;
        }
        why.push(if want { "an external drive is connected".into() } else { "no external drive is connected".into() });
    }
    Some(why)
}

/// Weighted pick, deterministic for a given seed (stable during one launch).
fn pick<'a>(candidates: &'a [(Ad, Vec<String>)], seed: u64) -> Option<&'a (Ad, Vec<String>)> {
    let total: u64 = candidates.iter().map(|c| c.0.weight.max(1) as u64).sum();
    if total == 0 {
        return None;
    }
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17) % total;
    for c in candidates {
        let w = c.0.weight.max(1) as u64;
        if x < w {
            return Some(c);
        }
        x -= w;
    }
    candidates.last()
}

pub struct Sponsors {
    catalog: Arc<Mutex<Vec<Ad>>>,
    signals: Arc<Mutex<Signals>>,
    seed: u64,
    /// (ad id, placement) already counted during this launch.
    seen: Mutex<HashSet<(String, &'static str)>>,
    pending: Arc<Mutex<wire::PendingCounts>>,
}

impl Sponsors {
    /// Downloads the catalog and, if allowed, flushes yesterday's view totals.
    pub fn start(measurement: bool) -> Sponsors {
        let catalog = Arc::new(Mutex::new(Vec::new()));
        let signals = Arc::new(Mutex::new(Signals { os: "", ..Default::default() }));
        let pending = Arc::new(Mutex::new(wire::PendingCounts::load()));
        let (c, s, p) = (catalog.clone(), signals.clone(), pending.clone());
        std::thread::spawn(move || {
            let detected = Signals::detect();
            {
                let mut sig = s.lock().unwrap();
                sig.os = detected.os;
                sig.lang = detected.lang;
            }
            if let Some(ads) = wire::fetch_catalog() {
                *c.lock().unwrap() = ads;
            }
            if measurement {
                wire::report_counts_if_due(&p);
            }
        });
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
        Sponsors { catalog, signals, seed, seen: Mutex::new(HashSet::new()), pending }
    }

    pub fn set_disk(&self, full: Option<f32>, external_drive: bool) {
        let mut s = self.signals.lock().unwrap();
        s.disk_full = full;
        s.external_drive = external_drive;
    }

    /// Infers interests from a finished scan in the background (on-device only).
    pub fn learn_from_scan(&self, tree: crate::scan::SharedTree) {
        let signals = self.signals.clone();
        std::thread::spawn(move || {
            let found = interests::infer(&tree.read().unwrap());
            signals.lock().unwrap().interests.extend(found);
        });
    }

    /// Forgets everything inferred (used when personalization is switched off).
    pub fn forget_interests(&self) {
        self.signals.lock().unwrap().interests.clear();
    }

    pub fn signals(&self) -> Signals {
        self.signals.lock().unwrap().clone()
    }

    /// Chooses the ad to display, locally.
    pub fn choose(&self, personalized: bool) -> Choice {
        let signals = self.signals.lock().unwrap().clone();
        let catalog = self.catalog.lock().unwrap();
        let candidates: Vec<(Ad, Vec<String>)> =
            catalog.iter().filter_map(|ad| evaluate(&ad.target, &signals, personalized).map(|why| (ad.clone(), why))).collect();
        match pick(&candidates, self.seed) {
            Some((ad, why)) => Choice { ad: ad.clone(), reasons: why.clone() },
            None => Choice { ad: Ad::house(), reasons: Vec::new() },
        }
    }

    /// Counts a view (at most once per ad and placement per launch).
    pub fn record_view(&self, ad: &Ad, placement: &'static str, measurement: bool) {
        if ad.is_house() || !measurement {
            return;
        }
        if self.seen.lock().unwrap().insert((ad.id.clone(), placement)) {
            let mut p = self.pending.lock().unwrap();
            p.add(&ad.id, placement);
            p.save();
        }
    }

    /// Where a click should go.
    pub fn click_url(&self, ad: &Ad, placement: &'static str, measurement: bool) -> String {
        if ad.is_house() || !measurement {
            ad.url.clone()
        } else {
            wire::click_redirect(&ad.id, placement)
        }
    }

    /// Views waiting to be reported (shown in Settings, for transparency).
    pub fn pending_views(&self) -> u32 {
        self.pending.lock().unwrap().totals().values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig() -> Signals {
        Signals { os: "macos", lang: "it".into(), disk_full: Some(0.93), external_drive: false, interests: [Interest::Developer].into() }
    }

    #[test]
    fn matching_rules() {
        let t = Target { interests: vec![Interest::Developer, Interest::Gamer], os: vec!["macos".into()], disk_full_above: Some(0.9), ..Default::default() };
        let why = evaluate(&t, &sig(), true).unwrap();
        assert_eq!(why.len(), 3);
        // personal rules never apply when personalization is off
        assert!(evaluate(&t, &sig(), false).is_none());
        // plain context still works without personalization
        let t2 = Target { lang: vec!["it".into()], ..Default::default() };
        assert!(evaluate(&t2, &sig(), false).is_some());
        let t3 = Target { interests: vec![Interest::Gamer], ..Default::default() };
        assert!(evaluate(&t3, &sig(), true).is_none());
    }

    #[test]
    fn weighted_pick_is_stable() {
        let mk = |id: &str, w| (Ad { id: id.into(), weight: w, ..Ad::house() }, vec![]);
        let c = vec![mk("a", 1), mk("b", 5)];
        let first = pick(&c, 42).unwrap().0.id.clone();
        assert_eq!(pick(&c, 42).unwrap().0.id, first);
    }
}
