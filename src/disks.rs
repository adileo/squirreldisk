//! Lists the mounted volumes worth scanning.

use sysinfo::Disks;

#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
    pub fs: String,
    pub is_boot: bool,
}

impl Volume {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
    pub fn used_frac(&self) -> f32 {
        if self.total == 0 { 0.0 } else { self.used() as f32 / self.total as f32 }
    }
}

const PSEUDO_FS: &[&str] = &[
    "tmpfs", "devtmpfs", "devfs", "proc", "sysfs", "overlay", "squashfs", "autofs", "cgroup", "cgroup2",
    "efivarfs", "fuse.portal", "nullfs", "ramfs", "securityfs", "tracefs", "debugfs", "binfmt_misc",
];

pub fn list() -> Vec<Volume> {
    if crate::scan::demo::enabled() {
        return crate::scan::demo::volumes()
            .into_iter()
            .map(|(name, mount, total, available, removable)| Volume {
                name: name.into(),
                mount: mount.into(),
                total,
                available,
                removable,
                fs: "apfs".into(),
                is_boot: mount == "/",
            })
            .collect();
    }
    let disks = Disks::new_with_refreshed_list();
    let mut out: Vec<Volume> = Vec::new();
    for d in disks.list() {
        let mount = d.mount_point().to_string_lossy().into_owned();
        let fs = d.file_system().to_string_lossy().into_owned();
        if PSEUDO_FS.contains(&fs.as_str()) || d.total_space() == 0 {
            continue;
        }
        if cfg!(target_os = "macos") && mount.starts_with("/System/Volumes/") {
            continue; // parts of the startup disk (Data, VM, Preboot…)
        }
        if cfg!(target_os = "linux")
            && (mount.starts_with("/snap/") || mount.starts_with("/boot") || mount.starts_with("/run/") || mount.starts_with("/var/lib/docker"))
        {
            continue;
        }
        if out.iter().any(|v| v.mount == mount) {
            continue;
        }
        let is_boot = mount == "/" || mount.eq_ignore_ascii_case("C:\\");
        let mut name = d.name().to_string_lossy().into_owned();
        if name.is_empty() || name.starts_with("/dev/") {
            name = if is_boot {
                "System".to_string()
            } else {
                std::path::Path::new(&mount).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(mount.clone())
            };
        }
        if cfg!(windows) {
            name = if name.is_empty() { mount.clone() } else { format!("{name} ({})", mount.trim_end_matches('\\')) };
        }
        let (mut total, mut available) = (d.total_space(), d.available_space());
        // On macOS the real usage of the startup disk is on the APFS container.
        if let Some((t, a)) = crate::scan::platform::volume_space(d.mount_point()) {
            total = t;
            available = a.max(available.min(t));
        }
        out.push(Volume { name, mount, total, available, removable: d.is_removable(), fs, is_boot });
    }
    out.sort_by_key(|v| (!v.is_boot, v.removable, v.mount.clone()));
    out
}
