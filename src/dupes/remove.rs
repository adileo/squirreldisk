//! Removes the copies chosen for removal, one group at a time, after
//! checking that a copy of each file stays behind.

use super::engine::Spec;
use crate::safety::{Os, Rules};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Where a copy lives.
#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    Local,
    Ssh(String),
    Rclone,
}

impl Location {
    pub fn of(spec: &Spec) -> Location {
        match spec {
            Spec::Local(_) => Location::Local,
            Spec::Ssh { host, .. } => Location::Ssh(host.clone()),
            Spec::Rclone(_) => Location::Rclone,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    pub at: Location,
    pub path: String,
    /// Removing it frees space (not a clone).
    pub frees: bool,
    /// Modification time seen when comparing: a file changed since is left alone.
    pub mtime: Option<i64>,
}

/// One group: what goes, and what must still be there afterwards.
#[derive(Clone, Debug)]
pub struct Batch {
    pub size: u64,
    pub remove: Vec<Target>,
    pub keep: Vec<Target>,
}

#[derive(Default)]
pub struct Progress {
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub freed: AtomicU64,
    pub errors: Mutex<Vec<String>>,
    /// Paths removed, for the screen to drop them.
    pub removed: Mutex<Vec<String>>,
    pub finished: AtomicBool,
    pub cancel: AtomicBool,
}

/// Removes the batches' copies: to the Trash, or for good with `permanent`
/// (servers and cloud storage have no Trash: always for good).
pub fn start(batches: Vec<Batch>, permanent: bool) -> Arc<Progress> {
    let p = Arc::new(Progress::default());
    p.total.store(batches.iter().map(|b| b.remove.len() as u64).sum(), Ordering::Relaxed);
    let pc = p.clone();
    std::thread::Builder::new()
        .name("dupes-remove".into())
        .spawn(move || {
            run(&batches, permanent, &pc);
            pc.finished.store(true, Ordering::Relaxed);
        })
        .expect("spawn remove thread");
    p
}

fn run(batches: &[Batch], permanent: bool, p: &Progress) {
    let local_rules = Rules::current();
    let remote_rules = Rules::for_os(Os::Linux, None);
    for b in batches {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        // never leave a file without a copy: a kept copy on this machine
        // must still be there, unchanged in size
        let local_keeps: Vec<&Target> = b.keep.iter().filter(|k| k.at == Location::Local).collect();
        let kept_ok = b.keep.iter().any(|k| k.at != Location::Local) || local_keeps.iter().any(|k| std::fs::metadata(&k.path).is_ok_and(|m| m.len() == b.size));
        if b.keep.is_empty() || !kept_ok {
            let name = b.remove.first().map(|t| t.path.rsplit('/').next().unwrap_or(&t.path).to_string()).unwrap_or_default();
            p.errors.lock().unwrap().push(format!("{name}: the copy to keep is missing, nothing removed"));
            p.done.fetch_add(b.remove.len() as u64, Ordering::Relaxed);
            continue;
        }
        for t in &b.remove {
            let rules = if t.at == Location::Local { &local_rules } else { &remote_rules };
            let res = if rules.check(&t.path).is_forbidden() {
                Err("protected system location".to_string())
            } else if t.at == Location::Local && !unchanged(&t.path, b.size, t.mtime) {
                Err("changed since the comparison, left alone".to_string())
            } else {
                remove(t, permanent)
            };
            match res {
                Ok(()) => {
                    if t.frees {
                        p.freed.fetch_add(b.size, Ordering::Relaxed);
                    }
                    p.removed.lock().unwrap().push(t.path.clone());
                }
                Err(e) => p.errors.lock().unwrap().push(format!("{}: {e}", t.path)),
            }
            p.done.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Still the file that was compared: same size, same modification time.
fn unchanged(path: &str, size: u64, mtime: Option<i64>) -> bool {
    let Ok(md) = std::fs::symlink_metadata(path) else { return false };
    let secs = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    md.is_file() && md.len() == size && mtime.is_none_or(|m| secs == Some(m))
}

fn remove(t: &Target, permanent: bool) -> Result<(), String> {
    match &t.at {
        Location::Local if permanent => std::fs::remove_file(&t.path).map_err(|e| e.to_string()),
        // to the Trash: a mistake can be undone
        Location::Local => crate::delete::to_trash(&t.path),
        Location::Ssh(host) => {
            crate::scan::remote::run_ssh(host, &format!("rm -f -- {}", crate::scan::remote::sh_quote(&t.path)))?;
            Ok(())
        }
        Location::Rclone => {
            let st = crate::rclone::command()
                .args(["deletefile", &t.path])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| format!("cannot run rclone: {e}"))?;
            if st.success() { Ok(()) } else { Err("rclone could not delete it".into()) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(p: &Progress) {
        let t = Instant::now();
        while !p.finished.load(Ordering::Relaxed) {
            assert!(t.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn keeps_a_copy() {
        let dir = std::env::temp_dir().join(format!("sqd-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.jpg"), dir.join("b.jpg"));
        std::fs::write(&a, b"same").unwrap();
        std::fs::write(&b, b"same").unwrap();
        let t = |p: &std::path::Path| Target { at: Location::Local, path: p.to_string_lossy().into_owned(), frees: true, mtime: None };
        // the kept copy is gone: nothing is removed
        let gone = dir.join("gone.jpg");
        let p = start(vec![Batch { size: 4, remove: vec![t(&a)], keep: vec![t(&gone)] }], false);
        wait(&p);
        assert!(a.exists());
        assert_eq!(p.errors.lock().unwrap().len(), 1);
        // changed since the comparison: left alone
        let mut stale = t(&a);
        stale.mtime = Some(1);
        let p = start(vec![Batch { size: 4, remove: vec![stale], keep: vec![t(&b)] }], false);
        wait(&p);
        assert!(a.exists());
        assert!(p.errors.lock().unwrap()[0].contains("changed"));
        assert!(unchanged(&a.to_string_lossy(), 4, None) && !unchanged(&a.to_string_lossy(), 5, None));
        // for good: gone, and the kept copy stays
        let p = start(vec![Batch { size: 4, remove: vec![t(&a)], keep: vec![t(&b)] }], true);
        wait(&p);
        assert!(!a.exists() && b.exists());
        assert_eq!(p.freed.load(Ordering::Relaxed), 4);
        // the kept copy is there: the other goes (to the Trash)
        if std::env::var_os("SQD_TEST_TRASH").is_some() {
            let p = start(vec![Batch { size: 4, remove: vec![t(&a)], keep: vec![t(&b)] }], false);
            wait(&p);
            assert!(!a.exists() && b.exists());
            assert_eq!(p.freed.load(Ordering::Relaxed), 4);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
