use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyAccessStatus {
    pub platform: &'static str,
    pub has_access: bool,
    pub can_elevate: bool,
    pub label: &'static str,
}

#[tauri::command]
#[allow(clippy::needless_return)]
pub fn get_privacy_access_status() -> PrivacyAccessStatus {
    #[cfg(target_os = "macos")]
    {
        return PrivacyAccessStatus {
            platform: "macos",
            has_access: macos::has_full_disk_access(),
            can_elevate: false,
            label: "Full Disk Access",
        };
    }

    #[cfg(target_os = "windows")]
    {
        let elevated = windows_impl::is_elevated();
        return PrivacyAccessStatus {
            platform: "windows",
            has_access: elevated,
            can_elevate: !elevated,
            label: "Administrator",
        };
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PrivacyAccessStatus {
            platform: "linux",
            has_access: true,
            can_elevate: false,
            label: "Full Access",
        }
    }
}

#[tauri::command]
#[allow(clippy::needless_return)]
pub fn request_privacy_access(app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app_handle;
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::relaunch_as_admin()?;
        app_handle.exit(0);
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app_handle;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    pub fn has_full_disk_access() -> bool {
        let home = match std::env::var_os("HOME") {
            Some(value) => PathBuf::from(value),
            None => return false,
        };

        let probes = [
            home.join("Library/Application Support/com.apple.TCC/TCC.db"),
            home.join("Library/Safari/CloudTabs.db"),
            home.join("Library/Safari/Bookmarks.plist"),
            home.join("Library/Mail"),
        ];

        let mut saw_permission_denied = false;
        for path in probes.iter() {
            match std::fs::metadata(path) {
                Ok(_) => match std::fs::File::open(path) {
                    Ok(_) => return true,
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                        saw_permission_denied = true;
                    }
                    Err(_) => {
                        if path.is_dir() {
                            match std::fs::read_dir(path) {
                                Ok(_) => return true,
                                Err(error)
                                    if error.kind() == std::io::ErrorKind::PermissionDenied =>
                                {
                                    saw_permission_denied = true;
                                }
                                Err(_) => {}
                            }
                        }
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    saw_permission_denied = true;
                }
                Err(_) => {}
            }
        }

        !saw_permission_denied
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    pub fn is_elevated() -> bool {
        use std::mem;
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token: HANDLE = 0;
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }

            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut size: u32 = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut core::ffi::c_void,
                mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            );
            CloseHandle(token);

            ok != 0 && elevation.TokenIsElevated != 0
        }
    }

    pub fn relaunch_as_admin() -> Result<(), String> {
        use std::iter::once;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;

        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let exe_wide: Vec<u16> = exe.as_os_str().encode_wide().chain(once(0)).collect();
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();

        let result = unsafe {
            ShellExecuteW(
                0,
                verb.as_ptr(),
                exe_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1, // SW_NORMAL
            )
        };

        if (result as isize) <= 32 {
            return Err(format!(
                "Elevation declined or failed (ShellExecute returned {})",
                result as isize
            ));
        }

        Ok(())
    }
}
