//! OS specific bits of scanning: allocated size, device ids, special folders.

use std::fs::Metadata;
use std::path::Path;

/// Information about one directory entry, gathered with a single `lstat`
/// (or directly from `FindNextFile` data on Windows).
pub struct EntryInfo {
    /// Bytes actually allocated on disk.
    pub alloc: u64,
    pub dev: u64,
    /// (dev, inode) when the file has more than one hard link.
    pub hardlink: Option<(u64, u64)>,
}

#[cfg(unix)]
pub fn entry_info(md: &Metadata) -> EntryInfo {
    use std::os::unix::fs::MetadataExt;
    // st_blocks is in 512-byte units on every unix we care about. It accounts for
    // sparse files and filesystem compression (APFS/btrfs/zfs).
    #[cfg(target_os = "macos")]
    let alloc = if is_dataless(md) { 0 } else { md.blocks() * 512 };
    #[cfg(not(target_os = "macos"))]
    let alloc = md.blocks() * 512;
    let hardlink = if !md.is_dir() && md.nlink() > 1 { Some((md.dev(), md.ino())) } else { None };
    EntryInfo { alloc, dev: md.dev(), hardlink }
}

#[cfg(windows)]
pub fn entry_info_path(md: &Metadata, path: &Path) -> EntryInfo {
    use std::os::windows::fs::MetadataExt;
    const COMPRESSED: u32 = 0x800;
    const SPARSE: u32 = 0x200;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x40000;
    const RECALL_ON_DATA: u32 = 0x400000;
    let attrs = md.file_attributes();
    let len = md.len();
    let alloc = if attrs & (OFFLINE | RECALL_ON_DATA) != 0 {
        // Cloud placeholder (OneDrive, Dropbox, Google Drive…): data is online only.
        0
    } else if attrs & (COMPRESSED | SPARSE | RECALL_ON_OPEN) != 0 {
        // Compressed / sparse / partially hydrated files: ask NTFS.
        compressed_size(path).unwrap_or(len)
    } else {
        // Round up to the typical 4 KiB cluster; tiny files may live in the MFT.
        if len <= 700 { 0 } else { (len + 4095) & !4095 }
    };
    EntryInfo { alloc, dev: 0, hardlink: None }
}

#[cfg(windows)]
fn compressed_size(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut high: u32 = 0;
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == u32::MAX {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error().unwrap_or(0) != 0 {
            return None;
        }
    }
    Some(((high as u64) << 32) | low as u64)
}

/// Cross-platform entry point.
#[allow(unused_variables)]
pub fn info(md: &Metadata, path: &Path) -> EntryInfo {
    #[cfg(unix)]
    {
        entry_info(md)
    }
    #[cfg(windows)]
    {
        entry_info_path(md, path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        EntryInfo { alloc: md.len(), dev: 0, hardlink: None }
    }
}

pub fn device_of(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path).ok().map(|m| m.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Some(0)
    }
}

/// Devices that belong to the same logical volume as `root`.
/// On macOS the startup disk is split in a read-only System volume and a Data
/// volume glued together with firmlinks: we treat both as one.
pub fn allowed_devices(root: &Path) -> Vec<u64> {
    let mut v = Vec::new();
    if let Some(d) = device_of(root) {
        v.push(d);
    }
    #[cfg(target_os = "macos")]
    if root == Path::new("/") {
        if let Some(d) = device_of(Path::new("/System/Volumes/Data")) {
            v.push(d);
        }
    }
    v
}

/// Paths that must never be descended into while scanning `root`.
pub fn skip_paths(root: &Path) -> Vec<std::path::PathBuf> {
    #[allow(unused_mut)]
    let mut v: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    if root == Path::new("/") {
        // The Data volume is already reachable through firmlinks (/Users, /Applications, ...).
        v.push("/System/Volumes/Data".into());
    }
    #[cfg(target_os = "linux")]
    {
        for p in ["/proc", "/sys", "/dev", "/run", "/snap"] {
            if Path::new(p) != root {
                v.push(p.into());
            }
        }
    }
    let _ = root;
    v
}

/// Size of the whole volume containing `path`: (total, available).
pub fn volume_space(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
        if ok == 0 {
            return None;
        }
        Some((total, avail))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Is `path` the root of a volume (i.e. is the reported "hidden space" meaningful)?
pub fn is_volume_root(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    #[cfg(unix)]
    {
        match (device_of(path), path.parent().and_then(device_of)) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// BSD flag set on File Provider placeholders whose data is not on disk.
#[cfg(target_os = "macos")]
pub const SF_DATALESS: u32 = 0x4000_0000;

/// macOS: is this a cloud placeholder (dataless file or folder)?
#[cfg(target_os = "macos")]
pub fn is_dataless(md: &Metadata) -> bool {
    use std::os::darwin::fs::MetadataExt;
    md.st_flags() & SF_DATALESS != 0
}

/// One entry returned by the fast bulk directory lister.
#[allow(dead_code)]
pub struct RawEntry {
    pub name: String,
    pub kind: RawKind,
    pub alloc: u64,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum RawKind {
    File,
    Dir,
    Link,
    MountPoint,
    Other,
}

/// macOS: lists a directory with `getattrlistbulk`, which returns names, types,
/// allocated sizes and ids for many entries per syscall (instead of one
/// `lstat` per file). Roughly 3-5x faster on APFS.
#[cfg(target_os = "macos")]
pub fn list_bulk(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const VREG: u32 = 1;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;

    let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let _guard = Fd(fd);
    let mut al = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_FLAGS
            | libc::ATTR_CMN_FILEID
            | ATTR_CMN_ERROR,
        volattr: 0,
        dirattr: libc::ATTR_DIR_MOUNTSTATUS,
        fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE,
        forkattr: 0,
    };
    let mut buf = vec![0u8; 128 * 1024];
    let mut out = Vec::new();
    let rd_u32 = |b: &[u8], o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
    let rd_u64 = |b: &[u8], o: usize| u64::from_ne_bytes(b[o..o + 8].try_into().unwrap());
    loop {
        let n = unsafe {
            libc::getattrlistbulk(
                fd,
                &mut al as *mut _ as *mut libc::c_void,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            break;
        }
        let mut off = 0usize;
        for _ in 0..n {
            let len = rd_u32(&buf, off) as usize;
            let e = &buf[off..off + len];
            let mut p = 4;
            let returned_common = rd_u32(e, p);
            let returned_dir = rd_u32(e, p + 8);
            let returned_file = rd_u32(e, p + 12);
            p += 20;
            // name
            let name_ref = p;
            let name_off = rd_u32(e, p) as i32;
            let name_len = rd_u32(e, p + 4) as usize;
            p += 8;
            let start = (name_ref as i64 + name_off as i64) as usize;
            let raw = &e[start..(start + name_len).min(e.len())];
            let raw = raw.split(|b| *b == 0).next().unwrap_or(raw);
            let name = String::from_utf8_lossy(raw).into_owned();
            let mut dev = 0;
            if returned_common & libc::ATTR_CMN_DEVID != 0 {
                dev = rd_u32(e, p) as u64;
                p += 4;
            }
            let mut objtype = 0;
            if returned_common & libc::ATTR_CMN_OBJTYPE != 0 {
                objtype = rd_u32(e, p);
                p += 4;
            }
            let mut bsd_flags = 0;
            if returned_common & libc::ATTR_CMN_FLAGS != 0 {
                bsd_flags = rd_u32(e, p);
                p += 4;
            }
            let mut ino = 0;
            if returned_common & libc::ATTR_CMN_FILEID != 0 {
                ino = rd_u64(e, p);
                p += 8;
            }
            let mut err = 0;
            if returned_common & ATTR_CMN_ERROR != 0 {
                err = rd_u32(e, p);
                p += 4;
            }
            let mut mount = 0;
            if returned_dir & libc::ATTR_DIR_MOUNTSTATUS != 0 {
                mount = rd_u32(e, p);
                p += 4;
            }
            let mut nlink = 1;
            if returned_file & libc::ATTR_FILE_LINKCOUNT != 0 {
                nlink = rd_u32(e, p);
                p += 4;
            }
            let mut alloc = 0;
            if returned_file & libc::ATTR_FILE_ALLOCSIZE != 0 && p + 8 <= e.len() {
                alloc = rd_u64(e, p);
            }
            off += len;
            if returned_common & ATTR_CMN_ERROR != 0 && err != 0 {
                continue;
            }
            // Cloud placeholders (iCloud, Dropbox, Google Drive, OneDrive via
            // File Provider): contents live online, nothing to count, and
            // listing a dataless folder could trigger a download.
            let dataless = bsd_flags & SF_DATALESS != 0;
            if dataless {
                alloc = 0;
            }
            let kind = match objtype {
                VDIR if dataless => RawKind::Other,
                VDIR => {
                    if returned_dir & libc::ATTR_DIR_MOUNTSTATUS != 0 && mount & libc::DIR_MNTSTATUS_MNTPOINT != 0 {
                        RawKind::MountPoint
                    } else {
                        RawKind::Dir
                    }
                }
                VREG => RawKind::File,
                VLNK => RawKind::Link,
                _ => RawKind::Other,
            };
            out.push(RawEntry {
                name,
                kind,
                alloc,
                dev,
                ino,
                nlink,
            });
        }
    }
    Ok(out)
}
