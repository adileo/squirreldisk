//! Deletion jobs with progress, optional backup and safety re-checks.

use crate::safety::{Os, Rules, Verdict};
use crate::scan::remote::{hide_console, run_ssh, sh_quote};
use crate::tree::Source;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Trash,
    Permanent,
    /// Permanent delete after overwriting every file (DoD 5220.22-M, 3 passes).
    SecureErase,
    BackupFolder(PathBuf),
    BackupRclone(String),
}

impl Mode {
    pub fn has_backup(&self) -> bool {
        matches!(self, Mode::BackupFolder(_) | Mode::BackupRclone(_))
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub node: u32,
    pub path: String,
    pub size: u64,
}

#[derive(Default)]
pub struct DeleteProgress {
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub total_items: AtomicU64,
    pub done_items: AtomicU64,
    pub freed: AtomicU64,
    pub phase: Mutex<String>,
    pub current: Mutex<String>,
    pub errors: Mutex<Vec<String>>,
    /// Node ids whose deletion succeeded; drained by the UI.
    pub completed: Mutex<Vec<u32>>,
    pub finished: AtomicBool,
    pub cancel: AtomicBool,
    pub backup_location: Mutex<Option<String>>,
}

impl DeleteProgress {
    pub fn fraction(&self) -> f32 {
        let t = self.total_bytes.load(Ordering::Relaxed).max(1);
        (self.done_bytes.load(Ordering::Relaxed) as f64 / t as f64).clamp(0.0, 1.0) as f32
    }
    fn err(&self, s: String) {
        self.errors.lock().unwrap().push(s);
    }
    fn set_phase(&self, s: &str) {
        *self.phase.lock().unwrap() = s.to_string();
    }
    fn set_current(&self, s: &str) {
        *self.current.lock().unwrap() = s.to_string();
    }
}

pub fn start(items: Vec<Item>, mode: Mode, source: Source, root_path: String) -> Arc<DeleteProgress> {
    let p = Arc::new(DeleteProgress::default());
    let total: u64 = items.iter().map(|i| i.size).sum();
    let factor = if mode.has_backup() { 2 } else { 1 };
    p.total_bytes.store(total * factor, Ordering::Relaxed);
    p.total_items.store(items.len() as u64, Ordering::Relaxed);
    let pc = p.clone();
    std::thread::Builder::new()
        .name("delete".into())
        .spawn(move || {
            run(&items, &mode, &source, &root_path, &pc);
            pc.set_current("");
            pc.finished.store(true, Ordering::Relaxed);
        })
        .expect("spawn delete thread");
    p
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}{:02}", tod / 3600, (tod / 60) % 60, tod % 60)
}

/// Path of `path` relative to the scanned root, safe to append under a backup dir.
fn relative_for_backup(path: &str, root: &str) -> PathBuf {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut out = PathBuf::new();
    for comp in rel.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".." && *c != ".") {
        out.push(comp.trim_end_matches(':'));
    }
    if out.as_os_str().is_empty() {
        out.push("root");
    }
    out
}

fn run(items: &[Item], mode: &Mode, source: &Source, root: &str, p: &DeleteProgress) {
    let rules = match source {
        Source::Local => Rules::current(),
        Source::Ssh { host } => {
            let home = run_ssh(host, "echo $HOME").ok().map(|h| h.trim().to_string());
            Rules::for_os(Os::Linux, home.as_deref())
        }
        Source::Rclone { .. } | Source::Demo => Rules::for_os(Os::Linux, None),
    };
    if *source == Source::Demo {
        return simulate(items, mode, p);
    }
    let backup_root = match mode {
        Mode::BackupFolder(dest) => {
            let d = dest.join(format!("SquirrelDisk Backup {}", timestamp()));
            *p.backup_location.lock().unwrap() = Some(d.to_string_lossy().into_owned());
            Some(d)
        }
        _ => None,
    };
    for item in items {
        if p.cancel.load(Ordering::Relaxed) {
            p.err("cancelled by user".into());
            break;
        }
        p.set_current(&item.path);
        // Re-check right before acting: the rules are the last line of defence.
        if !matches!(source, Source::Rclone { .. }) {
            if let Verdict::Forbidden(why) = rules.check(&item.path) {
                p.err(format!("{}: refused, {why}", item.path));
                continue;
            }
        }
        let before = p.done_bytes.load(Ordering::Relaxed);
        let result: Result<(), String> = (|| {
            // 1) backup
            match mode {
                Mode::BackupFolder(_) => {
                    let dest = backup_root.as_ref().unwrap().join(relative_for_backup(&item.path, root));
                    if source.is_local() && Path::new(&dest).starts_with(&item.path) {
                        return Err("backup destination is inside the item being deleted".into());
                    }
                    p.set_phase("Backing up");
                    backup_to_folder(source, &item.path, &dest, p)?;
                }
                Mode::BackupRclone(remote) => {
                    p.set_phase("Uploading backup");
                    let rel = relative_for_backup(&item.path, root);
                    let dest = format!("{}/SquirrelDisk Backup {}/{}", remote.trim_end_matches('/'), timestamp_day(), rel.to_string_lossy().replace('\\', "/"));
                    *p.backup_location.lock().unwrap() = Some(dest.clone());
                    backup_rclone(source, &item.path, &dest)?;
                    p.done_bytes.fetch_add(item.size, Ordering::Relaxed);
                }
                _ => {}
            }
            // 2) delete
            let base = p.done_bytes.load(Ordering::Relaxed);
            match (source, mode) {
                (Source::Local, Mode::Trash) => {
                    p.set_phase("Moving to Trash");
                    trash::delete(&item.path).map_err(|e| e.to_string())?;
                }
                (Source::Local, Mode::SecureErase) => {
                    p.set_phase("Securely erasing");
                    remove_all_secure(Path::new(&item.path), p)?;
                }
                (Source::Local, _) => {
                    p.set_phase("Deleting");
                    remove_all(Path::new(&item.path), p)?;
                }
                (Source::Ssh { host }, _) => {
                    p.set_phase("Deleting on server");
                    let out = run_ssh(host, &format!("rm -rf -- {} && echo SQD_OK", sh_quote(&item.path)))?;
                    if !out.contains("SQD_OK") {
                        return Err("remote rm failed".into());
                    }
                }
                (Source::Rclone { .. }, _) => {
                    p.set_phase("Deleting from cloud");
                    rclone_delete(&item.path)?;
                }
                (Source::Demo, _) => unreachable!("demo deletions are simulated"),
            }
            // normalise the progress to the item size
            p.done_bytes.store(base + item.size, Ordering::Relaxed);
            Ok(())
        })();
        match result {
            Ok(()) => {
                p.freed.fetch_add(item.size, Ordering::Relaxed);
                p.completed.lock().unwrap().push(item.node);
            }
            Err(e) => {
                p.err(format!("{}: {e}", item.path));
                let factor = if mode.has_backup() { 2 } else { 1 };
                p.done_bytes.store(before + item.size * factor, Ordering::Relaxed);
            }
        }
        p.done_items.fetch_add(1, Ordering::Relaxed);
    }
}

/// `MARKETING_DEMO`: pretend to delete, touching nothing.
fn simulate(items: &[Item], mode: &Mode, p: &DeleteProgress) {
    p.set_phase(match mode {
        Mode::Trash => "Moving to Trash",
        Mode::SecureErase => "Securely erasing",
        _ => "Deleting",
    });
    for item in items {
        p.set_current(&item.path);
        let base = p.done_bytes.load(Ordering::Relaxed);
        for k in 1..=12u64 {
            std::thread::sleep(std::time::Duration::from_millis(45));
            p.done_bytes.store(base + item.size * k / 12, Ordering::Relaxed);
        }
        p.freed.fetch_add(item.size, Ordering::Relaxed);
        p.completed.lock().unwrap().push(item.node);
        p.done_items.fetch_add(1, Ordering::Relaxed);
    }
}

fn timestamp_day() -> String {
    timestamp()
}

/// Removes a file or directory tree without following symlinks, reporting progress.
pub fn remove_all(path: &Path, p: &DeleteProgress) -> Result<(), String> {
    let md = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if md.is_dir() {
        let rd = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for e in rd.flatten() {
            if p.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            remove_all(&e.path(), p)?;
        }
        std::fs::remove_dir(path).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        let size = crate::scan::platform::info(&md, path).alloc;
        if let Err(e) = std::fs::remove_file(path) {
            // Windows refuses to delete read-only files.
            #[allow(clippy::permissions_set_readonly_false)]
            {
                let mut perm = md.permissions();
                if perm.readonly() {
                    perm.set_readonly(false);
                    let _ = std::fs::set_permissions(path, perm);
                    std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
                } else {
                    return Err(format!("{}: {e}", path.display()));
                }
            }
        }
        p.done_bytes.fetch_add(size, Ordering::Relaxed);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Secure erase
//
// Each regular file is overwritten in place following DoD 5220.22-M (ECE):
// pass 1 zeros, pass 2 ones, pass 3 random data, each flushed to disk with
// fsync. The file is then renamed to a random name (so the original name
// doesn't linger in the directory), truncated to zero and removed.
//
// Caveat, surfaced in the UI: on SSDs (wear levelling) and copy-on-write file
// systems (APFS, Btrfs, ZFS) the drive may write the new data elsewhere and
// keep the old blocks until they are trimmed. Full-disk encryption
// (FileVault, BitLocker, LUKS) is what really protects data on those.

const OVERWRITE_PASSES: usize = 3;

/// Fast PRNG (xoshiro256**) for the random pass, seeded from the OS.
struct Xoshiro([u64; 4]);

impl Xoshiro {
    fn from_os() -> Self {
        let mut seed = [0u8; 32];
        let from_os = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut seed));
        if from_os.is_err() {
            // Windows (or no /dev/urandom): mix time, pid and addresses.
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let mix = [t as u64, (t >> 64) as u64 ^ std::process::id() as u64, &seed as *const _ as u64, 0x9E37_79B9_7F4A_7C15];
            for (i, m) in mix.iter().enumerate() {
                seed[i * 8..i * 8 + 8].copy_from_slice(&m.to_le_bytes());
            }
        }
        let mut s = [0u64; 4];
        for (i, v) in s.iter_mut().enumerate() {
            *v = u64::from_le_bytes(seed[i * 8..i * 8 + 8].try_into().unwrap()) | 1;
        }
        Xoshiro(s)
    }
    fn next(&mut self) -> u64 {
        let r = (self.0[1].wrapping_mul(5)).rotate_left(7).wrapping_mul(9);
        let t = self.0[1] << 17;
        self.0[2] ^= self.0[0];
        self.0[3] ^= self.0[1];
        self.0[1] ^= self.0[2];
        self.0[0] ^= self.0[3];
        self.0[2] ^= t;
        self.0[3] = self.0[3].rotate_left(45);
        r
    }
    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

/// Overwrites the whole content of `path` with the DoD passes.
/// `on_pass` is called after each pass has been flushed to disk.
fn overwrite_file(path: &Path, len: u64, cancel: &AtomicBool, mut on_pass: impl FnMut()) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    let open = || std::fs::OpenOptions::new().write(true).open(path);
    let mut f = match open() {
        Ok(f) => f,
        Err(_) => {
            // read-only files (Windows attribute / unix mode): allow writing first
            if let Ok(md) = std::fs::metadata(path) {
                let mut perm = md.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                let _ = std::fs::set_permissions(path, perm);
            }
            open().map_err(|e| format!("{}: {e}", path.display()))?
        }
    };
    let mut buf = vec![0u8; 1 << 20];
    let mut rng = Xoshiro::from_os();
    for pass in 0..OVERWRITE_PASSES {
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        match pass {
            0 => buf.fill(0x00),
            1 => buf.fill(0xFF),
            _ => {}
        }
        let mut left = len;
        while left > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            if pass == 2 {
                rng.fill(&mut buf);
            }
            let n = left.min(buf.len() as u64) as usize;
            f.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", path.display()))?;
            left -= n as u64;
        }
        f.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
        on_pass();
    }
    Ok(())
}

fn shred_file(path: &Path, md: &std::fs::Metadata, p: &DeleteProgress) -> Result<(), String> {
    let alloc = crate::scan::platform::info(md, path).alloc;
    // A file with other hard links is shared: overwriting it would destroy
    // the data seen through the other names too, so just unlink it.
    #[cfg(unix)]
    let shared = {
        use std::os::unix::fs::MetadataExt;
        md.nlink() > 1
    };
    #[cfg(not(unix))]
    let shared = false;
    let mut credited = 0u64;
    if md.len() > 0 && !shared {
        overwrite_file(path, md.len(), &p.cancel, || {
            let step = alloc / OVERWRITE_PASSES as u64;
            p.done_bytes.fetch_add(step, Ordering::Relaxed);
            credited += step;
        })?;
    }
    // hide the original name, drop the length, then remove
    let mut rng = Xoshiro::from_os();
    let scrambled = path.with_file_name(format!("{:016x}", rng.next()));
    let target = if std::fs::rename(path, &scrambled).is_ok() { scrambled } else { path.to_path_buf() };
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&target) {
        let _ = f.set_len(0);
        let _ = f.sync_all();
    }
    std::fs::remove_file(&target).map_err(|e| format!("{}: {e}", path.display()))?;
    p.done_bytes.fetch_add(alloc.saturating_sub(credited), Ordering::Relaxed);
    Ok(())
}

/// Like [`remove_all`], but securely erases every regular file first.
pub fn remove_all_secure(path: &Path, p: &DeleteProgress) -> Result<(), String> {
    let md = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if md.is_dir() {
        let rd = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for e in rd.flatten() {
            if p.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            remove_all_secure(&e.path(), p)?;
        }
        std::fs::remove_dir(path).map_err(|e| format!("{}: {e}", path.display()))
    } else if md.is_file() {
        shred_file(path, &md, p)
    } else {
        // symlinks and special files: remove the entry, never follow it
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn copy_all(src: &Path, dst: &Path, p: &DeleteProgress) -> Result<(), String> {
    let md = std::fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if md.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let target = std::fs::read_link(src).map_err(|e| e.to_string())?;
            std::os::unix::fs::symlink(target, dst).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if md.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        for e in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?.flatten() {
            if p.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            copy_all(&e.path(), &dst.join(e.file_name()), p)?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let n = std::fs::copy(src, dst).map_err(|e| format!("{}: {e}", src.display()))?;
        if n != md.len() {
            return Err(format!("{}: incomplete copy", src.display()));
        }
        p.done_bytes.fetch_add(crate::scan::platform::info(&md, src).alloc, Ordering::Relaxed);
        Ok(())
    }
}

fn backup_to_folder(source: &Source, path: &str, dest: &Path, p: &DeleteProgress) -> Result<(), String> {
    match source {
        Source::Demo => Ok(()),
        Source::Local => copy_all(Path::new(path), dest, p),
        Source::Ssh { host } => {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut c = Command::new("scp");
            crate::scan::remote::apply_ssh_options(&mut c, host);
            let st = c
                .args(["-r", "-p", "-q"])
                .arg(format!("{host}:{}", sh_quote(path)))
                .arg(dest)
                .stdin(Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
            if st.success() { Ok(()) } else { Err("scp backup failed".into()) }
        }
        Source::Rclone { .. } => {
            let mut c = Command::new("rclone");
            hide_console(&mut c);
            let st = c.arg("copyto").arg(path).arg(dest).stdin(Stdio::null()).status().map_err(|e| e.to_string())?;
            if st.success() { Ok(()) } else { Err("rclone backup failed".into()) }
        }
    }
}

fn backup_rclone(source: &Source, path: &str, dest: &str) -> Result<(), String> {
    if !source.is_local() {
        return Err("cloud backup is only available for local files".into());
    }
    let mut c = Command::new("rclone");
    hide_console(&mut c);
    let st = c.arg("copyto").arg(path).arg(dest).stdin(Stdio::null()).status().map_err(|e| format!("cannot run rclone: {e}"))?;
    if st.success() { Ok(()) } else { Err("rclone upload failed".into()) }
}

fn rclone_delete(path: &str) -> Result<(), String> {
    let mut c = Command::new("rclone");
    hide_console(&mut c);
    // `purge` handles directories, `deletefile` single objects.
    let st = c.arg("purge").arg(path).stdin(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
    if st.success() {
        return Ok(());
    }
    let mut c = Command::new("rclone");
    hide_console(&mut c);
    let st = c.arg("deletefile").arg(path).stdin(Stdio::null()).status().map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err("rclone delete failed".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fake_dir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("sqd-test-del-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("junk/deep")).unwrap();
        fs::write(base.join("junk/a.bin"), vec![0u8; 10_000]).unwrap();
        fs::write(base.join("junk/deep/b.bin"), vec![0u8; 20_000]).unwrap();
        fs::write(base.join("keep.txt"), b"keep me").unwrap();
        base
    }

    fn wait(p: &DeleteProgress) {
        while !p.finished.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn permanent_delete_only_target() {
        let base = fake_dir("perm");
        let target = base.join("junk");
        let items = vec![Item { node: 7, path: target.to_string_lossy().into(), size: 30_000 }];
        let p = start(items, Mode::Permanent, Source::Local, base.to_string_lossy().into());
        wait(&p);
        assert!(p.errors.lock().unwrap().is_empty(), "{:?}", p.errors.lock().unwrap());
        assert!(!target.exists());
        assert!(base.join("keep.txt").exists());
        assert_eq!(*p.completed.lock().unwrap(), vec![7]);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn backup_then_delete() {
        let base = fake_dir("bak");
        let dest = std::env::temp_dir().join(format!("sqd-test-bakdest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);
        fs::create_dir_all(&dest).unwrap();
        let target = base.join("junk");
        let items = vec![Item { node: 1, path: target.to_string_lossy().into(), size: 30_000 }];
        let p = start(items, Mode::BackupFolder(dest.clone()), Source::Local, base.to_string_lossy().into());
        wait(&p);
        assert!(p.errors.lock().unwrap().is_empty(), "{:?}", p.errors.lock().unwrap());
        assert!(!target.exists());
        let loc = PathBuf::from(p.backup_location.lock().unwrap().clone().unwrap());
        assert_eq!(fs::read(loc.join("junk/deep/b.bin")).unwrap().len(), 20_000);
        fs::remove_dir_all(&base).unwrap();
        fs::remove_dir_all(&dest).unwrap();
    }

    #[test]
    fn overwrite_replaces_every_byte() {
        let base = fake_dir("shred");
        let f = base.join("secret.txt");
        let original = b"top secret payload ".repeat(5000);
        fs::write(&f, &original).unwrap();
        let mut passes = 0;
        overwrite_file(&f, original.len() as u64, &AtomicBool::new(false), || passes += 1).unwrap();
        assert_eq!(passes, OVERWRITE_PASSES);
        let after = fs::read(&f).unwrap();
        assert_eq!(after.len(), original.len());
        assert!(!after.windows(10).any(|w| w == b"top secret"));
        // last pass is random: not all zeros / ones
        assert!(after.iter().any(|b| *b != 0x00 && *b != 0xFF));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn secure_erase_removes_target_only() {
        let base = fake_dir("secure");
        let target = base.join("junk");
        let items = vec![Item { node: 3, path: target.to_string_lossy().into(), size: 30_000 }];
        let p = start(items, Mode::SecureErase, Source::Local, base.to_string_lossy().into());
        wait(&p);
        assert!(p.errors.lock().unwrap().is_empty(), "{:?}", p.errors.lock().unwrap());
        assert!(!target.exists());
        assert!(base.join("keep.txt").exists());
        assert_eq!(fs::read_dir(&base).unwrap().count(), 1);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn refuses_forbidden() {
        let p = start(vec![Item { node: 1, path: "/".into(), size: 1 }], Mode::Permanent, Source::Local, "/".into());
        wait(&p);
        assert_eq!(p.errors.lock().unwrap().len(), 1);
        assert!(p.completed.lock().unwrap().is_empty());
    }
}
