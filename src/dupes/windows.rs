//! Windows duplicate cleanup only: exclude OS/application data before listing,
//! and re-check the resolved target immediately before recycling it.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

const DIRS: &[&str] = &[
    "windows", "windows.old", "system32", "syswow64", "winsxs", "program files",
    "program files (x86)", "programdata", "appdata", "windowsapps", "recovery",
    "boot", "efi", "system volume information", "$recycle.bin", "$windows.~bt",
    "$windows.~ws", "$winreagent", "config.msi", "msocache",
];
const EXTENSIONS: &[&str] = &[
    "dll", "sys", "drv", "ocx", "cpl", "com", "scr", "efi", "mui",
    "winmd", "tlb", "olb", "ax", "acm", "vxd", "pyd", "lib", "msp",
    "msix", "msixbundle", "appx", "appxbundle", "cat", "inf", "manifest",
];

fn normalized(path: &Path) -> String {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {},
            Component::ParentDir => { clean.pop(); },
            _ => clean.push(part.as_os_str()),
        }
    }
    let s = clean.to_string_lossy().replace('\\', "/").to_lowercase();
    let s = s.strip_prefix("//?/").unwrap_or(&s);
    s.trim_end_matches('/').to_string()
}

struct Protection {
    roots: Vec<String>,
}

impl Protection {
    fn current() -> Self {
        let mut roots = Vec::new();
        for key in ["SystemRoot", "WINDIR", "ProgramFiles", "ProgramFiles(x86)",
            "ProgramW6432", "ProgramData", "ALLUSERSPROFILE", "LOCALAPPDATA", "APPDATA"] {
            if let Some(p) = std::env::var_os(key).filter(|p| !p.is_empty()) {
                let p = PathBuf::from(p);
                roots.push(normalized(&p));
                if let Ok(real) = p.canonicalize() { roots.push(normalized(&real)); }
            }
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            roots.push(normalized(&PathBuf::from(home).join("AppData")));
        }
        // Allow downloaded installers, but never offer this running program.
        if let Ok(exe) = std::env::current_exe() {
            roots.push(normalized(&exe));
        }
        Self { roots }
    }

    fn excludes(&self, path: &Path) -> bool {
        let p = normalized(path);
        if p.split('/').any(|part| DIRS.contains(&part.trim_end_matches([' ', '.']))) {
            return true;
        }
        if self.roots.iter().any(|root| p == *root || p.strip_prefix(root).is_some_and(|s| s.starts_with('/'))) {
            return true;
        }
        let name = p.rsplit('/').next().unwrap_or("").trim_end_matches([' ', '.']);
        if matches!(name, "pagefile.sys" | "hiberfil.sys" | "swapfile.sys" | "bootmgr" | "ntldr") || name.starts_with("ntuser.dat") {
            return true;
        }
        name.rsplit_once('.').is_some_and(|(_, ext)| EXTENSIONS.contains(&ext))
    }
}

pub fn protected(path: &Path) -> bool {
    static POLICY: OnceLock<Protection> = OnceLock::new();
    POLICY.get_or_init(Protection::current).excludes(path)
}

pub fn special(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // Reparse points include junctions and symlinks. Never enter them, even
    // when the directory bit is also set. SYSTEM files are never candidates.
    md.file_attributes() & (0x400 | 0x4) != 0
}

pub fn allowed_root(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    !protected(path)
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && root_attributes_allowed(path, m.file_attributes()))
        && path.canonicalize().is_ok_and(|p| !protected(&p))
}

fn root_attributes_allowed(path: &Path, attributes: u32) -> bool {
    let p = normalized(path);
    let drive_root = path.is_absolute() && p.len() == 2 && p.as_bytes()[0].is_ascii_alphabetic() && p.as_bytes()[1] == b':';
    // Windows marks drive roots (including C:\) as Hidden + System. Allow
    // listing that root, while keeping SYSTEM children and all reparse points
    // excluded. This exception is never used by the removal checks.
    attributes & 0x400 == 0 && (attributes & 0x4 == 0 || drive_root)
}

pub fn check_removal(path: &Path) -> Result<PathBuf, String> {
    let md = path.symlink_metadata().map_err(|e| e.to_string())?;
    let real = path.canonicalize().map_err(|e| e.to_string())?;
    if protected(path) || protected(&real) || special(&md) {
        return Err("protected Windows location or application component; left alone".into());
    }
    if !md.is_file() { return Err("not a regular file; left alone".into()); }
    Ok(real)
}

/// Explicit recycle-only operation. Never falls back to remove_file.
pub fn recycle(path: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName,
        FOF_NO_UI, FOF_WANTNUKEWARNING, FOFX_RECYCLEONDELETE, FOFX_EARLYFAILURE};
    let real = check_removal(Path::new(path))?;
    let text = real.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    // Network shares cannot promise a local Recycle Bin.
    if text.starts_with("UNC\\") || text.starts_with(r"\\") {
        return Err("Recycle Bin is unavailable for this network location; left alone".into());
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect();
    struct Com;
    impl Drop for Com { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
    let result = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(|e| e.to_string())?;
        let _com = Com;
        (|| -> windows::core::Result<()> {
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(FOF_NO_UI | FOF_WANTNUKEWARNING | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE)?;
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
            op.DeleteItem(&item, None)?;
            op.PerformOperations()?;
            if op.GetAnyOperationsAborted()?.as_bool() {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(0x80004004u32 as i32)));
            }
            Ok(())
        })()
    };
    result.map_err(|e| format!("Could not move to Recycle Bin; no permanent-delete fallback: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_drive_root_is_scannable_but_system_children_are_not() {
        const SYSTEM_DIRECTORY: u32 = 0x16; // hidden, system, directory
        for p in [r"C:\", r"D:\", r"\\?\C:\"] {
            assert!(root_attributes_allowed(Path::new(p), SYSTEM_DIRECTORY), "{p}");
            assert!(!root_attributes_allowed(Path::new(p), SYSTEM_DIRECTORY | 0x400), "root junctions stay excluded");
        }
        for p in [r"C:", r"C:\Windows", r"C:\Windows\System32", r"C:\Users\Me\hidden-system-folder"] {
            assert!(!root_attributes_allowed(Path::new(p), SYSTEM_DIRECTORY), "{p}");
        }
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let root = PathBuf::from(format!("{drive}\\"));
        assert!(allowed_root(&root), "the actual Windows system drive must remain scannable: {root:?}");
        assert!(protected(&root.join("Windows/System32")));
        assert!(!allowed_root(&root.join("Windows")));
    }

    #[test]
    fn protected_paths_and_components_on_any_drive() {
        let rules = Protection { roots: vec!["e:/custom-os".into()] };
        for p in [r"C:\Windows\System32\x.dat", r"D:\WINDOWS\WinSxS\x.txt",
            r"\\?\D:\Program Files\App\data.dat", r"C:\Users\Me\AppData\Local\App\data.json",
            r"D:\System Volume Information\x", r"E:\custom-os\x.txt", r"E:\custom-os\..\custom-os\x.txt",
            r"D:\photos\library.DLL", r"C:\Windows\setup.EXE", r"D:\Program Files\App\setup.msi", r"C:\Users\Me\NTUSER.DAT.LOG1"] {
            assert!(rules.excludes(Path::new(p)), "{p}");
        }
        for p in [r"D:\photos\image.jpg", r"C:\Users\Me\Documents\report.pdf",
            r"E:\custom-os-backup\notes.txt", r"D:\windows-notes\notes.txt", r"D:\photos\dll-notes.txt",
            r"C:\Users\Me\Downloads\setup.EXE", r"D:\downloads\setup.MSI"] {
            assert!(!rules.excludes(Path::new(p)), "{p}");
        }
        assert!(protected(&std::env::current_exe().unwrap()));
    }

    #[test]
    #[ignore = "moves and restores only a disposable test file via the Windows Recycle Bin"]
    fn recycle_round_trip() {
        let dir = super::super::test_dir(&format!("sqd-recycle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for extension in ["txt", "exe", "msi"] {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = dir.join(format!("sqd-recycle-roundtrip-{stamp}-{extension}.{extension}"));
        std::fs::write(&path, b"disposable round-trip fixture").unwrap();
        recycle(&path.to_string_lossy()).unwrap();
        assert!(!path.exists());
        // Shell display names can omit known extensions, and the namespace
        // can lag behind PerformOperations. Match our unique name and parent.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut item = loop {
            let items = trash::os_limited::list().unwrap();
            let found = items.iter().position(|item| {
                item.name.to_string_lossy().starts_with(&*path.file_stem().unwrap().to_string_lossy())
                    && normalized(&item.original_parent) == normalized(&dir)
            });
            if let Some(i) = found { break items.into_iter().nth(i).unwrap(); }
            assert!(std::time::Instant::now() < deadline, "file must actually be in the Recycle Bin; observed: {items:?}");
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        item.name = path.file_name().unwrap().to_os_string();
        trash::os_limited::restore_all([item]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"disposable round-trip fixture");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
