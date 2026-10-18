//! Content fingerprints of files: a quick one from the first and last bytes,
//! a sampled one from bytes spread over the whole file, and full ones over
//! every byte. The agent computes the same ones on
//! servers, so files can be compared across machines.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Bytes read at each end of a file for the quick fingerprint. Files up to
/// twice this are read whole, so their quick fingerprint is the full one.
pub const PART: u64 = 64 * 1024;

/// Files up to this size are read whole for the sampled fingerprint (it is
/// then the full one): reading them is about as quick as seeking around.
pub const SAMPLE_WHOLE: u64 = 4 << 20;
/// Bytes read at each sampled spot.
const CHUNK: u64 = 32 * 1024;

/// How many spots inside the file the sampled fingerprint reads: one per
/// 32 MiB, between 16 and 64 (at most 2 MiB read, even for a 100 GB file).
fn spots(size: u64) -> u64 {
    (size / (32 << 20)).clamp(16, 64)
}

/// Bytes a fingerprint reads.
pub fn cost(kind: Kind, size: u64) -> u64 {
    match kind {
        Kind::Partial => (2 * PART).min(size),
        Kind::Sample if size > SAMPLE_WHOLE => 2 * PART + spots(size) * CHUNK,
        _ => size,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Kind {
    /// First and last [`PART`] bytes (xxh3-128).
    Partial,
    /// First and last [`PART`] bytes plus [`spots`] chunks spread evenly in
    /// between (xxh3-128). Files that differ anywhere but there would look
    /// alike: for photos and videos, whose bytes all change with any edit
    /// or new encode, that does not happen in practice.
    Sample,
    /// Every byte (xxh3-128).
    Full,
    /// MD5 and SHA-1: what cloud storage reports, for comparing with it.
    Md5,
    Sha1,
}

impl Kind {
    pub fn code(self) -> char {
        match self {
            Kind::Partial => 'p',
            Kind::Sample => 'q',
            Kind::Full => 'f',
            Kind::Md5 => 'm',
            Kind::Sha1 => 's',
        }
    }
    pub fn from_code(c: &str) -> Option<Kind> {
        match c {
            "p" => Some(Kind::Partial),
            "q" => Some(Kind::Sample),
            "f" => Some(Kind::Full),
            "m" => Some(Kind::Md5),
            "s" => Some(Kind::Sha1),
            _ => None,
        }
    }
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

enum Hasher {
    Xx(Box<xxhash_rust::xxh3::Xxh3>),
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
}

impl Hasher {
    fn new(kind: Kind) -> Hasher {
        use sha1::Digest;
        match kind {
            Kind::Partial | Kind::Sample | Kind::Full => Hasher::Xx(Box::default()),
            Kind::Md5 => Hasher::Md5(md5::Md5::new()),
            Kind::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
        }
    }
    fn update(&mut self, b: &[u8]) {
        use sha1::Digest;
        match self {
            Hasher::Xx(h) => h.update(b),
            Hasher::Md5(h) => h.update(b),
            Hasher::Sha1(h) => h.update(b),
        }
    }
    fn finish(self) -> Vec<u8> {
        use sha1::Digest;
        match self {
            Hasher::Xx(h) => h.digest128().to_be_bytes().to_vec(),
            Hasher::Md5(h) => h.finalize().to_vec(),
            Hasher::Sha1(h) => h.finalize().to_vec(),
        }
    }
}

/// Fingerprints `path`, which must still be `size` bytes long. `read` counts
/// the bytes read as they go; `cancel` stops it.
pub fn file(path: &Path, kind: Kind, size: u64, cancel: &AtomicBool, read: &AtomicU64) -> Result<Vec<u8>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len != size {
        return Err("changed while comparing".into());
    }
    let mut h = Hasher::new(kind);
    if kind == Kind::Partial && size > 2 * PART {
        let mut buf = vec![0u8; PART as usize];
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        h.update(&buf);
        f.seek(SeekFrom::End(-(PART as i64))).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        h.update(&buf);
        h.update(&size.to_le_bytes());
        read.fetch_add(2 * PART, Ordering::Relaxed);
        return Ok(h.finish());
    }
    if kind == Kind::Sample && size > SAMPLE_WHOLE {
        let mut buf = vec![0u8; PART as usize];
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        h.update(&buf);
        let n = spots(size);
        let mut chunk = vec![0u8; CHUNK as usize];
        for i in 1..=n {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let at = (size - CHUNK) * i / (n + 1);
            f.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
            f.read_exact(&mut chunk).map_err(|e| e.to_string())?;
            h.update(&chunk);
        }
        f.seek(SeekFrom::End(-(PART as i64))).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        h.update(&buf);
        h.update(&size.to_le_bytes());
        read.fetch_add(cost(kind, size), Ordering::Relaxed);
        return Ok(h.finish());
    }
    // big files: read past the cache, they would only push out what is useful
    #[cfg(target_os = "macos")]
    if size > 32 << 20 {
        use std::os::unix::io::AsRawFd;
        unsafe { libc::fcntl(f.as_raw_fd(), libc::F_NOCACHE, 1) };
    }
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        read.fetch_add(n as u64, Ordering::Relaxed);
    }
    if total != size {
        return Err("changed while comparing".into());
    }
    Ok(h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_files_partial_equals_full() {
        let dir = std::env::temp_dir().join(format!("sqd-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (no, read) = (AtomicBool::new(false), AtomicU64::new(0));
        let small = dir.join("small");
        std::fs::write(&small, vec![7u8; 5000]).unwrap();
        let a = file(&small, Kind::Partial, 5000, &no, &read).unwrap();
        let b = file(&small, Kind::Full, 5000, &no, &read).unwrap();
        assert_eq!(a, b);
        // big files: same ends, different middle → same quick, different full
        let mut x = vec![1u8; 300_000];
        let mut y = x.clone();
        y[150_000] = 2;
        let (fx, fy) = (dir.join("x"), dir.join("y"));
        std::fs::write(&fx, &x).unwrap();
        std::fs::write(&fy, &y).unwrap();
        let n = x.len() as u64;
        assert_eq!(file(&fx, Kind::Partial, n, &no, &read).unwrap(), file(&fy, Kind::Partial, n, &no, &read).unwrap());
        assert_ne!(file(&fx, Kind::Full, n, &no, &read).unwrap(), file(&fy, Kind::Full, n, &no, &read).unwrap());
        x[0] = 9;
        std::fs::write(&fx, &x).unwrap();
        assert_ne!(file(&fx, Kind::Partial, n, &no, &read).unwrap(), file(&fy, Kind::Partial, n, &no, &read).unwrap());
        // sampled: a file read whole when small, spots when big
        assert_eq!(file(&fx, Kind::Sample, n, &no, &read).unwrap(), file(&fx, Kind::Full, n, &no, &read).unwrap());
        let big: Vec<u8> = (0..(6u32 << 20)).map(|i| (i % 253) as u8).collect();
        let mut big2 = big.clone();
        let (fb, fb2) = (dir.join("big"), dir.join("big2"));
        let m = big.len() as u64;
        let spot = ((m - CHUNK) * 3 / (spots(m) + 1)) as usize;
        big2[spot + 10] ^= 1;
        std::fs::write(&fb, &big).unwrap();
        std::fs::write(&fb2, &big2).unwrap();
        assert_ne!(file(&fb, Kind::Sample, m, &no, &read).unwrap(), file(&fb2, Kind::Sample, m, &no, &read).unwrap());
        assert_eq!(file(&fb, Kind::Partial, m, &no, &read).unwrap(), file(&fb2, Kind::Partial, m, &no, &read).unwrap());
        assert_eq!(cost(Kind::Sample, m), 2 * PART + 16 * CHUNK);
        // md5 of "abc"
        let abc = dir.join("abc");
        std::fs::write(&abc, b"abc").unwrap();
        assert_eq!(to_hex(&file(&abc, Kind::Md5, 3, &no, &read).unwrap()), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(to_hex(&file(&abc, Kind::Sha1, 3, &no, &read).unwrap()), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(from_hex("00ff10"), Some(vec![0, 255, 16]));
        assert!(file(&abc, Kind::Full, 4, &no, &read).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
