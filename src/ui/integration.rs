//! Hooks into the system: "Scan with SquirrelDisk" in the file manager's
//! menu for folders, and the `squirreldisk` command in the terminal.
//!
//! Everything here is per user (no admin rights, except for the command on
//! macOS when `/usr/local/bin` isn't writable) and best effort: failures
//! come back as a message for the Settings screen.

use crate::i18n::tr;
use std::path::{Path, PathBuf};

/// What to run: the app bundle on macOS, the AppImage file on Linux (the
/// executable itself lives on a temporary mount), else the executable.
fn target() -> Option<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(p) = crate::update::appimage_path() {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    Some(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// `…/SquirrelDisk.app` when running from a bundle.
#[cfg(target_os = "macos")]
fn app_bundle(exe: &Path) -> Option<&Path> {
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
}

/// Refused on macOS while the app runs from a temporary copy (opened from
/// the download or the disk image rather than Applications): whatever
/// points at it breaks as soon as it goes away.
fn check_location(t: &Path) -> Result<(), String> {
    if t.to_string_lossy().contains("/AppTranslocation/") || t.starts_with("/Volumes/") {
        return Err(tr("Move SquirrelDisk to Applications first").to_string());
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Folder menu

/// Adds or removes "Scan with SquirrelDisk" for folders. Called when the
/// setting changes, and at launch while it's on, so the entry follows the
/// app if it moves (and the language).
pub fn set_folder_menu(on: bool) -> Result<(), String> {
    let t = target().ok_or("can't find the app")?;
    if on {
        check_location(&t)?;
    }
    imp::set_folder_menu(on, &t, tr("Scan with SquirrelDisk")).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    const WORKFLOW: &str = "Scan with SquirrelDisk.workflow";

    /// A Quick Action (Finder: right click → Quick Actions) in
    /// `~/Library/Services`, written from the templates in
    /// `packaging/macos/quick-action`.
    pub fn set_folder_menu(on: bool, target: &Path, label: &str) -> std::io::Result<()> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or(std::io::ErrorKind::NotFound)?;
        let dir = home.join("Library/Services").join(WORKFLOW);
        if !on {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
                refresh_services();
            }
            return Ok(());
        }
        let quoted = shell_quote(&target.to_string_lossy());
        // a new instance per folder, like opening it from the terminal
        let command = match app_bundle(target) {
            Some(app) => format!("for f in \"$@\"; do open -n -a {} --args \"$f\"; done", shell_quote(&app.to_string_lossy())),
            None => format!("for f in \"$@\"; do {quoted} \"$f\" >/dev/null 2>&1 & done"),
        };
        let contents = dir.join("Contents");
        std::fs::create_dir_all(&contents)?;
        let info = include_str!("../../packaging/macos/quick-action/Info.plist").replace("{{LABEL}}", &xml_escape(label));
        let doc = include_str!("../../packaging/macos/quick-action/document.wflow").replace("{{COMMAND}}", &xml_escape(&command));
        std::fs::write(contents.join("Info.plist"), info)?;
        std::fs::write(contents.join("document.wflow"), doc)?;
        refresh_services();
        Ok(())
    }

    /// Makes the Services menu pick up the change now rather than at the
    /// next login.
    fn refresh_services() {
        std::thread::spawn(|| std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").status());
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ};

    /// Per-user verbs: on a folder, a folder's background and a drive. On
    /// Windows 11 they're under "Show more options" (the new menu only
    /// takes packaged apps). The installer removes them on uninstall.
    const KEYS: [(&str, &str); 3] = [
        (r"Software\Classes\Directory\shell\SquirrelDisk", "%1"),
        (r"Software\Classes\Directory\Background\shell\SquirrelDisk", "%V"),
        (r"Software\Classes\Drive\shell\SquirrelDisk", "%1"),
    ];

    pub fn set_folder_menu(on: bool, target: &Path, label: &str) -> std::io::Result<()> {
        let exe = target.to_string_lossy();
        for (key, arg) in KEYS {
            if on {
                set(key, None, label)?;
                set(key, Some("Icon"), &format!("\"{exe}\",0"))?;
                set(&format!(r"{key}\command"), None, &format!("\"{exe}\" \"{arg}\""))?;
            } else {
                // SAFETY: valid null-terminated string; a missing key is fine.
                unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr()) };
            }
        }
        Ok(())
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Sets a string value (the default one for `None`), creating the key.
    fn set(key: &str, name: Option<&str>, value: &str) -> std::io::Result<()> {
        let value = wide(value);
        let name = name.map(wide);
        // SAFETY: plain registry calls with valid null-terminated strings
        // and out-pointers; the key is closed before returning.
        unsafe {
            let mut h: HKEY = std::ptr::null_mut();
            let err = RegCreateKeyExW(HKEY_CURRENT_USER, wide(key).as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut());
            if err != 0 {
                return Err(std::io::Error::from_raw_os_error(err as i32));
            }
            let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
            let err = RegSetValueExW(h, name_ptr, 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32);
            RegCloseKey(h);
            if err != 0 {
                return Err(std::io::Error::from_raw_os_error(err as i32));
            }
        }
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    fn data_home() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }

    fn installed(program: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
    }

    /// Linux file managers each have their own way: "Open with" (any
    /// file manager following the desktop entry spec), plus a direct entry
    /// for Dolphin, Nemo and Nautilus (under Scripts) when installed.
    pub fn set_folder_menu(on: bool, target: &Path, label: &str) -> std::io::Result<()> {
        let data = data_home().ok_or(std::io::ErrorKind::NotFound)?;
        set_folder_menu_in(&data, on, target, label, installed)
    }

    /// The work of [`set_folder_menu`], in `data` (`~/.local/share`), with
    /// `installed` telling which file managers are there.
    pub fn set_folder_menu_in(data: &Path, on: bool, target: &Path, label: &str, installed: fn(&str) -> bool) -> std::io::Result<()> {
        let exec = format!("{} %f", desktop_quote(&target.to_string_lossy()));
        let open_with = format!("[Desktop Entry]\nType=Application\nName=SquirrelDisk\nComment={label}\nExec={exec}\nIcon=squirreldisk\nMimeType=inode/directory;\nNoDisplay=true\nTerminal=false\n");
        let dolphin = format!("[Desktop Entry]\nType=Service\nMimeType=inode/directory;\nActions=scan;\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\n\n[Desktop Action scan]\nName={label}\nIcon=squirreldisk\nExec={exec}\n");
        let nemo = format!("[Nemo Action]\nName={label}\nIcon-Name=squirreldisk\nExec={} %F\nSelection=s\nExtensions=dir;\n", desktop_quote(&target.to_string_lossy()));
        let script = format!("#!/bin/sh\n# {label}: the selected folder, or the one open\n[ $# -eq 0 ] && set -- \"$PWD\"\nexec {} \"$1\"\n", shell_quote(&target.to_string_lossy()));
        let files: [(PathBuf, String, bool); 5] = [
            (data.join("applications/squirreldisk-folder.desktop"), open_with, true),
            (data.join("kio/servicemenus/squirreldisk.desktop"), dolphin.clone(), installed("dolphin")),
            (data.join("kservices5/ServiceMenus/squirreldisk.desktop"), dolphin, installed("dolphin")),
            (data.join("nemo/actions/squirreldisk.nemo_action"), nemo, installed("nemo")),
            (data.join("nautilus/scripts").join(label), script, installed("nautilus")),
        ];
        for (path, contents, wanted) in files {
            if on && wanted {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&path, contents)?;
                // Dolphin (Plasma 6) and Nautilus only run executable entries
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            } else if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        if !on {
            // the Nautilus script is named after the label, in any language
            if let Ok(dir) = std::fs::read_dir(data.join("nautilus/scripts")) {
                for e in dir.flatten() {
                    if std::fs::read_to_string(e.path()).is_ok_and(|s| s.starts_with("#!/bin/sh\n# ") && s.contains(&*target.to_string_lossy())) {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
        let apps = data.join("applications");
        if !cfg!(test) {
            std::thread::spawn(move || std::process::Command::new("update-desktop-database").arg(apps).status());
        }
        Ok(())
    }

    /// Quoting for `Exec=` in desktop entries: the spec's quoting rule,
    /// then its string escapes (hence a backslash written four times).
    fn desktop_quote(s: &str) -> String {
        let mut q = String::from("\"");
        for c in s.chars() {
            match c {
                '\\' => q.push_str("\\\\\\\\"),
                '"' | '`' | '$' => {
                    q.push_str("\\\\");
                    q.push(c);
                }
                '%' => q.push_str("%%"),
                c => q.push(c),
            }
        }
        q.push('"');
        q
    }
}

#[cfg(unix)]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ----------------------------------------------------------------------------
// Command line

#[derive(Clone, Debug, PartialEq)]
pub enum Cli {
    /// `squirreldisk` runs this app, from this path.
    Installed(PathBuf),
    /// Can be installed at this path.
    Missing(PathBuf),
    /// Something else is there (another install, or a package manager's).
    Taken(PathBuf),
    /// Not from here: the reason, for the Settings screen.
    Unavailable(String),
}

/// Where `squirreldisk` goes: `/usr/local/bin` on macOS (on the default
/// PATH), `~/.local/bin` on Linux (on the PATH of most distributions).
/// Windows: the installer adds its folder to the PATH.
#[cfg(unix)]
fn cli_link() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from("/usr/local/bin/squirreldisk"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/bin/squirreldisk"))
    }
}

pub fn cli_status() -> Cli {
    #[cfg(windows)]
    {
        // installed by the MSI next to the app, with the folder on the PATH
        match std::env::current_exe() {
            Ok(exe) if exe.with_extension("com").is_file() => Cli::Installed(exe.with_extension("com")),
            _ => Cli::Unavailable(tr("Comes with the installer (the .msi)").to_string()),
        }
    }
    #[cfg(unix)]
    {
        let (Some(link), Some(t)) = (cli_link(), target()) else { return Cli::Unavailable("no home folder".into()) };
        let exe = cli_exe(&t);
        match std::fs::read_link(&link) {
            Ok(to) if to == exe => Cli::Installed(link),
            _ if link.symlink_metadata().is_ok() => Cli::Taken(link),
            _ => match check_location(&t) {
                Ok(()) => Cli::Missing(link),
                Err(e) => Cli::Unavailable(e),
            },
        }
    }
}

/// What the link points to: the executable (in the bundle on macOS, so
/// that the command runs the headless code directly), or the AppImage.
#[cfg(unix)]
fn cli_exe(target: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(app) = app_bundle(target) {
        return app.join("Contents/MacOS/squirreldisk");
    }
    target.to_path_buf()
}

/// Installs the command. On macOS, asks for an administrator password when
/// `/usr/local/bin` isn't writable, unless `quiet` (first launch), which
/// only does what needs no password. Blocks: call from a thread.
pub fn install_cli(quiet: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = quiet;
        Err(tr("Comes with the installer (the .msi)").to_string())
    }
    #[cfg(unix)]
    {
        let Cli::Missing(link) = cli_status() else { return Ok(()) };
        let exe = cli_exe(&target().ok_or("can't find the app")?);
        let direct = std::fs::create_dir_all(link.parent().unwrap()).and_then(|_| std::os::unix::fs::symlink(&exe, &link));
        match direct {
            Ok(()) => Ok(()),
            #[cfg(target_os = "macos")]
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && !quiet => {
                // the standard password prompt, with the paths as arguments
                // so nothing needs quoting in the script
                let script = "on run argv\ndo shell script \"mkdir -p /usr/local/bin && ln -s \" & quoted form of item 1 of argv & \" \" & quoted form of item 2 of argv with administrator privileges\nend run";
                let out = std::process::Command::new("osascript").arg("-e").arg(script).arg(&exe).arg(&link).output().map_err(|e| e.to_string())?;
                if out.status.success() {
                    Ok(())
                } else {
                    Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
                }
            }
            Err(e) => {
                let _ = quiet;
                Err(e.to_string())
            }
        }
    }
}

/// First launch: installs the command where no password is needed. Not
/// for development builds, which would leave links into `target/`.
pub fn install_cli_first_launch() {
    let dev = target().is_some_and(|t| t.components().any(|c| c.as_os_str() == "target"));
    if !dev && matches!(cli_status(), Cli::Missing(_)) {
        std::thread::spawn(|| install_cli(true));
    }
}

/// `~/.local/bin` isn't on every PATH: then the Settings screen says so.
pub fn cli_on_path(link: &Path) -> bool {
    let dir = link.parent().unwrap_or(link);
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir))
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::imp::set_folder_menu_in;
    use std::path::Path;

    #[test]
    fn linux_folder_menu_on_and_off() {
        let data = std::env::temp_dir().join(format!("sqd-menu-{}", std::process::id()));
        let target = Path::new("/opt/My Apps/SquirrelDisk.AppImage");
        set_folder_menu_in(&data, true, target, "Scan with SquirrelDisk", |p| p == "nautilus").unwrap();
        let entry = std::fs::read_to_string(data.join("applications/squirreldisk-folder.desktop")).unwrap();
        assert!(entry.contains("Exec=\"/opt/My Apps/SquirrelDisk.AppImage\" %f"), "{entry}");
        assert!(entry.contains("MimeType=inode/directory;"));
        assert!(data.join("nautilus/scripts/Scan with SquirrelDisk").is_file());
        assert!(!data.join("nemo").exists(), "Nemo isn't installed");
        set_folder_menu_in(&data, false, target, "Scan with SquirrelDisk", |_| true).unwrap();
        assert!(!data.join("applications/squirreldisk-folder.desktop").exists());
        assert!(!data.join("nautilus/scripts/Scan with SquirrelDisk").exists());
        let _ = std::fs::remove_dir_all(data);
    }
}
