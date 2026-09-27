//! Guard rails against deleting things that would break the system.
//!
//! Three verdicts:
//! * `Forbidden` – never deleted, whatever the user says.
//! * `Caution`   – allowed only after an explicit extra confirmation.
//! * `Ok`.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Caution(String),
    Forbidden(String),
}

impl Verdict {
    pub fn is_forbidden(&self) -> bool {
        matches!(self, Verdict::Forbidden(_))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Mac,
    Linux,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Normalises a path for comparisons: forward slashes, no trailing slash,
/// lower-case on Windows.
fn norm(p: &str, os: Os) -> String {
    let mut s = p.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') && !(os == Os::Windows && s.len() == 3) {
        s.pop();
    }
    if os == Os::Windows {
        s = s.to_lowercase();
        if s.len() == 2 && s.ends_with(':') {
            s.push('/');
        }
    }
    s
}

fn is_under(path: &str, base: &str) -> bool {
    path == base || (path.starts_with(base) && (base.ends_with('/') || path[base.len()..].starts_with('/')))
}

/// `path` is `base` itself or one of its ancestors.
fn contains(path: &str, base: &str) -> bool {
    is_under(base, path)
}

pub struct Rules {
    /// Deleting these, anything inside them, or any ancestor is forbidden.
    forbidden_trees: Vec<String>,
    /// Deleting these or an ancestor is forbidden; their *content* is fine (with caution).
    forbidden_exact: Vec<String>,
    /// Content of these requires caution.
    caution_trees: Vec<String>,
    os: Os,
}

impl Rules {
    pub fn for_os(os: Os, home: Option<&str>) -> Rules {
        let h = home.map(|h| norm(h, os));
        let mut r = Rules { forbidden_trees: vec![], forbidden_exact: vec![], caution_trees: vec![], os };
        let t = |v: &mut Vec<String>, items: &[&str]| v.extend(items.iter().map(|s| norm(s, os)));
        match os {
            Os::Mac | Os::Linux => {
                t(
                    &mut r.forbidden_trees,
                    &[
                        "/System", "/bin", "/sbin", "/etc", "/boot", "/dev", "/proc", "/sys", "/lib", "/lib32", "/lib64",
                        "/libx32", "/private/etc", "/private/var/db", "/private/var/vm", "/private/var/root",
                        "/usr/bin", "/usr/sbin", "/usr/lib", "/usr/libexec", "/usr/share", "/usr/include",
                        "/Library/Apple", "/var/lib", "/var/db", "/var/vm", "/run", "/srv", "/root", "/efi",
                        "/Library/Keychains", "/Library/Preferences/SystemConfiguration",
                    ],
                );
                t(
                    &mut r.forbidden_exact,
                    &[
                        "/", "/usr", "/usr/local", "/var", "/private", "/private/var", "/private/tmp", "/tmp",
                        "/Applications", "/Library", "/Users", "/home", "/opt", "/Volumes", "/mnt", "/media",
                        "/cores", "/snap", "/var/log", "/var/cache", "/nix",
                    ],
                );
                t(
                    &mut r.caution_trees,
                    &["/Applications", "/Library", "/usr/local", "/opt", "/var", "/private", "/snap", "/nix"],
                );
                if let Some(h) = &h {
                    r.forbidden_exact.push(h.clone());
                    for sub in ["Library", "Documents", "Desktop", "Pictures", "Music", "Movies", "Downloads", ".config", ".local", "Applications"] {
                        r.forbidden_exact.push(format!("{h}/{sub}"));
                    }
                    for sub in [".ssh", ".gnupg", "Library/Keychains", ".password-store", "Library/Mobile Documents"] {
                        r.forbidden_trees.push(format!("{h}/{sub}"));
                    }
                    for sub in ["Library", ".config", ".local/share"] {
                        r.caution_trees.push(format!("{h}/{sub}"));
                    }
                }
            }
            Os::Windows => {
                t(
                    &mut r.forbidden_trees,
                    &[
                        "C:/Windows", "C:/Program Files/WindowsApps", "C:/System Volume Information", "C:/Recovery",
                        "C:/Boot", "C:/pagefile.sys", "C:/hiberfil.sys", "C:/swapfile.sys", "C:/bootmgr",
                        "C:/$Recycle.Bin", "C:/ProgramData/Microsoft",
                    ],
                );
                t(
                    &mut r.forbidden_exact,
                    &["C:/", "C:/Program Files", "C:/Program Files (x86)", "C:/ProgramData", "C:/Users", "C:/Users/Public"],
                );
                t(&mut r.caution_trees, &["C:/Program Files", "C:/Program Files (x86)", "C:/ProgramData"]);
                if let Some(h) = &h {
                    r.forbidden_exact.push(h.clone());
                    for sub in ["AppData", "AppData/Local", "AppData/Roaming", "AppData/LocalLow", "Documents", "Desktop", "Downloads", "Pictures"] {
                        r.forbidden_exact.push(format!("{h}/{sub}"));
                    }
                    r.forbidden_trees.push(format!("{h}/ntuser.dat"));
                    r.caution_trees.push(format!("{h}/AppData"));
                }
            }
        }
        r
    }

    pub fn current() -> Rules {
        let home = crate::settings::home_dir().map(|h| h.to_string_lossy().into_owned());
        let mut r = Rules::for_os(Os::current(), home.as_deref());
        // Never delete ourselves.
        if let Ok(exe) = std::env::current_exe() {
            r.forbidden_trees.push(norm(&exe.to_string_lossy(), r.os));
        }
        r
    }

    pub fn check(&self, path: &str) -> Verdict {
        let p = norm(path, self.os);
        if p.is_empty() {
            return Verdict::Forbidden("empty path".into());
        }
        // Volume roots: "/", "C:/", "/Volumes/X", "/mnt/x", "/media/u/x".
        let depth = p.trim_matches('/').split('/').filter(|s| !s.is_empty()).count();
        if depth == 0 || (self.os == Os::Windows && p.len() <= 3) {
            return Verdict::Forbidden("this is the root of a disk".into());
        }
        if self.os != Os::Windows {
            if (p.starts_with("/Volumes/") && depth == 2) || (p.starts_with("/mnt/") && depth == 2) || (p.starts_with("/media/") && depth <= 3) {
                return Verdict::Forbidden("this is a mounted volume".into());
            }
        }
        for f in &self.forbidden_trees {
            if is_under(&p, f) || contains(&p, f) {
                return Verdict::Forbidden(format!("protected system location ({f})"));
            }
        }
        for f in &self.forbidden_exact {
            if contains(&p, f) {
                return Verdict::Forbidden(format!("essential folder ({f})"));
            }
        }
        for c in &self.caution_trees {
            if is_under(&p, c) {
                return Verdict::Caution(format!("inside {c}: apps or system components may stop working"));
            }
        }
        let name = p.rsplit('/').next().unwrap_or("");
        if name.starts_with('.') && depth <= 3 {
            return Verdict::Caution("hidden configuration folder".into());
        }
        Verdict::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_rules() {
        let r = Rules::for_os(Os::Mac, Some("/Users/me"));
        assert!(r.check("/").is_forbidden());
        assert!(r.check("/System/Library/x").is_forbidden());
        assert!(r.check("/Users").is_forbidden());
        assert!(r.check("/Users/me").is_forbidden());
        assert!(r.check("/Users/me/").is_forbidden());
        assert!(r.check("/Users/me/.ssh/id_rsa").is_forbidden());
        assert!(r.check("/private").is_forbidden()); // ancestor of /private/etc
        assert!(r.check("/Volumes/Backup").is_forbidden());
        assert!(r.check("/usr").is_forbidden());
        assert!(matches!(r.check("/Applications/Foo.app"), Verdict::Caution(_)));
        assert!(matches!(r.check("/Users/me/Library/Caches/foo"), Verdict::Caution(_)));
        assert_eq!(r.check("/Users/me/Downloads/big.iso"), Verdict::Ok);
        assert_eq!(r.check("/Users/me/projects/node_modules"), Verdict::Ok);
        assert_eq!(r.check("/Users/me/Downloads/big.iso"), Verdict::Ok);
        assert!(r.check("/Users/me/Downloads").is_forbidden());
    }

    #[test]
    fn windows_rules() {
        let r = Rules::for_os(Os::Windows, Some("C:\\Users\\Me"));
        assert!(r.check("C:\\").is_forbidden());
        assert!(r.check("c:\\windows\\system32").is_forbidden());
        assert!(r.check("C:\\Users\\Me").is_forbidden());
        assert!(r.check("C:\\pagefile.sys").is_forbidden());
        assert!(matches!(r.check("C:\\Program Files\\Foo"), Verdict::Caution(_)));
        assert_eq!(r.check("C:\\Users\\Me\\Videos\\x.mp4"), Verdict::Ok);
        assert_eq!(r.check("D:\\stuff"), Verdict::Ok);
    }
}
