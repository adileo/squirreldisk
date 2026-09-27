//! **Everything SquirrelDisk ever sends about sponsors is in this file.**
//!
//! | When                        | Request                                   | Contains                          |
//! |-----------------------------|-------------------------------------------|-----------------------------------|
//! | at launch                   | `GET  /ads.json?v=<app version>`          | the app version, nothing else     |
//! | when you click the banner   | browser opens `GET /c/<ad id>?p=<place>`  | ad id + "home"/"panel"            |
//! | at most once a day          | `POST /impressions`                       | `{v, counts:[{ad, placement, n}]}`|
//!
//! No user id, device id, cookie, timestamp, path, file name, interest or disk
//! information is ever included. Interests are only used locally to choose
//! among ads that were all downloaded anyway.

use super::Ad;
use std::collections::HashMap;
use std::sync::Mutex;

pub const API: &str = "https://www.squirreldisk.com";

const UA: &str = concat!("SquirrelDisk/", env!("CARGO_PKG_VERSION"));

#[derive(serde::Deserialize)]
struct Catalog {
    #[serde(default)]
    ads: Vec<Ad>,
}

/// `GET /ads.json`: the same request for every user.
pub fn fetch_catalog() -> Option<Vec<Ad>> {
    let url = format!("{API}/ads.json?v={}", env!("CARGO_PKG_VERSION"));
    let mut resp = ureq::get(&url).header("User-Agent", UA).call().ok()?;
    let catalog: Catalog = resp.body_mut().read_json().ok()?;
    Some(
        catalog
            .ads
            .into_iter()
            // only safe links, short texts, and ids usable in a URL path
            .filter(|a| a.url.starts_with("https://") && !a.title.trim().is_empty())
            .filter(|a| !a.id.is_empty() && a.id.len() <= 64 && a.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .map(|mut a| {
                a.title = a.title.chars().take(80).collect();
                a.text = a.text.chars().take(160).collect();
                a
            })
            .collect(),
    )
}

/// `GET /c/<id>?p=<placement>`: opened in the browser; the server counts
/// the click and redirects to the sponsor. No cookie is set.
pub fn click_redirect(ad_id: &str, placement: &str) -> String {
    format!("{API}/c/{ad_id}?p={placement}")
}

/// Views counted locally and reported as daily totals.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct PendingCounts {
    /// "ad|placement" → views
    counts: HashMap<String, u32>,
    /// Day number (days since 1970) of the last successful report.
    last_report_day: u64,
}

fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

impl PendingCounts {
    fn path() -> Option<std::path::PathBuf> {
        crate::settings::config_dir().map(|d| d.join("sponsor_counts.json"))
    }
    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let (Some(p), Ok(s)) = (Self::path(), serde_json::to_string(self)) {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, s);
        }
    }
    pub fn add(&mut self, ad: &str, placement: &str) {
        *self.counts.entry(format!("{ad}|{placement}")).or_insert(0) += 1;
    }
    pub fn totals(&self) -> &HashMap<String, u32> {
        &self.counts
    }
}

/// The only body ever POSTed.
#[derive(serde::Serialize)]
struct ImpressionReport<'a> {
    v: &'a str,
    counts: Vec<Count<'a>>,
}

#[derive(serde::Serialize)]
struct Count<'a> {
    ad: &'a str,
    placement: &'a str,
    n: u32,
}

/// `POST /impressions`: at most once per day, only if there is something to say.
pub fn report_counts_if_due(pending: &Mutex<PendingCounts>) {
    let body = {
        let p = pending.lock().unwrap();
        if p.counts.is_empty() || p.last_report_day >= today() {
            return;
        }
        let counts = p
            .counts
            .iter()
            .filter_map(|(k, n)| k.split_once('|').map(|(ad, placement)| (ad.to_string(), placement.to_string(), *n)))
            .collect::<Vec<_>>();
        counts
    };
    let report = ImpressionReport {
        v: env!("CARGO_PKG_VERSION"),
        counts: body.iter().map(|(ad, placement, n)| Count { ad, placement, n: *n }).collect(),
    };
    let ok = ureq::post(&format!("{API}/impressions")).header("User-Agent", UA).send_json(&report).map(|r| r.status().is_success()).unwrap_or(false);
    if ok {
        let mut p = pending.lock().unwrap();
        p.counts.clear();
        p.last_report_day = today();
        p.save();
    }
}
