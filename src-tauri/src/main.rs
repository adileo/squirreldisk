#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]
#![allow(unexpected_cfgs)]
mod privacy;
mod scan;
mod snapshots;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod window_style;

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use sysinfo::{DiskExt, System, SystemExt};
use tauri::Manager;
use tauri_plugin_shell::process::CommandChild;

#[cfg(target_os = "macos")]
use window_vibrancy::NSVisualEffectMaterial;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SquirrelDisk {
    name: String,
    s_mount_point: String,
    total_space: u64,
    available_space: u64,
    is_removable: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteOutcome {
    deleted_bytes: u64,
}

fn main() {
    if let Err(error) = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_hashtree_updater::init())
        .manage(MyState(Default::default()))
        .setup(|app| {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            let window = app.get_webview_window("main");
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let _ = app;
            // window.open_devtools();
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            if let Some(window) = window {
                #[cfg(target_os = "macos")]
                if let Err(error) = window_vibrancy::apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::HudWindow,
                    None,
                    None,
                ) {
                    eprintln!("Error applying blurred bg: {error}");
                }

                #[cfg(target_os = "windows")]
                if let Err(error) = window_vibrancy::apply_blur(&window, Some((18, 18, 18, 125))) {
                    eprintln!("Error applying blurred bg: {error}");
                }

                if let Err(error) = window_style::set_window_styles(&window) {
                    eprintln!("Error applying window styles: {error}");
                }
            }

            // app.listen_global("scan_stop", |event| {
            //     let s = app.state::<MyState>();
            //     s.0.lock().unwrap().take().unwrap().kill();
            // });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_disks,
            start_scanning,
            stop_scanning,
            show_in_folder,
            open_terminal,
            delete_permanently,
            open_full_disk_access_settings,
            privacy::get_privacy_access_status,
            privacy::request_privacy_access,
            snapshots::get_scan_snapshot,
            snapshots::list_scan_snapshots,
            snapshots::save_scan_snapshot,
            snapshots::delete_scan_snapshot
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("error while running tauri application: {error}");
    }
}

#[tauri::command]
fn open_full_disk_access_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[tauri::command]
async fn delete_permanently(path: String) -> Result<DeleteOutcome, String> {
    if path.trim().is_empty() {
        return Err("Path is empty".to_string());
    }

    tauri::async_runtime::spawn_blocking(move || delete_permanently_at_path(&PathBuf::from(path)))
        .await
        .map_err(|error| format!("Failed to join deletion task: {error}"))?
}

fn delete_permanently_at_path(path: &Path) -> Result<DeleteOutcome, String> {
    if let Some(reason) = deletion_protection_reason(path) {
        return Err(reason);
    }

    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let deleted_bytes = measured_delete_bytes(path).unwrap_or(0);

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())?;
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }

    Ok(DeleteOutcome { deleted_bytes })
}

fn deletion_protection_reason(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return Some("Only absolute scanned paths can be deleted".to_string());
    }

    if path.parent().is_none() {
        return Some("Refusing to delete a filesystem root".to_string());
    }

    let normalized = normalized_path_text(path);
    let protected_exact = [
        "/",
        "/Applications",
        "/Library",
        "/System",
        "/Users",
        "/Volumes",
        "/bin",
        "/dev",
        "/etc",
        "/private",
        "/sbin",
        "/usr",
        "/var",
    ];

    if protected_exact.contains(&normalized.as_str()) {
        return Some(format!("Refusing to delete protected path {normalized}"));
    }

    if std::env::var("HOME")
        .ok()
        .map(|home| normalized == normalized_path_text(Path::new(&home)))
        .unwrap_or(false)
    {
        return Some("Refusing to delete the home directory itself".to_string());
    }

    None
}

fn normalized_path_text(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let trimmed = text.trim_end_matches('/');

    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn measured_delete_bytes(path: &Path) -> std::io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let mut total = 0;
        for entry in fs::read_dir(path)? {
            total += measured_delete_bytes(&entry?.path()).unwrap_or(0);
        }
        Ok(total)
    } else {
        Ok(metadata.len())
    }
}

#[tauri::command]
fn show_in_folder(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Path is empty".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        let result = path.replace('/', "\\");
        Command::new("explorer")
            .args(["/select,", format!("{}", result).as_str()]) // The comma after select is not a typo
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        // if path.contains(",") {
        // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
        let new_path = if fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            PathBuf::from(&path)
        } else {
            PathBuf::from(&path)
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| "Path has no parent directory".to_string())?
        };
        Command::new("xdg-open")
            .arg(&new_path)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
        // } else {
        //     Command::new("dbus-send")
        //         .args([
        //             "--session",
        //             "--dest=org.freedesktop.FileManager1",
        //             "--type=method_call",
        //             "/org/freedesktop/FileManager1",
        //             "org.freedesktop.FileManager1.ShowItems",
        //             format!("array:string:\"file://{path}\"").as_str(),
        //             "string:\"\"",
        //         ])
        //         .spawn()
        //         .unwrap();
        // }
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err("Showing files in a folder is not supported on this platform".to_string())
    }
}

#[tauri::command]
fn open_terminal(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Path is empty".to_string());
    }

    let dir = terminal_directory(&PathBuf::from(path));

    #[cfg(target_os = "macos")]
    {
        let dir_text = dir.to_string_lossy().to_string();
        Command::new("open")
            .args(["-a", "Terminal", dir_text.as_str()])
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let dir_text = dir.to_string_lossy().to_string();
        let command = format!(
            "Set-Location -LiteralPath {}",
            powershell_literal(&dir_text)
        );
        Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .arg("powershell.exe")
            .arg("-NoExit")
            .arg("-Command")
            .arg(command)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        return open_linux_terminal(&dir);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err("Opening a terminal is not supported on this platform".to_string())
    }
}

fn terminal_directory(path: &Path) -> PathBuf {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => path.to_path_buf(),
        _ => path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

#[cfg(target_os = "windows")]
fn powershell_literal(path: &str) -> String {
    format!("'{}'", path.replace('\'', "''"))
}

#[cfg(target_os = "linux")]
fn open_linux_terminal(dir: &Path) -> Result<(), String> {
    let terminals = [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "alacritty",
    ];
    let mut errors = Vec::new();

    for terminal in terminals {
        match Command::new(terminal).current_dir(dir).spawn() {
            Ok(_) => return Ok(()),
            Err(error) => errors.push(format!("{terminal}: {error}")),
        }
    }

    Err(format!("Could not open terminal: {}", errors.join("; ")))
}
// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn get_disks() -> Result<String, String> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let mut vec: Vec<SquirrelDisk> = Vec::new();

    for disk in sys.disks() {
        vec.push(SquirrelDisk {
            name: disk.name().to_string_lossy().to_string(),
            s_mount_point: disk.mount_point().display().to_string(),
            total_space: disk.total_space(),
            available_space: disk.available_space(),
            is_removable: disk.is_removable(),
        });
    }
    serde_json::to_string(&vec).map_err(|error| error.to_string())
}

pub struct MyState(Mutex<Option<CommandChild>>);

#[tauri::command]
fn start_scanning(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MyState>,
    path: String,
    ratio: String,
) -> Result<(), String> {
    scan::start(app_handle, state, path, ratio)
}

#[tauri::command]
fn stop_scanning(
    _app_handle: tauri::AppHandle,
    state: tauri::State<'_, MyState>,
    _path: String,
) -> Result<(), ()> {
    scan::stop(state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_delete_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "squirreldisk-delete-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn deletion_protection_rejects_roots_and_major_system_dirs() {
        let temp_dir = std::env::temp_dir();
        let root = temp_dir.ancestors().last().unwrap();

        assert!(deletion_protection_reason(root).is_some());
        assert!(deletion_protection_reason(&temp_dir.join("squirreldisk-file")).is_none());

        #[cfg(not(target_os = "windows"))]
        {
            assert!(deletion_protection_reason(Path::new("/System")).is_some());
            assert!(deletion_protection_reason(Path::new("/Users")).is_some());
        }
    }

    #[test]
    fn delete_permanently_removes_files_and_reports_bytes() {
        let dir = temp_delete_dir("file");
        let file_path = dir.join("large.tmp");
        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(&[7_u8; 4096]).unwrap();
        drop(file);

        let outcome = delete_permanently_at_path(&file_path).unwrap();

        assert!(!file_path.exists());
        assert_eq!(outcome.deleted_bytes, 4096);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn delete_permanently_removes_directories_recursively() {
        let dir = temp_delete_dir("directory");
        let target = dir.join("collected");
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::write(target.join("a.bin"), &[1_u8; 200]).unwrap();
        fs::write(target.join("nested/b.bin"), &[2_u8; 300]).unwrap();

        let outcome = delete_permanently_at_path(&target).unwrap();

        assert!(!target.exists());
        assert_eq!(outcome.deleted_bytes, 500);
        fs::remove_dir_all(dir).unwrap();
    }
}
