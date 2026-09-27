//! Filesystem change notifications (FSEvents / ReadDirectoryChangesW / inotify).

use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

pub struct FsWatch {
    watcher: notify::RecommendedWatcher,
    pub rx: Receiver<PathBuf>,
    watched: Vec<PathBuf>,
    recursive_root: bool,
}

impl FsWatch {
    pub fn new(root: &Path) -> Option<FsWatch> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if matches!(ev.kind, EventKind::Access(_)) {
                    return;
                }
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        })
        .ok()?;
        // FSEvents and ReadDirectoryChangesW are cheap even for a whole disk;
        // inotify needs one watch per directory, so on Linux we only watch what
        // is on screen (see `focus`).
        let recursive_root = !cfg!(target_os = "linux");
        if recursive_root {
            watcher.watch(root, RecursiveMode::Recursive).ok()?;
        }
        Some(FsWatch { watcher, rx, watched: Vec::new(), recursive_root })
    }

    /// On Linux, watch the directories currently displayed.
    pub fn focus(&mut self, dirs: Vec<PathBuf>) {
        if self.recursive_root {
            return;
        }
        for d in self.watched.drain(..) {
            let _ = self.watcher.unwatch(&d);
        }
        for d in dirs.into_iter().take(256) {
            if self.watcher.watch(&d, RecursiveMode::NonRecursive).is_ok() {
                self.watched.push(d);
            }
        }
    }
}

/// Translates event paths into the namespace of the scanned tree
/// (macOS reports Data-volume paths through /System/Volumes/Data).
pub fn normalize_event_path(p: PathBuf, root: &str) -> PathBuf {
    if cfg!(target_os = "macos") && root == "/" {
        if let Ok(rest) = p.strip_prefix("/System/Volumes/Data") {
            return Path::new("/").join(rest);
        }
    }
    p
}
