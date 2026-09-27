//! `MARKETING_DEMO=1`: realistic but entirely fake disks and folders, used to
//! record screenshots and GIFs without showing anyone's real files.
//! Nothing here touches the filesystem.

use super::{Progress, ScanHandle};
use crate::tree::{Kind, Source, Tree, F_DONE, NONE};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub fn enabled() -> bool {
    std::env::var_os("MARKETING_DEMO").is_some()
}

const GB: u64 = 1_000_000_000;
const MB: u64 = 1_000_000;

const STARTUP_TOTAL: u64 = 994_662_584_320;
const HIDDEN: u64 = 12 * GB;

/// Bytes of the fake startup disk's files (computed once).
fn startup_files() -> u64 {
    static S: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *S.get_or_init(|| startup_disk_files("Macintosh HD").get(0).size)
}

/// Fake volumes: (name, mount, total, available, removable).
pub fn volumes() -> Vec<(&'static str, &'static str, u64, u64, bool)> {
    vec![
        ("Macintosh HD", "/", STARTUP_TOTAL, STARTUP_TOTAL - startup_files() - HIDDEN, false),
        ("Photo Archive", "/Volumes/Photo Archive", 2_000_398_934_016, 1_214_000_000_000, true),
        ("Time Machine", "/Volumes/Time Machine", 4_000_787_030_016, 2_950_000_000_000, true),
    ]
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[(self.next() % v.len() as u64) as usize]
    }
}

const DIRS: &[&str] = &[
    "assets", "cache", "build", "src", "data", "images", "logs", "v2", "archive", "exports", "tmp", "lib", "resources",
    "backup", "media", "renders", "samples", "models", "packages", "vendor", "dist", "shared", "old", "2024", "2025",
    "drafts", "raw", "thumbnails", "fonts", "plugins",
];
const EXT: &[&str] = &["mov", "mp4", "zip", "dmg", "psd", "raw", "wav", "bin", "db", "pack", "tar.gz", "sqlite", "jpg", "pdf"];

struct Builder {
    t: Tree,
    rng: Rng,
}

impl Builder {
    fn dir(&mut self, parent: u32, name: &str) -> u32 {
        let id = self.t.add_child(parent, name, Kind::Dir, 0, 0);
        self.t.get_mut(id).flags |= F_DONE;
        id
    }
    fn file(&mut self, parent: u32, name: &str, size: u64) {
        self.t.add_child_sized(parent, name, Kind::File, size, 1);
    }
    /// Random but plausible subtree of roughly `size` bytes.
    fn blob(&mut self, parent: u32, size: u64, depth: u32) {
        if depth == 0 || size < 60 * MB {
            let big = (self.rng.next() % 4) as usize;
            let mut rest = size;
            for _ in 0..big {
                let s = (rest as f64 * (0.25 + self.rng.f() * 0.4)) as u64;
                if s < 2 * MB {
                    break;
                }
                let name = format!("{}-{:03}.{}", self.rng.pick(DIRS), self.rng.next() % 999, self.rng.pick(EXT));
                self.file(parent, &name, s);
                rest -= s;
            }
            if rest > 0 {
                let n = (rest / (3 * MB)).clamp(3, 4000) as u32;
                self.t.add_child_sized(parent, "\u{2026}", Kind::SmallFiles, rest, n);
            }
            return;
        }
        let k = 2 + (self.rng.next() % 6) as usize;
        let weights: Vec<f64> = (0..k).map(|_| self.rng.f().powf(2.2) + 0.02).collect();
        let sum: f64 = weights.iter().sum();
        let mut used = std::collections::HashSet::new();
        for w in weights {
            let mut name = self.rng.pick(DIRS).to_string();
            while !used.insert(name.clone()) {
                name = format!("{}-{}", self.rng.pick(DIRS), self.rng.next() % 99);
            }
            let id = self.dir(parent, &name);
            self.blob(id, (size as f64 * w / sum) as u64, depth - 1);
        }
    }
    /// A named folder filled with a random subtree.
    fn folder(&mut self, parent: u32, name: &str, size: u64, depth: u32) -> u32 {
        let id = self.dir(parent, name);
        self.blob(id, size, depth);
        id
    }
}

/// The template tree for the demo startup disk, including hidden space.
fn startup_disk(name: &str) -> Tree {
    let mut t = startup_disk_files(name);
    t.add_child_sized(0, "Hidden space", Kind::Hidden, HIDDEN, 0);
    t
}

/// Files and folders of the demo startup disk (~640 GB).
fn startup_disk_files(name: &str) -> Tree {
    let mut b = Builder { t: Tree::new("/", name, Source::Demo), rng: Rng(0x5eed_cafe_f00d) };
    let r = 0;
    let users = b.dir(r, "Users");
    let alex = b.dir(users, "alex");
    let movies = b.dir(alex, "Movies");
    b.folder(movies, "Final Cut Library.fcpbundle", 88 * GB, 4);
    b.folder(movies, "Vacation 2025", 32 * GB, 3);
    b.folder(movies, "Screen Recordings", 19 * GB, 2);
    let pics = b.dir(alex, "Pictures");
    b.folder(pics, "Photos Library.photoslibrary", 78 * GB, 4);
    b.folder(pics, "Lightroom", 18 * GB, 3);
    let lib = b.dir(alex, "Library");
    b.folder(lib, "Caches", 22 * GB, 3);
    let support = b.dir(lib, "Application Support");
    b.folder(support, "Steam", 21 * GB, 3);
    b.folder(support, "Adobe", 8 * GB, 3);
    b.folder(support, "Spotify", 6 * GB, 2);
    b.folder(support, "Slack", 3 * GB, 2);
    b.folder(support, "Google", 5 * GB, 2);
    b.folder(lib, "Containers", 18 * GB, 3);
    let dev = b.dir(lib, "Developer");
    b.folder(dev, "Xcode", 14 * GB, 3);
    b.folder(dev, "CoreSimulator", 10 * GB, 3);
    let code = b.dir(alex, "Developer");
    let web = b.dir(code, "web-app");
    b.folder(web, "node_modules", 6 * GB, 4);
    b.folder(web, "src", 400 * MB, 2);
    let ml = b.dir(code, "ml-experiments");
    b.folder(ml, "datasets", 28 * GB, 3);
    b.folder(ml, "checkpoints", 12 * GB, 2);
    let game = b.dir(code, "game-engine");
    b.folder(game, "target", 9 * GB, 3);
    b.folder(game, "assets", 7 * GB, 3);
    b.folder(code, "mobile-app", 5 * GB, 3);
    let dl = b.dir(alex, "Downloads");
    b.file(dl, "Xcode_17.xip", 11 * GB);
    b.file(dl, "ubuntu-24.04-desktop-amd64.iso", 6_100 * MB);
    b.file(dl, "Figma-Installer.dmg", 380 * MB);
    b.folder(dl, "Old Projects.zip.expanded", 14 * GB, 3);
    b.folder(alex, "Documents", 18 * GB, 3);
    let music = b.dir(alex, "Music");
    b.folder(music, "Logic Projects", 16 * GB, 3);
    b.folder(music, "Samples", 6 * GB, 2);
    b.folder(alex, "Desktop", 4 * GB, 2);
    b.folder(users, "Shared", 3 * GB, 2);

    let apps = b.dir(r, "Applications");
    for (n, s) in [
        ("Xcode.app", 12.4), ("Final Cut Pro.app", 6.1), ("Adobe Photoshop 2026", 7.2), ("Logic Pro.app", 4.3),
        ("Docker.app", 3.1), ("Microsoft Word.app", 2.9), ("Microsoft Excel.app", 2.7), ("Blender.app", 1.9),
        ("Android Studio.app", 2.4), ("Steam.app", 0.6), ("Figma.app", 0.5), ("Spotify.app", 0.4),
        ("Visual Studio Code.app", 0.6), ("Slack.app", 0.5), ("Google Chrome.app", 1.3), ("Zoom.app", 0.3),
    ] {
        b.folder(apps, n, (s * GB as f64) as u64, 3);
    }
    for i in 0..40 {
        let name = format!("{}.app", ["Notes Pro", "PixelDraw", "Taskly", "Mailbird", "Cleanup", "Beatbox", "ScreenKit", "Markdown", "Ferry", "Orbit"][i % 10].to_string() + if i >= 10 { &["", " 2", " X", " Plus"][i / 10] } else { "" });
        let s = 80 * MB + b.rng.next() % (900 * MB);
        b.folder(apps, &name, s, 2);
    }
    b.folder(r, "System", 32 * GB, 4);
    b.folder(r, "Library", 24 * GB, 3);
    let private = b.dir(r, "private");
    let var = b.dir(private, "var");
    let vm = b.dir(var, "vm");
    b.file(vm, "sleepimage", 8 * GB);
    b.file(vm, "swapfile0", 2 * GB);
    b.folder(var, "folders", 9 * GB, 3);
    b.folder(r, "opt", 9 * GB, 3);
    b.folder(r, "usr", 2 * GB, 2);
    b.t
}

fn generic_disk(name: &str, mount: &str, used: u64) -> Tree {
    let mut b = Builder { t: Tree::new(mount, name, Source::Demo), rng: Rng(name.len() as u64 * 7919 + 17) };
    let split = [("Photos", 0.46), ("Videos", 0.28), ("Projects", 0.14), ("Backups", 0.12)];
    for (n, w) in split {
        b.folder(0, n, (used as f64 * w) as u64, 4);
    }
    b.t
}

pub fn template(name: &str, mount: &str) -> Tree {
    if mount == "/" {
        startup_disk(name)
    } else {
        let used = volumes().iter().find(|v| v.1 == mount).map(|v| v.2 - v.3).unwrap_or(500 * GB);
        generic_disk(name, mount, used)
    }
}

/// Plays back a template as if it were being scanned (≈6 s), so the chart
/// composes itself live.
pub fn start(name: String, mount: String) -> ScanHandle {
    let progress = Arc::new(Progress::default());
    let tree = Arc::new(RwLock::new(Tree::new(&mount, &name, Source::Demo)));
    let (t, p) = (tree.clone(), progress.clone());
    std::thread::spawn(move || {
        let src = template(&name, &mount);
        p.expected.store(src.get(0).size, Ordering::Relaxed);
        p.set_status("Scanning");
        // breadth-first order, like the real scanner
        let mut order = Vec::with_capacity(src.nodes.len());
        let mut queue = std::collections::VecDeque::from([src.root]);
        while let Some(id) = queue.pop_front() {
            order.push(id);
            for c in src.children(id) {
                queue.push_back(c);
            }
        }
        let mut map = vec![NONE; src.nodes.len()];
        map[src.root as usize] = 0;
        let batches = 200usize;
        let per = order.len().div_ceil(batches).max(1);
        for chunk in order[1..].chunks(per) {
            if p.cancelled() {
                break;
            }
            {
                let mut tr = t.write().unwrap();
                for &id in chunk {
                    let n = src.get(id);
                    let parent = map[n.parent as usize];
                    let leaf = n.kind != Kind::Dir;
                    let nid = if leaf {
                        let nid = tr.add_child_sized(parent, &n.name, n.kind, n.size, n.files);
                        p.bytes.fetch_add(n.size, Ordering::Relaxed);
                        p.files.fetch_add(n.files as u64, Ordering::Relaxed);
                        nid
                    } else {
                        p.dirs.fetch_add(1, Ordering::Relaxed);
                        tr.add_child(parent, &n.name, Kind::Dir, 0, 0)
                    };
                    tr.get_mut(nid).flags = n.flags;
                    map[id as usize] = nid;
                }
            }
            std::thread::sleep(Duration::from_millis(28));
        }
        t.write().unwrap().get_mut(0).flags |= F_DONE;
        p.set_status("Scanned in 5.9s");
        p.done.store(true, Ordering::Relaxed);
    });
    ScanHandle { tree, progress }
}
