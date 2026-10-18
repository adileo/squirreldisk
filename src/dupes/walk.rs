//! Fast parallel listing of every file under a folder, with what is needed
//! to find duplicates: size, modification time and identity on disk.

use crate::scan::platform;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};

#[derive(Clone, Debug)]
pub struct Found {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u32,
    /// APFS: id of the data shared with exact clones of this file.
    pub clone: Option<u64>,
}

/// Folders never worth looking into: version control internals, dependency
/// caches, system bookkeeping and trash.
const SKIP_DIRS: &[&str] = &[
    ".git", ".hg", ".svn", "node_modules", ".Trash", ".Trashes", ".Spotlight-V100", ".fseventsd", ".DocumentRevisions-V100", ".TemporaryItems",
    "$RECYCLE.BIN", "System Volume Information", ".cache", "__pycache__",
];

/// Packages whose files belong together: deleting one breaks the whole.
const SKIP_PACKAGES: &[&str] = &[".app", ".framework", ".bundle", ".plugin", ".kext", ".appex", ".xpc", ".dSYM", ".xcarchive", ".pkg"];

fn skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name) || SKIP_PACKAGES.iter().any(|ext| name.len() > ext.len() && name.to_ascii_lowercase().ends_with(&ext.to_ascii_lowercase()))
}

pub struct Counters {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
}

impl Default for Counters {
    fn default() -> Self {
        Counters { files: AtomicU64::new(0), bytes: AtomicU64::new(0), errors: AtomicU64::new(0) }
    }
}

/// Lists `root` with `threads` workers and calls `emit` (from any of them)
/// for each regular file of at least `min_size` bytes. Stays on the device
/// of `root` and never follows links.
pub fn walk(root: &Path, min_size: u64, threads: usize, cancel: &AtomicBool, counters: &Counters, emit: &(dyn Fn(Found) + Sync)) {
    let root_dev = platform::device_of(root);
    let queue: Mutex<(VecDeque<PathBuf>, usize)> = Mutex::new((VecDeque::from([root.to_path_buf()]), 0));
    let cv = Condvar::new();
    let work = || loop {
        let dir = {
            let mut st = queue.lock().unwrap();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    cv.notify_all();
                    return;
                }
                if let Some(d) = st.0.pop_front() {
                    st.1 += 1;
                    break d;
                }
                if st.1 == 0 {
                    cv.notify_all();
                    return;
                }
                st = cv.wait(st).unwrap();
            }
        };
        let mut subdirs = Vec::new();
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list(&dir, root_dev, min_size, counters, emit, &mut subdirs))).unwrap_or(false);
        if !ok {
            counters.errors.fetch_add(1, Ordering::Relaxed);
        }
        let mut st = queue.lock().unwrap();
        st.0.extend(subdirs);
        st.1 -= 1;
        drop(st);
        cv.notify_all();
    };
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(work);
        }
    });
}

fn list(dir: &Path, root_dev: Option<u64>, min_size: u64, counters: &Counters, emit: &(dyn Fn(Found) + Sync), subdirs: &mut Vec<PathBuf>) -> bool {
    #[cfg(target_os = "macos")]
    if std::env::var_os("SQD_NO_BULK").is_none() {
        if let Ok(entries) = platform::list_bulk(dir) {
            use platform::RawKind;
            for e in entries {
                match e.kind {
                    RawKind::Dir if !skip_dir(&e.name) && root_dev.is_none_or(|d| d == e.dev) => subdirs.push(dir.join(&e.name)),
                    RawKind::File if !e.dataless => {
                        counters.files.fetch_add(1, Ordering::Relaxed);
                        counters.bytes.fetch_add(e.len, Ordering::Relaxed);
                        if e.len >= min_size.max(1) {
                            let path = dir.join(&e.name).to_string_lossy().into_owned();
                            emit(Found { path, size: e.len, mtime: e.mtime, dev: e.dev, ino: e.ino, nlink: e.nlink, clone: e.clone });
                        }
                    }
                    _ => {}
                }
            }
            return true;
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let Ok(md) = e.path().symlink_metadata() else { continue };
        let name = e.file_name();
        if md.is_dir() {
            let same_dev = match (root_dev, id_of(&md)) {
                (Some(r), Some((d, _, _))) => r == d,
                _ => true,
            };
            if same_dev && !skip_dir(&name.to_string_lossy()) {
                subdirs.push(e.path());
            }
        } else if md.is_file() {
            #[cfg(target_os = "macos")]
            if platform::is_dataless(&md) {
                continue;
            }
            counters.files.fetch_add(1, Ordering::Relaxed);
            counters.bytes.fetch_add(md.len(), Ordering::Relaxed);
            if md.len() >= min_size.max(1) {
                let (dev, ino, nlink) = id_of(&md).unwrap_or((0, 0, 1));
                let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                emit(Found { path: e.path().to_string_lossy().into_owned(), size: md.len(), mtime, dev, ino, nlink, clone: None });
            }
        }
    }
    true
}

#[cfg(unix)]
fn id_of(md: &std::fs::Metadata) -> Option<(u64, u64, u32)> {
    use std::os::unix::fs::MetadataExt;
    Some((md.dev(), md.ino(), md.nlink() as u32))
}

#[cfg(not(unix))]
fn id_of(_: &std::fs::Metadata) -> Option<(u64, u64, u32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_with_sizes_and_times() {
        let dir = std::env::temp_dir().join(format!("sqd-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(dir.join("Tool.app/Contents")).unwrap();
        std::fs::write(dir.join("one.jpg"), vec![1u8; 4000]).unwrap();
        std::fs::write(dir.join("a/b/two.jpg"), vec![1u8; 12345]).unwrap();
        std::fs::write(dir.join("a/tiny"), b"x").unwrap();
        std::fs::write(dir.join("node_modules/x/dep.js"), vec![0u8; 9000]).unwrap();
        std::fs::write(dir.join("Tool.app/Contents/bin"), vec![0u8; 9000]).unwrap();
        let out = Mutex::new(Vec::new());
        let (no, counters) = (AtomicBool::new(false), Counters::default());
        walk(&dir, 100, 3, &no, &counters, &|f| out.lock().unwrap().push(f));
        let mut v = out.into_inner().unwrap();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(v.len(), 2, "{v:?}");
        for f in &v {
            let md = std::fs::metadata(&f.path).unwrap();
            assert_eq!(f.size, md.len());
            let secs = md.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            assert_eq!(f.mtime, secs, "{}", f.path);
            assert!(f.ino != 0 || cfg!(windows));
        }
        assert_eq!(counters.files.load(Ordering::Relaxed), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
