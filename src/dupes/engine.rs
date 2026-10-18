//! The duplicate finder.
//!
//! Every place is listed at the same time (folders and disks here, servers
//! through the agent, cloud storage through rclone) and files stream into
//! buckets by size: only files sharing a size can be identical, so most are
//! never read. As soon as a bucket holds two files they get a quick
//! fingerprint (first and last 64 KiB), and files whose quick fingerprints
//! match get a sampled one (spots spread over the whole file, at most about
//! 2 MiB read; small files are read whole), or, in exact mode, a full one
//! over every byte. Files with the same fingerprint are duplicates,
//! published while the rest is still being compared. Names play no part:
//! copies are often renamed, and different photos often share a name.
//!
//! Cloud storage reports MD5 or SHA-1 for free: when a bucket holds cloud
//! files, the other files in it get that fingerprint instead, so a photo on
//! this Mac can be matched with its copy on Google Drive without
//! downloading it.
//!
//! Reads are scheduled per disk (and per server or cloud), biggest possible
//! savings first, so the large videos show up early. Fingerprints of local
//! files are remembered between runs, keyed by inode, size and time, which
//! makes comparing again nearly instant.

use super::hash::{self, Kind, PART};
use super::walk::{self, Counters, Found};
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A place to compare.
#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    Local(PathBuf),
    Ssh { host: String, path: String },
    Rclone(String),
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Smaller files are ignored.
    pub min_size: u64,
    /// Read every byte of big files instead of samples.
    pub exact: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { min_size: 64 * 1024, exact: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FoundFile {
    pub place: usize,
    pub path: String,
    pub mtime: i64,
    /// An APFS clone of another copy: removing it frees no space.
    pub shares_data: bool,
}

/// Files with identical content.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundGroup {
    /// Stable while the group exists.
    pub key: u64,
    pub size: u64,
    /// Full fingerprint, in hex.
    pub digest: String,
    pub files: Vec<FoundFile>,
}

#[derive(Clone, Debug, Default)]
pub struct PlaceStatus {
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub walking: bool,
    pub done: bool,
    pub files: u64,
    pub bytes: u64,
    /// Files that share their size with another one.
    pub candidates: u64,
    /// Bytes to read to compare them, and read so far.
    pub to_read: u64,
    pub read: u64,
    pub errors: u64,
    pub elapsed: Duration,
    pub places: Vec<PlaceStatus>,
}

impl Status {
    /// Progress of the comparison once listing is over.
    pub fn fraction(&self) -> Option<f32> {
        if self.done {
            Some(1.0)
        } else if self.walking || self.to_read == 0 {
            None
        } else {
            Some((self.read as f64 / self.to_read as f64).clamp(0.0, 0.99) as f32)
        }
    }
}

struct Inner {
    cancel: AtomicBool,
    walking: AtomicBool,
    done: AtomicBool,
    candidates: AtomicU64,
    to_read: AtomicU64,
    read: AtomicU64,
    hash_errors: AtomicU64,
    counters: Vec<Counters>,
    places: Mutex<Vec<PlaceStatus>>,
    results: Mutex<Results>,
    started: Instant,
    finished: Mutex<Option<Duration>>,
}

/// A running (or finished) comparison. Dropping it stops it.
pub struct Finder {
    inner: Arc<Inner>,
}

impl Finder {
    pub fn start(specs: Vec<Spec>, opts: Options) -> Finder {
        let inner = Arc::new(Inner {
            cancel: AtomicBool::new(false),
            walking: AtomicBool::new(true),
            done: AtomicBool::new(false),
            candidates: AtomicU64::new(0),
            to_read: AtomicU64::new(0),
            read: AtomicU64::new(0),
            hash_errors: AtomicU64::new(0),
            counters: specs.iter().map(|_| Counters::default()).collect(),
            places: Mutex::new(vec![PlaceStatus::default(); specs.len()]),
            results: Mutex::new(Results::default()),
            started: Instant::now(),
            finished: Mutex::new(None),
        });
        let i = inner.clone();
        std::thread::Builder::new().name("dupes".into()).spawn(move || coordinate(i, specs, opts)).expect("spawn dupes thread");
        Finder { inner }
    }

    pub fn cancel(&self) {
        self.inner.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_done(&self) -> bool {
        self.inner.done.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> Status {
        let i = &self.inner;
        Status {
            walking: i.walking.load(Ordering::Relaxed),
            done: i.done.load(Ordering::Relaxed),
            files: i.counters.iter().map(|c| c.files.load(Ordering::Relaxed)).sum(),
            bytes: i.counters.iter().map(|c| c.bytes.load(Ordering::Relaxed)).sum(),
            candidates: i.candidates.load(Ordering::Relaxed),
            to_read: i.to_read.load(Ordering::Relaxed),
            read: i.read.load(Ordering::Relaxed),
            errors: i.hash_errors.load(Ordering::Relaxed) + i.counters.iter().map(|c| c.errors.load(Ordering::Relaxed)).sum::<u64>(),
            elapsed: i.finished.lock().unwrap().unwrap_or_else(|| i.started.elapsed()),
            places: i.places.lock().unwrap().clone(),
        }
    }

    /// The size buckets whose groups changed since `seen`, with their
    /// groups now (none: the bucket has no duplicates any more). Only what
    /// changed is copied: with tens of thousands of groups, copying them
    /// all a few times a second is what made the screen lag.
    pub fn changes_since(&self, seen: &mut u64) -> Option<Vec<(u64, Vec<FoundGroup>)>> {
        let r = self.inner.results.lock().unwrap();
        if r.version == *seen {
            return None;
        }
        let since = *seen;
        *seen = r.version;
        Some(r.buckets.iter().filter(|(_, (v, _))| *v > since).map(|(size, (_, g))| (*size, g.clone())).collect())
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Groups by size bucket, each with the version that last changed it.
#[derive(Default)]
struct Results {
    version: u64,
    buckets: HashMap<u64, (u64, Vec<FoundGroup>)>,
}

// ---------------------------------------------------------------------------
// work queues: one per disk, server or cloud account

struct Job {
    prio: u64,
    seq: u64,
    idx: usize,
    kind: Kind,
    path: String,
    size: u64,
}

impl PartialEq for Job {
    fn eq(&self, o: &Self) -> bool {
        self.seq == o.seq
    }
}
impl Eq for Job {}
impl PartialOrd for Job {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Job {
    /// Biggest savings first; quick fingerprints before full ones; then in order.
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.prio.cmp(&o.prio).then(o.kind.cmp(&self.kind)).then(o.seq.cmp(&self.seq))
    }
}

#[derive(Default)]
struct Queue {
    st: Mutex<(BinaryHeap<Job>, bool)>,
    cv: Condvar,
}

impl Queue {
    fn push(&self, j: Job) {
        self.st.lock().unwrap().0.push(j);
        self.cv.notify_one();
    }
    fn close(&self) {
        let mut st = self.st.lock().unwrap();
        st.1 = true;
        st.0.clear();
        self.cv.notify_all();
    }
    fn pop(&self) -> Option<Job> {
        let mut st = self.st.lock().unwrap();
        loop {
            if let Some(j) = st.0.pop() {
                return Some(j);
            }
            if st.1 {
                return None;
            }
            st = self.cv.wait(st).unwrap();
        }
    }
}

#[derive(Clone, PartialEq)]
enum ExecKind {
    Local(u64),
    Ssh(String),
    Rclone(String),
}

struct Exec {
    kind: ExecKind,
    queue: Arc<Queue>,
}

/// MD5 and SHA-1 that cloud storage reported for a file.
type CloudHashes = (Option<Vec<u8>>, Option<Vec<u8>>);

enum Msg {
    Found { place: usize, f: Found, cloud: Option<CloudHashes> },
    WalkDone { place: usize, error: Option<String> },
    Hashed { idx: usize, kind: Kind, result: Result<Vec<u8>, String> },
}

// ---------------------------------------------------------------------------
// coordinator

struct Entry {
    place: usize,
    exec: usize,
    path: String,
    size: u64,
    mtime: i64,
    dev: u64,
    ino: u64,
    clone: Option<u64>,
    cloud: bool,
    hashes: [Option<Vec<u8>>; 5],
    pending: u8,
    failed: u8,
}

fn slot(k: Kind) -> usize {
    match k {
        Kind::Partial => 0,
        Kind::Full => 1,
        Kind::Md5 => 2,
        Kind::Sha1 => 3,
        Kind::Sample => 4,
    }
}

fn bit(k: Kind) -> u8 {
    1 << slot(k)
}

impl Entry {
    fn has(&self, k: Kind) -> bool {
        self.hashes[slot(k)].is_some()
    }
    fn hash(&self, k: Kind) -> Option<&Vec<u8>> {
        self.hashes[slot(k)].as_ref()
    }
    fn local(&self) -> bool {
        !self.cloud
    }
}

struct State {
    inner: Arc<Inner>,
    execs: Vec<Exec>,
    entries: Vec<Entry>,
    buckets: HashMap<u64, Vec<usize>>,
    /// (machine, device, inode) of the files already listed.
    seen: HashSet<(usize, u64, u64)>,
    seen_paths: HashSet<(usize, String)>,
    outstanding: usize,
    seq: u64,
    cache: Cache,
    /// Fingerprint that confirms big files: sampled, or full in exact mode.
    confirm: Kind,
}

fn coordinate(inner: Arc<Inner>, specs: Vec<Spec>, opts: Options) {
    let (tx, rx) = channel::<Msg>();
    // executors, shared by places on the same disk / host / account
    let mut execs: Vec<Exec> = Vec::new();
    let mut place_exec = Vec::new();
    for spec in &specs {
        let kind = match spec {
            Spec::Local(p) => ExecKind::Local(crate::scan::platform::device_of(p).unwrap_or(0)),
            Spec::Ssh { host, .. } => ExecKind::Ssh(host.clone()),
            Spec::Rclone(r) => ExecKind::Rclone(r.split(':').next().unwrap_or(r).to_string()),
        };
        let e = match execs.iter().position(|e| e.kind == kind) {
            Some(e) => e,
            None => {
                execs.push(Exec { kind, queue: Arc::new(Queue::default()) });
                execs.len() - 1
            }
        };
        place_exec.push(e);
    }
    let boot = crate::scan::platform::device_of(Path::new("/"));
    for e in &execs {
        let threads = match &e.kind {
            // the startup SSD reads fast in parallel; external disks, often
            // spinning or on USB, are kinder to fewer readers
            ExecKind::Local(d) if Some(*d) == boot => 4,
            ExecKind::Local(_) => 2,
            ExecKind::Ssh(_) => 1,
            ExecKind::Rclone(_) => 3,
        };
        for _ in 0..threads {
            let (q, tx, inner, kind) = (e.queue.clone(), tx.clone(), inner.clone(), e.kind.clone());
            std::thread::spawn(move || hash_worker(q, tx, inner, kind));
        }
    }
    for (k, spec) in specs.iter().enumerate() {
        let (tx, inner, spec) = (tx.clone(), inner.clone(), spec.clone());
        std::thread::spawn(move || {
            let error = list_place(k, &spec, opts.min_size, &inner, &tx).err();
            let _ = tx.send(Msg::WalkDone { place: k, error });
        });
    }
    drop(tx);

    let mut st = State {
        inner: inner.clone(),
        execs,
        entries: Vec::new(),
        buckets: HashMap::new(),
        seen: HashSet::new(),
        seen_paths: HashSet::new(),
        outstanding: 0,
        seq: 0,
        cache: Cache::load(),
        confirm: if opts.exact { Kind::Full } else { Kind::Sample },
    };
    let mut walkers = specs.len();
    let mut dirty: HashSet<u64> = HashSet::new();
    let mut last_publish = Instant::now();
    loop {
        if inner.cancel.load(Ordering::Relaxed) {
            break;
        }
        let first = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // handle everything waiting, then update the touched buckets once
        for m in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok()).take(20_000)) {
            match m {
                Msg::Found { place, f, cloud } => {
                    if let Some(size) = st.add(place, place_exec[place], f, cloud) {
                        dirty.insert(size);
                    }
                }
                Msg::WalkDone { place, error } => {
                    walkers -= 1;
                    let mut ps = inner.places.lock().unwrap();
                    ps[place].done = true;
                    ps[place].error = error;
                }
                Msg::Hashed { idx, kind, result } => {
                    st.outstanding -= 1;
                    let e = &mut st.entries[idx];
                    e.pending &= !bit(kind);
                    match result {
                        Ok(h) => {
                            if e.local() && e.ino != 0 {
                                st.cache.put(e.dev, e.ino, e.size, e.mtime, kind, &h);
                            }
                            e.hashes[slot(kind)] = Some(h);
                        }
                        Err(_) => {
                            e.failed |= bit(kind);
                            inner.hash_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    dirty.insert(e.size);
                }
            }
        }
        inner.walking.store(walkers > 0, Ordering::Relaxed);
        // regrouping is cheap per bucket, but not every few microseconds
        if !dirty.is_empty() && (last_publish.elapsed() > Duration::from_millis(150) || walkers == 0) {
            let sizes: Vec<u64> = dirty.drain().collect();
            let mut results = Vec::new();
            for size in sizes {
                st.schedule(size);
                results.push((size, st.regroup(size)));
            }
            let mut r = inner.results.lock().unwrap();
            let next = r.version + 1;
            let mut changed = false;
            for (size, groups) in results {
                // most fingerprints change nothing a screen would show
                let same = match r.buckets.get(&size) {
                    Some((_, old)) => *old == groups,
                    None => groups.is_empty(),
                };
                if !same {
                    r.buckets.insert(size, (next, groups));
                    changed = true;
                }
            }
            if changed {
                r.version = next;
            }
            last_publish = Instant::now();
        }
        if walkers == 0 && st.outstanding == 0 && dirty.is_empty() {
            break;
        }
    }
    for e in &st.execs {
        e.queue.close();
    }
    if !inner.cancel.load(Ordering::Relaxed) {
        st.cache.save();
    }
    inner.walking.store(false, Ordering::Relaxed);
    *inner.finished.lock().unwrap() = Some(inner.started.elapsed());
    inner.done.store(true, Ordering::Relaxed);
}

impl State {
    /// Takes in a listed file; returns its bucket when it is new.
    fn add(&mut self, place: usize, exec: usize, f: Found, cloud: Option<CloudHashes>) -> Option<u64> {
        // the same file reached twice (overlapping places, hard links)
        let machine = match self.execs[exec].kind {
            ExecKind::Local(_) => usize::MAX,
            _ => exec,
        };
        if f.ino != 0 {
            if !self.seen.insert((machine, f.dev, f.ino)) {
                return None;
            }
        } else if !self.seen_paths.insert((machine, path_key(&f.path, cfg!(windows) && machine == usize::MAX))) {
            return None;
        }
        let is_cloud = cloud.is_some();
        let (md5, sha1) = cloud.unwrap_or((None, None));
        let idx = self.entries.len();
        self.entries.push(Entry {
            place,
            exec,
            path: f.path,
            size: f.size,
            mtime: f.mtime,
            dev: f.dev,
            ino: f.ino,
            clone: f.clone,
            cloud: is_cloud,
            hashes: [None, None, md5, sha1, None],
            pending: 0,
            failed: 0,
        });
        let b = self.buckets.entry(f.size).or_default();
        b.push(idx);
        let n = b.len() as u64;
        self.inner.candidates.fetch_add(if n == 2 { 2 } else if n > 2 { 1 } else { 0 }, Ordering::Relaxed);
        (n >= 2).then_some(f.size)
    }

    /// Asks for the fingerprints that can tell this bucket's files apart.
    fn schedule(&mut self, size: u64) {
        let members = self.buckets.get(&size).cloned().unwrap_or_default();
        if members.len() < 2 {
            return;
        }
        let prio = size.saturating_mul(members.len() as u64 - 1);
        let clouds: Vec<usize> = members.iter().copied().filter(|i| self.entries[*i].cloud).collect();
        if !clouds.is_empty() {
            // compare in the fingerprint the cloud gives
            let md5 = clouds.iter().filter(|i| self.entries[**i].has(Kind::Md5)).count();
            let sha1 = clouds.iter().filter(|i| self.entries[**i].has(Kind::Sha1)).count();
            let kind = if md5 >= sha1 { Kind::Md5 } else { Kind::Sha1 };
            for &m in &members {
                self.need(m, kind, prio);
            }
            return;
        }
        // small files are read whole at once
        if size <= 2 * PART {
            for &m in &members {
                self.need(m, Kind::Full, prio);
            }
            return;
        }
        for &m in &members {
            self.need(m, Kind::Partial, prio);
        }
        let mut by_partial: HashMap<&Vec<u8>, Vec<usize>> = HashMap::new();
        for &m in &members {
            if let Some(h) = self.entries[m].hash(Kind::Partial) {
                by_partial.entry(h).or_default().push(m);
            }
        }
        let alike: Vec<usize> = by_partial.into_values().filter(|v| v.len() > 1).flatten().collect();
        for m in alike {
            self.need(m, self.confirm, prio);
        }
    }

    fn need(&mut self, idx: usize, kind: Kind, prio: u64) {
        let e = &mut self.entries[idx];
        if e.has(kind) || e.pending & bit(kind) != 0 || e.failed & bit(kind) != 0 {
            return;
        }
        if e.local() && e.ino != 0 {
            if let Some(h) = self.cache.get(e.dev, e.ino, e.size, e.mtime, kind) {
                // read in `schedule` before it compares quick fingerprints
                e.hashes[slot(kind)] = Some(h);
                return;
            }
        }
        if e.cloud && matches!(kind, Kind::Partial | Kind::Sample | Kind::Full) {
            return;
        }
        e.pending |= bit(kind);
        self.inner.to_read.fetch_add(hash::cost(kind, e.size), Ordering::Relaxed);
        self.seq += 1;
        self.outstanding += 1;
        self.execs[e.exec].queue.push(Job { prio, seq: self.seq, idx, kind, path: e.path.clone(), size: e.size });
    }

    /// Groups of identical files in a bucket: files linked by any equal full fingerprint.
    fn regroup(&self, size: u64) -> Vec<FoundGroup> {
        let members = self.buckets.get(&size).cloned().unwrap_or_default();
        if members.len() < 2 {
            return Vec::new();
        }
        let mut parent: Vec<usize> = (0..members.len()).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while p[r] != r {
                r = p[r];
            }
            let mut x = x;
            while p[x] != r {
                let next = p[x];
                p[x] = r;
                x = next;
            }
            r
        }
        for kind in [Kind::Full, Kind::Sample, Kind::Md5, Kind::Sha1] {
            let mut first: HashMap<&Vec<u8>, usize> = HashMap::new();
            for (k, &m) in members.iter().enumerate() {
                if let Some(h) = self.entries[m].hash(kind) {
                    match first.get(h) {
                        Some(&j) => {
                            let (a, b) = (find(&mut parent, j), find(&mut parent, k));
                            parent[a] = b;
                        }
                        None => {
                            first.insert(h, k);
                        }
                    }
                }
            }
        }
        let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (k, &m) in members.iter().enumerate() {
            let r = find(&mut parent, k);
            comps.entry(r).or_default().push(m);
        }
        let mut out = Vec::new();
        for (_, mut v) in comps {
            if v.len() < 2 {
                continue;
            }
            v.sort_by(|a, b| self.entries[*a].path.cmp(&self.entries[*b].path));
            let min = *v.iter().min().unwrap();
            let digest = v
                .iter()
                .find_map(|m| self.entries[*m].hash(Kind::Full))
                .or_else(|| v.iter().find_map(|m| self.entries[*m].hash(Kind::Sample)))
                .or_else(|| v.iter().find_map(|m| self.entries[*m].hash(Kind::Md5)))
                .or_else(|| v.iter().find_map(|m| self.entries[*m].hash(Kind::Sha1)))
                .map(|h| hash::to_hex(h))
                .unwrap_or_default();
            // APFS clones of each other share their data
            let mut clones: HashMap<(u64, u64), usize> = HashMap::new();
            for m in &v {
                let e = &self.entries[*m];
                if let Some(c) = e.clone {
                    *clones.entry((e.dev, c)).or_default() += 1;
                }
            }
            let files = v
                .iter()
                .map(|m| {
                    let e = &self.entries[*m];
                    let shares = e.clone.is_some_and(|c| clones.get(&(e.dev, c)).copied().unwrap_or(0) > 1);
                    FoundFile { place: e.place, path: e.path.clone(), mtime: e.mtime, shares_data: shares }
                })
                .collect();
            out.push(FoundGroup { key: (size.rotate_left(17)) ^ (min as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15), size, digest, files });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// listing places

fn list_place(place: usize, spec: &Spec, min_size: u64, inner: &Arc<Inner>, tx: &Sender<Msg>) -> Result<(), String> {
    let counters = &inner.counters[place];
    match spec {
        Spec::Local(root) => {
            // one spelling per folder (Windows also accepts `/`), so the same
            // file reached through two places is recognised as one
            let root = &PathBuf::from(if cfg!(windows) { root.to_string_lossy().replace('/', "\\") } else { root.to_string_lossy().into_owned() });
            if !root.is_dir() {
                return Err(format!("{} is not available", root.display()));
            }
            let tx = Mutex::new(tx.clone());
            let emit = |f: Found| {
                let _ = tx.lock().unwrap().send(Msg::Found { place, f, cloud: None });
            };
            walk::walk(root, min_size, crate::scan::local::thread_count(), &inner.cancel, counters, &emit);
            Ok(())
        }
        Spec::Ssh { host, path } => list_ssh(place, host, path, min_size, inner, tx),
        Spec::Rclone(remote) => list_rclone(place, remote, min_size, inner, tx),
    }
}

fn list_ssh(place: usize, host: &str, path: &str, min_size: u64, inner: &Arc<Inner>, tx: &Sender<Msg>) -> Result<(), String> {
    use crate::scan::remote::{agent_path, prepare_ssh, sh_quote, ssh_command, unescape, Method};
    let progress = crate::scan::Progress::default();
    if !matches!(prepare_ssh(host, &progress)?, Method::Agent) {
        return Err("the server needs the SquirrelDisk agent, which could not be installed".into());
    }
    let mut child = ssh_command(host)
        .arg(format!("{} --agent dupes-list {} {min_size}", agent_path(), sh_quote(path)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let c = &inner.counters[place];
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut finished = false;
    for line in reader.lines() {
        if inner.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            return Err("cancelled".into());
        }
        let Ok(line) = line else { break };
        let mut it = line.split('\t');
        match it.next() {
            Some("F") => {
                let v: Vec<&str> = it.collect();
                if v.len() < 6 {
                    continue;
                }
                let n = |k: usize| v[k].parse::<u64>().unwrap_or(0);
                let f = Found { size: n(0), mtime: v[1].parse().unwrap_or(0), dev: n(2), ino: n(3), nlink: n(4) as u32, clone: None, path: unescape(v[5]) };
                let _ = tx.send(Msg::Found { place, f, cloud: None });
            }
            Some("P") => {
                let v: Vec<u64> = it.map(|x| x.parse().unwrap_or(0)).collect();
                if v.len() >= 2 {
                    c.files.store(v[0], Ordering::Relaxed);
                    c.bytes.store(v[1], Ordering::Relaxed);
                }
            }
            Some("D") => finished = true,
            _ => {}
        }
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut e, &mut err);
    }
    let _ = child.wait();
    if finished {
        Ok(())
    } else {
        Err(err.lines().last().unwrap_or("the server stopped answering").to_string())
    }
}

#[derive(serde::Deserialize)]
struct LsEntry {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Size", default)]
    size: i64,
    #[serde(rename = "ModTime", default)]
    mod_time: String,
    #[serde(rename = "Hashes", default)]
    hashes: HashMap<String, String>,
}

/// What identifies a file when the system gives no inode (Windows): its
/// path, with one kind of separator and no case (NTFS ignores case).
fn path_key(path: &str, windows: bool) -> String {
    if windows {
        path.replace('/', "\\").to_lowercase()
    } else {
        path.to_string()
    }
}

/// `remote:dir` + `a/b.jpg`
pub fn rclone_join(remote: &str, rel: &str) -> String {
    if remote.ends_with(':') || remote.ends_with('/') {
        format!("{remote}{rel}")
    } else {
        format!("{remote}/{rel}")
    }
}

fn list_rclone(place: usize, remote: &str, min_size: u64, inner: &Arc<Inner>, tx: &Sender<Msg>) -> Result<(), String> {
    let mut c = crate::rclone::command();
    let mut child = c
        .args(["lsjson", "-R", "--files-only", "--fast-list", "--no-mimetype", "--hash", "--hash-type", "MD5", "--hash-type", "SHA-1", remote])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run rclone: {e}"))?;
    let counters = &inner.counters[place];
    let reader = BufReader::new(child.stdout.take().unwrap());
    for line in reader.lines() {
        if inner.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            return Err("cancelled".into());
        }
        let Ok(line) = line else { break };
        let l = line.trim().trim_end_matches(',');
        if !l.starts_with('{') {
            continue;
        }
        let Ok(e) = serde_json::from_str::<LsEntry>(l) else { continue };
        let size = e.size.max(0) as u64;
        counters.files.fetch_add(1, Ordering::Relaxed);
        counters.bytes.fetch_add(size, Ordering::Relaxed);
        if size < min_size.max(1) {
            continue;
        }
        let md5 = e.hashes.get("md5").and_then(|h| hash::from_hex(h));
        let sha1 = e.hashes.get("sha1").and_then(|h| hash::from_hex(h));
        let f = Found { path: rclone_join(remote, &e.path), size, mtime: parse_rfc3339(&e.mod_time), dev: 0, ino: 0, nlink: 1, clone: None };
        let _ = tx.send(Msg::Found { place, f, cloud: Some((md5, sha1)) });
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut e, &mut err);
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("rclone failed").to_string())
    }
}

/// Seconds since the epoch of an RFC 3339 time (`2024-06-12T10:20:30.5+02:00`).
fn parse_rfc3339(s: &str) -> i64 {
    let num = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d), Some(hh), Some(mm), Some(ss)) = (num(0, 4), num(5, 7), num(8, 10), num(11, 13), num(14, 16), num(17, 19)) else {
        return 0;
    };
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut t = days * 86400 + hh * 3600 + mm * 60 + ss;
    // zone
    let rest = &s[19..];
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let h = rest.get(1..3).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        let mi = rest.get(4..6).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        let off = h * 3600 + mi * 60;
        t -= if sign == '+' { off } else { -off };
    }
    t
}

// ---------------------------------------------------------------------------
// fingerprinting

fn hash_worker(q: Arc<Queue>, tx: Sender<Msg>, inner: Arc<Inner>, kind: ExecKind) {
    let mut agent: Option<AgentConn> = None;
    while let Some(job) = q.pop() {
        if inner.cancel.load(Ordering::Relaxed) {
            break;
        }
        let result = match &kind {
            ExecKind::Local(_) => hash::file(Path::new(&job.path), job.kind, job.size, &inner.cancel, &inner.read),
            ExecKind::Ssh(host) => {
                let r = ssh_hash(&mut agent, host, &job);
                if r.is_err() {
                    agent = None;
                }
                inner.read.fetch_add(hash::cost(job.kind, job.size), Ordering::Relaxed);
                r
            }
            ExecKind::Rclone(_) => {
                let r = rclone_hash(&job);
                inner.read.fetch_add(job.size, Ordering::Relaxed);
                r
            }
        };
        if tx.send(Msg::Hashed { idx: job.idx, kind: job.kind, result }).is_err() {
            break;
        }
    }
}

/// A `--agent dupes-hash` process on a server, answering one file per line.
struct AgentConn {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for AgentConn {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ssh_hash(conn: &mut Option<AgentConn>, host: &str, job: &Job) -> Result<Vec<u8>, String> {
    use crate::scan::remote::{agent_path, escape, ssh_command, unescape};
    if conn.is_none() {
        let mut child = ssh_command(host)
            .arg(format!("{} --agent dupes-hash", agent_path()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        *conn = Some(AgentConn { child, stdin, stdout });
    }
    let c = conn.as_mut().unwrap();
    writeln!(c.stdin, "{}\t{}\t{}", job.kind.code(), job.size, escape(&job.path)).map_err(|e| e.to_string())?;
    c.stdin.flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    if c.stdout.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
        return Err("the server stopped answering".into());
    }
    let mut it = line.trim_end_matches('\n').splitn(2, '\t');
    match (it.next(), it.next()) {
        (Some("H"), Some(hex)) => hash::from_hex(hex).ok_or_else(|| "bad answer".into()),
        (Some("X"), Some(e)) => Err(unescape(e)),
        _ => Err("bad answer".into()),
    }
}

/// Cloud files the provider gave no fingerprint for: rclone reads them.
fn rclone_hash(job: &Job) -> Result<Vec<u8>, String> {
    let name = match job.kind {
        Kind::Md5 => "MD5",
        Kind::Sha1 => "SHA-1",
        _ => return Err("not available for cloud files".into()),
    };
    let out = crate::rclone::command()
        .args(["hashsum", name, "--download", &job.path])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run rclone: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().next().and_then(hash::from_hex).ok_or_else(|| "rclone could not read it".into())
}

// ---------------------------------------------------------------------------
// remembered fingerprints

const CACHE_MAX: usize = 2_000_000;
const CACHE_FILE: &str = "dupes-cache-v2.bin";
/// Quick, sampled and full fingerprints.
const SLOTS: usize = 3;

/// (device, inode, size, mtime) → quick, sampled and full fingerprints.
type CacheMap = HashMap<(u64, u64, u64, i64), [Option<[u8; 16]>; SLOTS]>;

fn cache_slot(kind: Kind) -> Option<usize> {
    match kind {
        Kind::Partial => Some(0),
        Kind::Sample => Some(1),
        Kind::Full => Some(2),
        _ => None,
    }
}

#[derive(Default)]
struct Cache {
    map: CacheMap,
    changed: bool,
}

impl Cache {
    fn path() -> Option<PathBuf> {
        if cfg!(test) {
            return None;
        }
        crate::settings::config_dir().map(|d| d.join(CACHE_FILE))
    }

    fn load() -> Cache {
        let mut c = Cache::default();
        let Some(bytes) = Self::path().and_then(|p| std::fs::read(p).ok()) else { return c };
        const REC: usize = 8 * 4 + 1 + 16 * SLOTS;
        for r in bytes.chunks_exact(REC) {
            let u = |k: usize| u64::from_le_bytes(r[k * 8..k * 8 + 8].try_into().unwrap());
            let has = r[32];
            let slot = |k: usize| (has & (1 << k) != 0).then(|| <[u8; 16]>::try_from(&r[33 + 16 * k..49 + 16 * k]).unwrap());
            c.map.insert((u(0), u(1), u(2), u(3) as i64), [slot(0), slot(1), slot(2)]);
        }
        c
    }

    fn get(&self, dev: u64, ino: u64, size: u64, mtime: i64, kind: Kind) -> Option<Vec<u8>> {
        let k = cache_slot(kind)?;
        self.map.get(&(dev, ino, size, mtime)).and_then(|v| v[k]).map(|h| h.to_vec())
    }

    fn put(&mut self, dev: u64, ino: u64, size: u64, mtime: i64, kind: Kind, h: &[u8]) {
        let Some(k) = cache_slot(kind) else { return };
        let Ok(h) = <[u8; 16]>::try_from(h) else { return };
        if self.map.len() >= CACHE_MAX && !self.map.contains_key(&(dev, ino, size, mtime)) {
            return;
        }
        self.map.entry((dev, ino, size, mtime)).or_default()[k] = Some(h);
        self.changed = true;
    }

    fn save(&self) {
        if !self.changed {
            return;
        }
        let Some(path) = Self::path() else { return };
        let mut out = Vec::with_capacity(self.map.len() * (33 + 16 * SLOTS));
        for ((dev, ino, size, mtime), slots) in &self.map {
            for v in [*dev, *ino, *size, *mtime as u64] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.push(slots.iter().enumerate().fold(0u8, |b, (k, h)| b | ((h.is_some() as u8) << k)));
            for h in slots {
                out.extend_from_slice(&h.unwrap_or_default());
            }
        }
        // the first format: start over
        if let Some(old) = path.parent().map(|d| d.join("dupes-cache.bin")) {
            let _ = std::fs::remove_file(old);
        }
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(f: &Finder) {
        let t = Instant::now();
        while !f.is_done() {
            assert!(t.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn finds_duplicates_across_places() {
        let dir = std::env::temp_dir().join(format!("sqd-dupes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["a/sub", "b", "c"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let big: Vec<u8> = (0..400_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut big2 = big.clone();
        big2[200_000] ^= 1; // same size and ends, different middle
        let small = vec![5u8; 3000];
        std::fs::write(dir.join("a/IMG_1.HEIC"), &big).unwrap();
        std::fs::write(dir.join("b/IMG_1.HEIC"), &big).unwrap();
        std::fs::write(dir.join("c/copy.HEIC"), &big).unwrap();
        std::fs::write(dir.join("a/sub/other.HEIC"), &big2).unwrap();
        std::fs::write(dir.join("a/s1"), &small).unwrap();
        std::fs::write(dir.join("b/s2"), &small).unwrap();
        std::fs::write(dir.join("c/alone"), vec![1u8; 3000]).unwrap();
        #[cfg(unix)]
        std::fs::hard_link(dir.join("a/s1"), dir.join("c/hardlink")).unwrap();
        let specs = vec![Spec::Local(dir.join("a")), Spec::Local(dir.join("b")), Spec::Local(dir.join("c")), Spec::Local(dir.join("a/sub"))];
        let f = Finder::start(specs.clone(), Options { min_size: 1000, exact: false });
        wait(&f);
        let mut seen = 0;
        let mut groups: Vec<FoundGroup> = f.changes_since(&mut seen).unwrap().into_iter().flat_map(|(_, g)| g).collect();
        groups.sort_by_key(|g| g.size);
        assert_eq!(groups.len(), 2, "{groups:#?}");
        assert_eq!(groups[0].size, 3000);
        assert_eq!(groups[0].files.len(), 2, "hard link counted once");
        assert_eq!(groups[1].size, 400_000);
        let names: Vec<&str> = groups[1].files.iter().map(|x| x.path.rsplit('/').next().unwrap()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(!names.contains(&"other.HEIC"));
        let st = f.status();
        assert!(st.done && !st.walking);
        assert_eq!(st.places.iter().filter(|p| p.error.is_some()).count(), 0);
        // exact mode finds the same
        let f = Finder::start(specs, Options { min_size: 1000, exact: true });
        wait(&f);
        let mut seen = 0;
        assert_eq!(f.changes_since(&mut seen).unwrap().into_iter().flat_map(|(_, g)| g).count(), 2);
        // nothing new since
        assert!(f.changes_since(&mut seen).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_paths_are_one_file() {
        assert_eq!(path_key("C:/Photos/a\\IMG.JPG", true), path_key("c:\\photos\\A\\img.jpg", true));
        assert_ne!(path_key("/a/IMG.JPG", false), path_key("/a/img.jpg", false));
    }

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), 0);
        assert_eq!(parse_rfc3339("2024-06-12T10:20:30.123+02:00"), 1718180430);
        assert_eq!(parse_rfc3339("garbage"), 0);
        assert_eq!(rclone_join("gdrive:", "a/b.jpg"), "gdrive:a/b.jpg");
        assert_eq!(rclone_join("gdrive:Foto", "a.jpg"), "gdrive:Foto/a.jpg");
    }
}
