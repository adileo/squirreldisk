//! Parallel local filesystem scanner.
//!
//! A small pool of worker threads pulls directories from a shared BFS queue,
//! lists them with one `lstat` per entry and inserts the result into the shared
//! tree under a short write lock. BFS order makes the sunburst "grow" from the
//! center while the scan is running.

use super::{platform, Progress, ScanHandle, ScanLimits, SharedTree};
use crate::tree::{Kind, Source, Tree, F_DENIED, F_DONE, F_EXPANDABLE, NONE};
use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, RwLock};

pub const SMALL_FILES_NAME: &str = "\u{2026}";

struct Ctx {
    allowed_devs: Vec<u64>,
    skip: Vec<PathBuf>,
    hardlinks: Mutex<HashSet<(u64, u64)>>,
    limits: ScanLimits,
    progress: Arc<Progress>,
}

#[derive(Default)]
struct Listing {
    dirs: Vec<(String, u64)>,
    mounts: Vec<String>,
    files: Vec<(String, u64)>,
    small_size: u64,
    small_count: u32,
    denied: bool,
}

impl Listing {
    fn files_total(&self) -> (u64, u32) {
        let s: u64 = self.files.iter().map(|f| f.1).sum::<u64>() + self.small_size;
        (s, self.files.len() as u32 + self.small_count)
    }
}

fn list_dir(path: &Path, ctx: &Ctx, all_files: bool) -> Listing {
    let mut out = Listing::default();
    let mut files: Vec<(OsString, u64)> = Vec::new();
    let mut nfiles = 0u64;
    let mut bytes = 0u64;
    #[cfg(target_os = "macos")]
    let bulk_ok = std::env::var_os("SQD_NO_BULK").is_none() && gather_bulk(path, ctx, &mut out, &mut files, &mut nfiles, &mut bytes);
    #[cfg(not(target_os = "macos"))]
    let bulk_ok = false;
    if !bulk_ok && !gather_std(path, ctx, &mut out, &mut files, &mut nfiles, &mut bytes) {
        out.denied = true;
        ctx.progress.errors.fetch_add(1, Ordering::Relaxed);
        return out;
    }
    ctx.progress.files.fetch_add(nfiles, Ordering::Relaxed);
    ctx.progress.bytes.fetch_add(bytes, Ordering::Relaxed);

    // Keep only the biggest files as individual nodes.
    let max = if all_files { 5000 } else { ctx.limits.max_files_per_dir };
    let keep_min = if all_files { 0 } else { ctx.limits.keep_min };
    if files.len() > max {
        files.select_nth_unstable_by(max, |a, b| b.1.cmp(&a.1));
    }
    for (i, (name, size)) in files.into_iter().enumerate() {
        if i < max && size >= keep_min {
            out.files.push((name.to_string_lossy().into_owned(), size));
        } else {
            out.small_size += size;
            out.small_count += 1;
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn gather_bulk(path: &Path, ctx: &Ctx, out: &mut Listing, files: &mut Vec<(OsString, u64)>, nfiles: &mut u64, bytes: &mut u64) -> bool {
    use platform::RawKind;
    let Ok(entries) = platform::list_bulk(path) else { return false };
    for e in entries {
        match e.kind {
            RawKind::Link | RawKind::Other => {}
            RawKind::MountPoint => out.mounts.push(e.name),
            RawKind::Dir => {
                if !ctx.skip.is_empty() && ctx.skip.iter().any(|s| s == &path.join(&e.name)) {
                    continue;
                }
                if !ctx.allowed_devs.is_empty() && !ctx.allowed_devs.contains(&e.dev) {
                    out.mounts.push(e.name);
                } else {
                    out.dirs.push((e.name, 0));
                }
            }
            RawKind::File => {
                let mut alloc = e.alloc;
                if e.nlink > 1 && !ctx.hardlinks.lock().unwrap().insert((e.dev, e.ino)) {
                    alloc = 0;
                }
                *nfiles += 1;
                *bytes += alloc;
                files.push((OsString::from(e.name), alloc));
            }
        }
    }
    true
}

/// Dataless cloud folders (macOS File Provider) or offline/recall-on-access
/// folders (Windows Cloud Files) are not descended into.
#[allow(unused_variables)]
fn is_cloud_placeholder_dir(md: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        platform::is_dataless(md)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        md.file_attributes() & (0x1000 | 0x400000) != 0
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        false
    }
}

fn gather_std(path: &Path, ctx: &Ctx, out: &mut Listing, files: &mut Vec<(OsString, u64)>, nfiles: &mut u64, bytes: &mut u64) -> bool {
    let Ok(rd) = std::fs::read_dir(path) else { return false };
    for entry in rd {
        let Ok(entry) = entry else {
            ctx.progress.errors.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            // Never follow links/junctions: they would double count or loop.
            continue;
        }
        let md = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                ctx.progress.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        let epath = entry.path();
        let info = platform::info(&md, &epath);
        if ft.is_dir() {
            if is_cloud_placeholder_dir(&md) {
                continue;
            }
            if ctx.skip.iter().any(|s| s == &epath) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !ctx.allowed_devs.is_empty() && cfg!(unix) && !ctx.allowed_devs.contains(&info.dev) {
                out.mounts.push(name);
            } else {
                out.dirs.push((name, info.alloc));
                *bytes += info.alloc;
            }
        } else {
            let mut alloc = info.alloc;
            if let Some(key) = info.hardlink {
                if !ctx.hardlinks.lock().unwrap().insert(key) {
                    alloc = 0; // already counted through another link
                }
            }
            *nfiles += 1;
            *bytes += alloc;
            files.push((entry.file_name(), alloc));
        }
    }
    true
}

fn insert_listing(tree: &mut Tree, node: u32, l: &Listing) -> Vec<(u32, String)> {
    let mut new_dirs = Vec::with_capacity(l.dirs.len());
    let mut total = 0u64;
    for (name, own) in &l.dirs {
        let id = tree.add_child(node, name, Kind::Dir, *own, 0);
        total += own;
        new_dirs.push((id, name.clone()));
    }
    for name in &l.mounts {
        tree.add_child(node, name, Kind::Mount, 0, 0);
    }
    for (name, size) in &l.files {
        tree.add_child(node, name, Kind::File, *size, 1);
    }
    if l.small_count > 0 {
        let id = tree.add_child(node, SMALL_FILES_NAME, Kind::SmallFiles, l.small_size, l.small_count);
        tree.get_mut(id).flags |= F_EXPANDABLE;
    }
    let (fs, fc) = l.files_total();
    total += fs;
    tree.add_size(node, total as i64, fc as i64);
    let n = tree.get_mut(node);
    n.flags |= F_DONE;
    if l.denied {
        n.flags |= F_DENIED;
    }
    new_dirs
}

struct Queue {
    state: Mutex<(VecDeque<(u32, PathBuf)>, usize)>,
    cv: Condvar,
}

fn worker(tree: &SharedTree, q: &Queue, ctx: &Ctx) {
    loop {
        let (node, path) = {
            let mut st = q.state.lock().unwrap();
            loop {
                if ctx.progress.cancelled() {
                    q.cv.notify_all();
                    return;
                }
                if let Some(j) = st.0.pop_front() {
                    st.1 += 1;
                    break j;
                }
                if st.1 == 0 {
                    q.cv.notify_all();
                    return;
                }
                st = q.cv.wait(st).unwrap();
            }
        };
        let wid = std::thread::current().id().as_u64_compat();
        ctx.progress.inflight.lock().unwrap().push((wid, path.to_string_lossy().into_owned(), std::time::Instant::now()));
        // A bug in one directory must never hang the whole scan.
        let listing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list_dir(&path, ctx, false))).unwrap_or_else(|_| Listing { denied: true, ..Default::default() });
        ctx.progress.dirs.fetch_add(1, Ordering::Relaxed);
        ctx.progress.inflight.lock().unwrap().retain(|e| e.0 != wid);
        let new_dirs = {
            let mut t = tree.write().unwrap();
            if !t.is_alive(node) {
                Vec::new()
            } else {
                insert_listing(&mut t, node, &listing)
            }
        };
        let mut st = q.state.lock().unwrap();
        for (id, name) in new_dirs {
            st.0.push_back((id, path.join(name)));
        }
        st.1 -= 1;
        drop(st);
        q.cv.notify_all();
    }
}

trait ThreadIdExt {
    fn as_u64_compat(&self) -> usize;
}
impl ThreadIdExt for std::thread::ThreadId {
    fn as_u64_compat(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish() as usize
    }
}

pub fn thread_count() -> usize {
    if let Some(n) = std::env::var("SQD_THREADS").ok().and_then(|s| s.parse().ok()) {
        return n;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 8)
}

/// Scans `root` into `tree` (whose root node must represent `root`). Blocks.
pub fn run(tree: SharedTree, root: &Path, progress: Arc<Progress>, limits: ScanLimits, threads: usize) {
    let ctx = Ctx {
        allowed_devs: platform::allowed_devices(root),
        skip: platform::skip_paths(root),
        hardlinks: Mutex::new(HashSet::new()),
        limits,
        progress: progress.clone(),
    };
    // The root directory itself.
    let root_id = tree.read().unwrap().root;
    let q = Queue { state: Mutex::new((VecDeque::from([(root_id, root.to_path_buf())]), 0)), cv: Condvar::new() };
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| worker(&tree, &q, &ctx));
        }
    });
}

/// Starts an asynchronous scan of a local path.
pub fn start(root: PathBuf, display_name: String) -> ScanHandle {
    let progress = Arc::new(Progress::default());
    let root_str = root.to_string_lossy().into_owned();
    let tree = Arc::new(RwLock::new(Tree::new(&root_str, &display_name, Source::Local)));
    let (t, p) = (tree.clone(), progress.clone());
    std::thread::Builder::new()
        .name("scan".into())
        .spawn(move || {
            let volume_root = platform::is_volume_root(&root);
            let space = platform::volume_space(&root);
            let mut limits = ScanLimits::default();
            if let (true, Some((total, avail))) = (volume_root, space) {
                let used = total.saturating_sub(avail);
                p.expected.store(used, Ordering::Relaxed);
                limits = ScanLimits::for_expected(used);
            }
            p.set_status("Scanning");
            let started = std::time::Instant::now();
            run(t.clone(), &root, p.clone(), limits, thread_count());
            if !p.cancelled() && volume_root {
                if let Some((total, avail)) = space {
                    // Whatever the volume reports as used but we could not see.
                    let used = total.saturating_sub(avail);
                    let mut tr = t.write().unwrap();
                    let seen = tr.get(tr.root).size;
                    if used > seen {
                        let r = tr.root;
                        tr.add_child_sized(r, "Hidden space", Kind::Hidden, used - seen, 0);
                    }
                }
            }
            p.set_status(format!("Scanned in {:.1}s", started.elapsed().as_secs_f32()));
            p.done.store(true, Ordering::Relaxed);
        })
        .expect("spawn scan thread");
    ScanHandle { tree, progress }
}

/// Synchronous scan into a fresh tree (used for incremental refreshes and the agent).
pub fn scan_blocking(root: &Path, limits: ScanLimits, progress: Arc<Progress>) -> Tree {
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.to_string_lossy().into_owned());
    let tree = Arc::new(RwLock::new(Tree::new(&root.to_string_lossy(), &name, Source::Local)));
    run(tree.clone(), root, progress, limits, thread_count().min(6));
    Arc::try_unwrap(tree).ok().expect("tree still shared").into_inner().unwrap()
}

fn quiet_ctx(root: &Path) -> Ctx {
    Ctx {
        allowed_devs: platform::allowed_devices(root),
        skip: Vec::new(),
        hardlinks: Mutex::new(HashSet::new()),
        limits: ScanLimits::default(),
        progress: Arc::new(Progress::default()),
    }
}

/// Replaces a "small files" aggregate with the actual files of its directory.
pub fn expand_small(tree: &SharedTree, small: u32) -> bool {
    let (dir, path) = {
        let t = tree.read().unwrap();
        if !t.is_alive(small) || t.get(small).kind != Kind::SmallFiles || !t.source.is_local() {
            return false;
        }
        let dir = t.get(small).parent;
        (dir, t.path_buf(dir))
    };
    let ctx = quiet_ctx(&path);
    let listing = list_dir(&path, &ctx, true);
    let mut t = tree.write().unwrap();
    if !t.is_alive(dir) {
        return false;
    }
    let existing: HashSet<String> =
        t.children(dir).filter(|c| t.get(*c).kind == Kind::File).map(|c| t.get(c).name.to_string()).collect();
    t.remove(small);
    let mut rest_size = listing.small_size;
    let mut rest_count = listing.small_count;
    for (name, size) in &listing.files {
        if existing.contains(name) {
            continue;
        }
        if *size == 0 {
            rest_count += 1;
            continue;
        }
        t.add_child_sized(dir, name, Kind::File, *size, 1);
    }
    if rest_count > 0 {
        let id = t.add_child_sized(dir, SMALL_FILES_NAME, Kind::SmallFiles, rest_size, rest_count);
        t.get_mut(id).flags &= !F_EXPANDABLE;
        rest_size = 0;
    }
    let _ = rest_size;
    true
}

/// Re-lists a directory after a filesystem change and patches the tree.
/// Sub-directories that still exist are kept as they are; new ones are scanned.
pub fn refresh_dir(tree: &SharedTree, dir: u32) {
    let path = {
        let t = tree.read().unwrap();
        if !t.is_alive(dir) || t.get(dir).kind != Kind::Dir {
            return;
        }
        t.path_buf(dir)
    };
    if !path.exists() {
        tree.write().unwrap().remove(dir);
        return;
    }
    let ctx = quiet_ctx(&path);
    let listing = list_dir(&path, &ctx, false);
    // Scan newly created sub-directories outside the lock.
    let known: HashSet<String> = {
        let t = tree.read().unwrap();
        t.children(dir).filter(|c| t.get(*c).kind == Kind::Dir).map(|c| t.get(c).name.to_string()).collect()
    };
    let mut new_trees = Vec::new();
    for (name, _) in &listing.dirs {
        if !known.contains(name) {
            let sub = scan_blocking(&path.join(name), ScanLimits::default(), Arc::new(Progress::default()));
            new_trees.push((name.clone(), sub));
        }
    }
    let mut t = tree.write().unwrap();
    if !t.is_alive(dir) {
        return;
    }
    let listed: HashSet<&str> = listing.dirs.iter().map(|d| d.0.as_str()).collect();
    let children: Vec<u32> = t.children(dir).collect();
    for c in children {
        let n = t.get(c);
        let drop = match n.kind {
            Kind::Dir => !listed.contains(&*n.name),
            Kind::File | Kind::SmallFiles | Kind::Mount | Kind::Link => true,
            Kind::Hidden => false,
        };
        if drop {
            t.remove(c);
        }
    }
    for name in &listing.mounts {
        t.add_child(dir, name, Kind::Mount, 0, 0);
    }
    for (name, size) in &listing.files {
        t.add_child_sized(dir, name, Kind::File, *size, 1);
    }
    if listing.small_count > 0 {
        let id = t.add_child_sized(dir, SMALL_FILES_NAME, Kind::SmallFiles, listing.small_size, listing.small_count);
        t.get_mut(id).flags |= F_EXPANDABLE;
    }
    for (name, sub) in new_trees {
        t.graft(dir, &sub, &name);
    }
    let _ = NONE;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scans_fake_tree() {
        let base = std::env::temp_dir().join(format!("sqd-test-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("a/b")).unwrap();
        fs::write(base.join("a/big.bin"), vec![1u8; 200_000]).unwrap();
        fs::write(base.join("a/b/small.txt"), b"hello").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&base, base.join("a/loop")).unwrap();
        let t = scan_blocking(&base, ScanLimits { keep_min: 100_000, max_files_per_dir: 10 }, Arc::new(Progress::default()));
        assert_eq!(t.get(t.root).files, 2);
        assert!(t.get(t.root).size >= 200_000);
        let (a, ok) = t.find_path(&base.join("a").to_string_lossy());
        assert!(ok);
        assert!(t.find_child(a, "big.bin").is_some());
        assert!(t.find_child(a, "loop").is_none());
        fs::remove_dir_all(&base).unwrap();
    }
}
