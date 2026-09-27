pub mod demo;
pub mod local;
pub mod pathtree;
pub mod platform;
pub mod remote;

use crate::tree::Tree;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub type SharedTree = Arc<RwLock<Tree>>;

/// Live progress of a scan, shared between scanner threads and the UI.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    /// Expected total bytes (volume used space), 0 if unknown.
    pub expected: AtomicU64,
    pub done: AtomicBool,
    pub cancel: AtomicBool,
    pub error: Mutex<Option<String>>,
    pub status: Mutex<String>,
    /// Directories currently being listed, with the time listing started.
    pub inflight: Mutex<Vec<(usize, String, std::time::Instant)>>,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        let exp = self.expected.load(Ordering::Relaxed);
        if exp == 0 {
            return None;
        }
        Some((self.bytes.load(Ordering::Relaxed) as f64 / exp as f64).clamp(0.0, 0.995) as f32)
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn set_status(&self, s: impl Into<String>) {
        *self.status.lock().unwrap() = s.into();
    }
    /// A directory that has been blocking a worker for longer than `secs`.
    pub fn stuck(&self, secs: f32) -> Option<(String, f32)> {
        let v = self.inflight.lock().unwrap();
        v.iter()
            .map(|(_, p, t)| (p.clone(), t.elapsed().as_secs_f32()))
            .filter(|(_, e)| *e > secs)
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }
    pub fn fail(&self, s: impl Into<String>) {
        *self.error.lock().unwrap() = Some(s.into());
        self.done.store(true, Ordering::Relaxed);
    }
}

/// Tunables that keep memory bounded.
#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    /// Files smaller than this are folded into a "small files" node.
    pub keep_min: u64,
    /// Max individual file nodes per directory.
    pub max_files_per_dir: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        ScanLimits { keep_min: 32 * 1024, max_files_per_dir: 48 }
    }
}

impl ScanLimits {
    /// Pick sensible limits from the amount of data we expect to scan.
    pub fn for_expected(bytes: u64) -> Self {
        let keep_min = (bytes / 4_000_000).clamp(16 * 1024, 1024 * 1024);
        ScanLimits { keep_min, max_files_per_dir: 48 }
    }
}

pub struct ScanHandle {
    pub tree: SharedTree,
    pub progress: Arc<Progress>,
}
