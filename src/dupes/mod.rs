//! Duplicate files: the groups of identical files found in the places being
//! compared, which copy of each to keep, and the "always keep" rules.
//!
//! [`engine`] finds them; [`mock`] makes up a library of photos, videos and
//! project files for the demo and screenshots.

pub mod engine;
pub mod hash;
pub mod remove;
pub mod walk;

use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Photo,
    Raw,
    Video,
    Audio,
    Other,
}

impl Media {
    pub fn is_visual(self) -> bool {
        matches!(self, Media::Photo | Media::Raw | Media::Video)
    }

    /// What a file is, from its extension.
    pub fn of(name: &str) -> Media {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "jpg" | "jpeg" | "heic" | "heif" | "png" | "gif" | "webp" | "tif" | "tiff" | "bmp" | "avif" | "jxl" => Media::Photo,
            "arw" | "cr2" | "cr3" | "nef" | "nrw" | "raf" | "orf" | "rw2" | "dng" | "pef" | "srw" | "x3f" | "3fr" | "iiq" | "erf" | "rwl" => Media::Raw,
            "mp4" | "mov" | "m4v" | "avi" | "mkv" | "mts" | "m2ts" | "3gp" | "webm" | "insv" | "braw" | "r3d" | "mxf" | "wmv" | "mpg" | "mpeg" => Media::Video,
            "wav" | "mp3" | "aac" | "m4a" | "flac" | "aif" | "aiff" | "ogg" | "opus" | "wma" => Media::Audio,
            _ => Media::Other,
        }
    }
}

/// Libraries managed by an app (Photos, Final Cut, Lightroom…): their files
/// are listed, so copies elsewhere are found, but never removed from inside,
/// which would break the library.
pub fn in_library(path: &str) -> bool {
    const LIBS: &[&str] = &[".photoslibrary", ".fcpbundle", ".imovielibrary", ".musiclibrary", ".tvlibrary", ".aplibrary", ".lrdata", ".cocatalog", ".logicx", ".band", ".dvdmedia"];
    path.split(['/', '\\']).any(|c| {
        let c = c.to_ascii_lowercase();
        LIBS.iter().any(|l| c.ends_with(l))
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaceKind {
    Folder,
    Disk,
    Server,
    Cloud,
}

/// A place added to the comparison.
#[derive(Clone, Debug)]
pub struct Place {
    pub kind: PlaceKind,
    pub label: String,
    pub path: String,
    /// What the finder lists (none for made-up places).
    pub spec: Option<engine::Spec>,
}

/// One copy of a duplicated file.
#[derive(Clone, Debug)]
pub struct Copy {
    pub path: String,
    /// Index into the places.
    pub place: usize,
    /// Last modified, as (year, month, day).
    pub modified: (u16, u8, u8),
    /// Last modified, in seconds since the epoch (0: unknown).
    pub mtime: i64,
    /// Chosen for removal.
    pub marked: bool,
    /// Matches an "always keep" rule.
    pub protected: bool,
    /// Inside an app's library: never removed (see [`in_library`]).
    pub locked: bool,
    /// An APFS clone of another copy: removing it frees nothing.
    pub shares: bool,
}

impl Copy {
    pub fn new(path: String, place: usize, modified: (u16, u8, u8)) -> Copy {
        let locked = in_library(&path);
        Copy { path, place, modified, mtime: 0, marked: false, protected: false, locked, shares: false }
    }
    pub fn name(&self) -> &str {
        self.path.rsplit_once(['/', '\\']).map(|(_, n)| n).unwrap_or(&self.path)
    }
    /// Always kept, whatever the selection says.
    pub fn kept(&self) -> bool {
        self.protected || self.locked
    }
}

/// (year, month, day) of a time in seconds since the epoch, in UTC.
pub fn ymd(secs: i64) -> (u16, u8, u8) {
    let z = secs.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y.clamp(0, 9999) as u16, m as u8, d as u8)
}

#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub dims: Option<(u32, u32)>,
    /// Seconds.
    pub duration: Option<u32>,
    pub device: Option<String>,
    pub taken: Option<(u16, u8, u8)>,
}

/// Files with the same content.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: u32,
    pub name: String,
    pub ext: String,
    pub media: Media,
    /// Size of one copy.
    pub size: u64,
    pub copies: Vec<Copy>,
    /// The copy the rules elected to keep.
    pub keeper: usize,
    /// The user changed the selection: rules no longer touch it.
    pub manual: bool,
    pub meta: Meta,
    /// Content fingerprint, in hex (shown shortened).
    pub digest: String,
    /// Seed for the made-up preview.
    pub seed: u32,
    /// Fraction of the scan at which it is found.
    pub found_at: f32,
}

impl Group {
    pub fn marked(&self) -> usize {
        self.copies.iter().filter(|c| c.marked).count()
    }
    /// Bytes freed by removing the marked copies (clones free nothing).
    pub fn freed(&self) -> u64 {
        self.size * self.copies.iter().filter(|c| c.marked && !c.shares).count() as u64
    }
    /// Bytes taken by the copies beyond the first.
    pub fn extra(&self) -> u64 {
        self.size * (self.copies.len() as u64).saturating_sub(1)
    }
    /// Every copy is marked: nothing would be left.
    pub fn none_kept(&self) -> bool {
        self.copies.iter().all(|c| c.marked)
    }
}

/// "Always keep the copies in these places."
#[derive(Clone, Debug)]
pub struct Rule {
    pub pattern: String,
    pub on: bool,
}

/// Which copy to keep when no rule decides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prefer {
    Oldest,
    Newest,
    ShortestPath,
}

impl Prefer {
    pub fn next(self) -> Prefer {
        match self {
            Prefer::Oldest => Prefer::Newest,
            Prefer::Newest => Prefer::ShortestPath,
            Prefer::ShortestPath => Prefer::Oldest,
        }
    }
}

/// Elects the copy to keep and marks the others, unless the user already
/// chose by hand. Copies matched by a rule or inside a library are never
/// marked, and one of them is the copy kept.
pub fn apply_rules(g: &mut Group, rules: &[Rule], prefer: Prefer) {
    for c in g.copies.iter_mut() {
        c.protected = rules.iter().any(|r| r.on && glob_match(&r.pattern, &c.path));
        if c.kept() {
            c.marked = false;
        }
    }
    let any_kept = g.copies.iter().any(|c| c.kept());
    let candidates = (0..g.copies.len()).filter(|i| !any_kept || g.copies[*i].kept());
    let keeper = match prefer {
        Prefer::Oldest => candidates.min_by_key(|i| (g.copies[*i].modified, g.copies[*i].path.len())),
        Prefer::Newest => candidates.max_by_key(|i| g.copies[*i].modified),
        Prefer::ShortestPath => candidates.min_by_key(|i| g.copies[*i].path.len()),
    };
    g.keeper = keeper.unwrap_or(0);
    if g.manual {
        return;
    }
    for (i, c) in g.copies.iter_mut().enumerate() {
        c.marked = i != g.keeper && !c.kept();
    }
}

/// Glob match on a whole path, ignoring case: `**` crosses folders, `*` and
/// `?` stay within one. A pattern without wildcards matches the folder it
/// names and everything inside it.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim().trim_end_matches('/');
    if pattern.is_empty() {
        return false;
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = path.to_lowercase().chars().collect();
    if !p.iter().any(|c| matches!(c, '*' | '?')) {
        return s.starts_with(&p) && (s.len() == p.len() || s[p.len()] == '/');
    }
    glob(&p, &s)
}

fn glob(p: &[char], s: &[char]) -> bool {
    match p.first() {
        None => s.is_empty(),
        // `**/` is any number of whole folders, none included
        Some('*') if p.get(1) == Some(&'*') && p.get(2) == Some(&'/') => {
            (0..=s.len()).any(|i| (i == 0 || s[i - 1] == '/') && glob(&p[3..], &s[i..]))
        }
        Some('*') if p.get(1) == Some(&'*') => (0..=s.len()).any(|i| glob(&p[2..], &s[i..])),
        Some('*') => (0..=s.len()).take_while(|i| *i == 0 || s[i - 1] != '/').any(|i| glob(&p[1..], &s[i..])),
        Some('?') => s.first().is_some_and(|c| *c != '/') && glob(&p[1..], &s[1..]),
        Some(c) => s.first() == Some(c) && glob(&p[1..], &s[1..]),
    }
}

/// Progress of a comparison.
pub struct Scan {
    pub started: Instant,
    pub length: Duration,
}

impl Scan {
    pub fn new() -> Scan {
        Scan { started: Instant::now(), length: Duration::from_millis(9000) }
    }
    pub fn fraction(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() / self.length.as_secs_f32()).min(1.0)
    }
    pub fn is_done(&self) -> bool {
        self.fraction() >= 1.0
    }
}

pub mod mock {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
        fn range(&mut self, lo: u64, hi: u64) -> u64 {
            lo + self.below(hi - lo + 1)
        }
        fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
            &v[self.below(v.len() as u64) as usize]
        }
    }

    pub fn places() -> Vec<Place> {
        vec![
            Place { kind: PlaceKind::Folder, label: "Pictures".into(), path: "~/Pictures".into(), spec: None },
            Place { kind: PlaceKind::Disk, label: "T7 Shield".into(), path: "/Volumes/T7 Shield".into(), spec: None },
            Place { kind: PlaceKind::Server, label: "nas.local".into(), path: "nas.local:/volume1".into(), spec: None },
        ]
    }

    pub fn rules() -> Vec<Rule> {
        vec![
            Rule { pattern: "~/Pictures/Photos Library.photoslibrary/**".into(), on: true },
            Rule { pattern: "**/Archive/**".into(), on: true },
            Rule { pattern: "nas.local:/volume1/backup".into(), on: false },
        ]
    }

    /// Folders where copies end up, per place.
    const DIRS: &[(usize, &str)] = &[
        (0, "~/Pictures/Imports/2024-06-12"),
        (0, "~/Pictures/Photos Library.photoslibrary/originals/A"),
        (0, "~/Pictures/Da sistemare"),
        (0, "~/Pictures/Export/Instagram"),
        (1, "/Volumes/T7 Shield/Archive/2024/Giugno"),
        (1, "/Volumes/T7 Shield/Progetti/Brand Shoot/footage"),
        (1, "/Volumes/T7 Shield/Progetti/Brand Shoot copy/footage"),
        (1, "/Volumes/T7 Shield/Import SD/DCIM/100MSDCF"),
        (2, "nas.local:/volume1/photos/2024/06"),
        (2, "nas.local:/volume1/backup/macbook/Pictures"),
    ];

    struct Kind {
        media: Media,
        name: fn(&mut Rng) -> String,
        ext: &'static str,
        size: (u64, u64),
        dims: Option<(u32, u32)>,
        duration: Option<(u32, u32)>,
        device: Option<&'static str>,
        weight: u64,
    }

    const MB: u64 = 1_000_000;

    fn kinds() -> Vec<Kind> {
        vec![
            Kind { media: Media::Photo, name: |r| format!("IMG_{:04}.HEIC", r.range(1000, 9999)), ext: "HEIC", size: (2 * MB, 5 * MB), dims: Some((4032, 3024)), duration: None, device: Some("iPhone 15 Pro"), weight: 10 },
            Kind { media: Media::Raw, name: |r| format!("DSC{:05}.ARW", r.range(1000, 99999)), ext: "ARW", size: (24 * MB, 49 * MB), dims: Some((6000, 4000)), duration: None, device: Some("Sony α7 IV"), weight: 8 },
            Kind { media: Media::Raw, name: |r| format!("_MG_{:04}.CR3", r.range(1000, 9999)), ext: "CR3", size: (22 * MB, 38 * MB), dims: Some((6000, 4000)), duration: None, device: Some("Canon EOS R6"), weight: 4 },
            Kind { media: Media::Photo, name: |r| format!("PXL_202406{:02}_{:06}.jpg", r.range(1, 28), r.range(100000, 999999)), ext: "JPG", size: (3 * MB, 7 * MB), dims: Some((4080, 3072)), duration: None, device: Some("Pixel 8"), weight: 5 },
            Kind { media: Media::Photo, name: |r| format!("Screenshot 2024-06-{:02} at {}.{:02}.png", r.range(1, 28), r.range(9, 22), r.range(0, 59)), ext: "PNG", size: (MB / 2, 4 * MB), dims: Some((3024, 1964)), duration: None, device: None, weight: 3 },
            Kind { media: Media::Video, name: |r| format!("GX01{:04}.MP4", r.range(1000, 9999)), ext: "MP4", size: (900 * MB, 4000 * MB), dims: Some((3840, 2160)), duration: Some((40, 520)), device: Some("GoPro HERO12"), weight: 4 },
            Kind { media: Media::Video, name: |r| format!("C{:04}.MP4", r.range(1, 999)), ext: "MP4", size: (1500 * MB, 9000 * MB), dims: Some((3840, 2160)), duration: Some((30, 900)), device: Some("Sony FX3"), weight: 3 },
            Kind { media: Media::Video, name: |r| format!("IMG_{:04}.MOV", r.range(1000, 9999)), ext: "MOV", size: (80 * MB, 1200 * MB), dims: Some((1920, 1080)), duration: Some((8, 180)), device: Some("iPhone 15 Pro"), weight: 4 },
            Kind { media: Media::Video, name: |r| format!("{}_v{}.mov", r.pick(&["final_export", "teaser", "reel_cut", "intervista"]), r.range(1, 6)), ext: "MOV", size: (2000 * MB, 14000 * MB), dims: Some((3840, 2160)), duration: Some((30, 240)), device: None, weight: 2 },
            Kind { media: Media::Audio, name: |r| format!("{}_{:02}.wav", r.pick(&["ZOOM", "VO", "ambient"]), r.range(1, 40)), ext: "WAV", size: (30 * MB, 400 * MB), dims: None, duration: Some((20, 1800)), device: Some("Zoom H5"), weight: 2 },
            Kind { media: Media::Other, name: |r| format!("{}.psd", r.pick(&["mockup", "cover", "thumbnail_yt", "poster A3"])), ext: "PSD", size: (80 * MB, 600 * MB), dims: None, duration: None, device: None, weight: 2 },
            Kind { media: Media::Other, name: |r| format!("{}.zip", r.pick(&["Brand Shoot - consegna", "LUTs pack", "fonts"])), ext: "ZIP", size: (40 * MB, 3000 * MB), dims: None, duration: None, device: None, weight: 1 },
        ]
    }

    /// Made-up duplicate groups, the same every time.
    pub fn groups(places: usize) -> Vec<Group> {
        let kinds = kinds();
        let total_weight: u64 = kinds.iter().map(|k| k.weight).sum();
        let mut r = Rng(0x5eed_d00d_cafe_f00d);
        let mut out = Vec::new();
        let dirs: Vec<&(usize, &str)> = DIRS.iter().filter(|(p, _)| *p < places).collect();
        for id in 0..140u32 {
            let mut w = r.below(total_weight);
            let k = kinds.iter().find(|k| if w < k.weight { true } else { w -= k.weight; false }).unwrap();
            let name = (k.name)(&mut r);
            let n = (2 + r.below(10).saturating_sub(6)).min(4) as usize; // mostly 2, some 3–4
            let mut picked: Vec<usize> = Vec::new();
            while picked.len() < n.min(dirs.len()) {
                let d = r.below(dirs.len() as u64) as usize;
                if !picked.contains(&d) {
                    picked.push(d);
                }
            }
            let base = (2024u16, r.range(1, 6) as u8, r.range(1, 28) as u8);
            let copies = picked
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let (place, dir) = *dirs[*d];
                    let later = if i == 0 { 0 } else { r.range(0, 4) as u8 };
                    let month = (base.1 + later).min(12);
                    Copy::new(format!("{dir}/{name}"), place, (base.0, month, base.2))
                })
                .collect();
            out.push(Group {
                id,
                name,
                ext: k.ext.to_string(),
                media: k.media,
                size: r.range(k.size.0, k.size.1),
                copies,
                keeper: 0,
                manual: false,
                meta: Meta {
                    dims: k.dims,
                    duration: k.duration.map(|(a, b)| r.range(a as u64, b as u64) as u32),
                    device: k.device.map(String::from),
                    taken: Some(base),
                },
                digest: format!("{:016x}{:016x}", r.next(), r.next()),
                seed: r.next() as u32,
                found_at: 0.12 + (r.below(1000) as f32 / 1000.0) * 0.86,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("**/Archive/**", "/Volumes/T7/Archive/2024/a.jpg"));
        assert!(!glob_match("**/Archive/**", "/Volumes/T7/Archived/a.jpg"));
        assert!(glob_match("~/Pictures", "~/Pictures/a/b.jpg"));
        assert!(!glob_match("~/Pictures", "~/Pictures2/b.jpg"));
        assert!(glob_match("*.heic", "img.HEIC"));
        assert!(!glob_match("*.heic", "dir/img.heic"));
        assert!(glob_match("**/*.heic", "dir/img.heic"));
        assert!(glob_match("**/*.heic", "img.heic"));
        assert!(glob_match("/a/?/c", "/a/b/c"));
        assert!(!glob_match("", "/a"));
    }

    fn group(paths: &[(&str, (u16, u8, u8))]) -> Group {
        let mut g = mock::groups(1).remove(0);
        g.copies = paths.iter().map(|(p, d)| Copy::new(p.to_string(), 0, *d)).collect();
        g.manual = false;
        g
    }

    #[test]
    fn rules_keep_protected_copies() {
        let mut g = group(&[("/a/x.jpg", (2024, 1, 1)), ("/b/x.jpg", (2024, 2, 1)), ("/c/x.jpg", (2024, 3, 1))]);
        apply_rules(&mut g, &[], Prefer::Oldest);
        assert_eq!(g.keeper, 0);
        assert_eq!(g.marked(), 2);
        apply_rules(&mut g, &[], Prefer::Newest);
        assert_eq!(g.keeper, 2);
        let rules = vec![Rule { pattern: "/b".into(), on: true }];
        apply_rules(&mut g, &rules, Prefer::Oldest);
        assert_eq!(g.keeper, 1);
        assert!(!g.copies[1].marked);
        assert_eq!(g.marked(), 2);
        // a rule turned off no longer protects
        apply_rules(&mut g, &[Rule { pattern: "/b".into(), on: false }], Prefer::Oldest);
        assert_eq!(g.keeper, 0);
    }

    #[test]
    fn libraries_are_never_marked() {
        let mut g = group(&[("/p/Imports/IMG_1.HEIC", (2024, 1, 1)), ("/p/Photos Library.photoslibrary/originals/A/IMG_1.HEIC", (2024, 5, 1))]);
        apply_rules(&mut g, &[], Prefer::Oldest);
        assert!(g.copies[1].locked && !g.copies[1].marked);
        assert_eq!(g.keeper, 1);
        assert!(g.copies[0].marked);
        assert_eq!(ymd(0), (1970, 1, 1));
        assert_eq!(ymd(1718180430), (2024, 6, 12));
        assert_eq!(Media::of("a.ARW"), Media::Raw);
    }
}
