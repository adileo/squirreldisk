use std::fs;
use std::path::Path;

use regex::{Captures, Regex};

use tauri::Emitter;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use crate::MyState;

const ROOT_SCAN_BANNED_PATHS: [&str; 7] = [
    "/dev", "/mnt", "/cdrom", "/proc", "/media", "/Volumes", "/System",
];

#[cfg(target_os = "macos")]
const MACOS_CLOUD_LIBRARY_DIRS: [&str; 2] = ["Mobile Documents", "CloudStorage"];

fn push_path_arg(path: &Path, paths: &mut Vec<String>) {
    paths.push(path.display().to_string());
}

#[cfg(target_os = "macos")]
fn is_macos_cloud_library_entry(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .map(|name| MACOS_CLOUD_LIBRARY_DIRS.contains(&name))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn add_library_children_skipping_cloud_dirs(library_dir: &Path, paths: &mut Vec<String>) -> bool {
    let initial_len = paths.len();
    let entries = match fs::read_dir(library_dir) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    for entry in entries.flatten() {
        if !is_macos_cloud_library_entry(&entry.file_name()) {
            push_path_arg(&entry.path(), paths);
        }
    }

    paths.len() > initial_len
}

#[cfg(target_os = "macos")]
fn add_home_children_skipping_cloud_dirs(home_dir: &Path, paths: &mut Vec<String>) -> bool {
    let initial_len = paths.len();
    let entries = match fs::read_dir(home_dir) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let name = entry.file_name();

        if name == "Library" && entry_path.is_dir() {
            add_library_children_skipping_cloud_dirs(&entry_path, paths);
        } else {
            push_path_arg(&entry_path, paths);
        }
    }

    paths.len() > initial_len
}

#[cfg(target_os = "macos")]
fn add_user_homes_skipping_cloud_dirs(users_dir: &Path, paths: &mut Vec<String>) -> bool {
    let initial_len = paths.len();
    let users = match fs::read_dir(users_dir) {
        Ok(users) => users,
        Err(_) => return false,
    };

    for user_entry in users.flatten() {
        let user_path = user_entry.path();
        if user_path.is_dir() && !add_home_children_skipping_cloud_dirs(&user_path, paths) {
            push_path_arg(&user_path, paths);
        }
    }

    paths.len() > initial_len
}

#[cfg(target_os = "macos")]
fn add_macos_targets_skipping_cloud_dirs(path: &Path, paths: &mut Vec<String>) -> bool {
    if path == Path::new("/Users") {
        return add_user_homes_skipping_cloud_dirs(path, paths);
    }

    if path.parent() == Some(Path::new("/Users")) {
        return add_home_children_skipping_cloud_dirs(path, paths);
    }

    if path.file_name().map_or(false, |name| name == "Library") {
        return add_library_children_skipping_cloud_dirs(path, paths);
    }

    false
}

#[derive(Clone, serde::Serialize)]
struct Payload {
    items: u64,
    total: u64,
    errors: u64,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RestrictedPathPayload {
    path: String,
    operation: String,
    message: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanFailurePayload {
    path: String,
    message: String,
}

fn progress_regex() -> Regex {
    Regex::new(r"\(scanned ([0-9]+), total ([0-9]+)(?:, linked [0-9]+, shared [0-9]+)?(?:, erred ([0-9]+))?\)").unwrap()
}

fn restricted_path_regex() -> Regex {
    Regex::new(r#"\[error\]\s+([^\s]+)\s+"([^"]+)":\s*(.+)"#).unwrap()
}

fn payload_from_progress_captures(groups: Captures) -> Payload {
    Payload {
        items: groups
            .get(1)
            .map_or("0", |m| m.as_str())
            .trim_end()
            .parse::<u64>()
            .unwrap_or(0),
        total: groups
            .get(2)
            .map_or("0", |m| m.as_str())
            .trim_end()
            .parse::<u64>()
            .unwrap_or(0),
        errors: groups
            .get(3)
            .map_or("0", |m| m.as_str())
            .trim_end()
            .parse::<u64>()
            .unwrap_or(0),
    }
}

fn restricted_path_from_captures(groups: Captures) -> RestrictedPathPayload {
    RestrictedPathPayload {
        operation: groups.get(1).map_or("", |m| m.as_str()).to_string(),
        path: groups.get(2).map_or("", |m| m.as_str()).to_string(),
        message: groups.get(3).map_or("", |m| m.as_str()).to_string(),
    }
}

fn add_shared_scan_args(paths: &mut Vec<String>, include_shared_scan_args: bool) {
    if include_shared_scan_args {
        paths.push("--deduplicate-hardlinks".to_string());
        paths.push("--omit-json-shared-details".to_string());
        paths.push("--omit-json-shared-summary".to_string());
    }
}

fn initial_scan_args_for_platform(ratio: &str, include_shared_scan_args: bool) -> Vec<String> {
    let mut paths = vec!["--json-output".to_string(), "--progress".to_string()];
    add_shared_scan_args(&mut paths, include_shared_scan_args);
    paths.push("--threads=max".to_string());
    paths.push(["--min-ratio=", ratio].join(""));
    paths
}

fn initial_scan_args(ratio: &str) -> Vec<String> {
    initial_scan_args_for_platform(ratio, !cfg!(target_os = "windows"))
}

// Start scan
pub fn start(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MyState>,
    path: String,
    ratio: String,
) -> Result<(), String> {
    println!("Start Scanning {}", path);
    let scan_root = path.clone();
    let mut paths_to_scan = initial_scan_args(&ratio);

    if path.eq("/") {
        let paths = fs::read_dir("/").map_err(|error| error.to_string())?;

        for scan_path in paths {
            let scan_path_str = match scan_path {
                Ok(scan_path) => scan_path.path(),
                Err(_) => continue,
            };
            let path_str = scan_path_str.to_string_lossy();
            if ROOT_SCAN_BANNED_PATHS.contains(&path_str.as_ref()) {
                continue;
            }

            #[cfg(target_os = "macos")]
            {
                if path_str == "/Users" {
                    if !add_user_homes_skipping_cloud_dirs(Path::new("/Users"), &mut paths_to_scan)
                    {
                        push_path_arg(&scan_path_str, &mut paths_to_scan);
                    }
                    continue;
                }
            }

            push_path_arg(&scan_path_str, &mut paths_to_scan);
        }
    } else {
        #[cfg(target_os = "macos")]
        {
            if !add_macos_targets_skipping_cloud_dirs(Path::new(&path), &mut paths_to_scan) {
                paths_to_scan.push(path);
            }
        }

        #[cfg(not(target_os = "macos"))]
        paths_to_scan.push(path);
    }

    app_handle
        .emit(
            "scan_status",
            Payload {
                items: 0,
                total: 0,
                errors: 0,
            },
        )
        .ok();

    let pdu_command = app_handle.shell().sidecar("pdu").map_err(|error| {
        let message = format!("failed to create `pdu` sidecar command: {error}");
        emit_scan_failure(&app_handle, &scan_root, &message);
        message
    })?;
    let (mut rx, child) = pdu_command.args(paths_to_scan).spawn().map_err(|error| {
        let message = format!("Failed to spawn sidecar: {error}");
        emit_scan_failure(&app_handle, &scan_root, &message);
        message
    })?;

    match state.0.lock() {
        Ok(mut guard) => {
            *guard = Some(child);
        }
        Err(error) => {
            let message = format!("Failed to lock scan state: {error}");
            let _ = child.kill();
            emit_scan_failure(&app_handle, &scan_root, &message);
            return Err(message);
        }
    }

    // unlisten to the event using the `id` returned on the `listen_global` function
    // an `once_global` API is also exposed on the `App` struct

    let progress_re = progress_regex();
    let error_re = restricted_path_regex();

    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    //println!("Stdout:{}", &line);
                    let string = String::from_utf8_lossy(&line).to_string();
                    app_handle.emit("scan_completed", string).ok();
                }
                CommandEvent::Stderr(msg) => {
                    // println!("Stderr:{}", &msg);

                    let string = String::from_utf8_lossy(&msg).to_string();
                    for groups in progress_re.captures_iter(&string) {
                        if groups.len() > 2 {
                            emit_scan_status(&app_handle, groups)
                        }
                    }
                    for groups in error_re.captures_iter(&string) {
                        app_handle
                            .emit(
                                "scan_restricted_path",
                                restricted_path_from_captures(groups),
                            )
                            .ok();
                    }
                }
                CommandEvent::Terminated(t) => {
                    println!("{t:?}");
                    // app_handle.unlisten(id);
                    // child.kill();
                }
                _ => {}
            };
            // if let CommandEvent::Stdout(line) = event {
            //     println!("StdErr: {}", line);
            // } else {
            //     println!("Terminated {}", event);
            // }
            // if let CommandEvent::Stderr(line) = event {
            //     println!("StdErr: {}", line);
            // }
            // if let CommandEvent::Terminated(line) = event {
            //     println!("Terminated");
            // }
        }
        Result::<(), ()>::Ok(())
    });

    Ok(())
    // thread::spawn(move || {
    //     let path = PathBuf::from(path);
    //     let mut vec: Vec<PathBuf> = Vec::new();
    //     vec.push(path);

    //     fn progress_and_error_reporter<Data>(
    //         app_handle: tauri::AppHandle,
    //     ) -> ProgressAndErrorReporter<Data, fn(ErrorReport)>
    //     where
    //         Data: Size + Into<u64> + Send + Sync,
    //         ProgressReport<Data>: Default + 'static,
    //         u64: Into<Data>,
    //     {
    //         let progress_reporter = move |report: ProgressReport<Data>| {
    //             let ProgressReport {
    //                 items,
    //                 total,
    //                 errors,
    //             } = report;
    //             let mut text = String::new();
    //             write!(
    //                 text,
    //                 "\r(scanned {items}, total {total}",
    //                 items = items,
    //                 total = total.into(),
    //             )
    //             .unwrap();
    //             if errors != 0 {
    //                 write!(text, ", erred {}", errors).unwrap();
    //             }
    //             write!(text, ")").unwrap();
    //             println!("{}", text);
    //             app_handle
    //                 .emit_all(
    //                     "scan_status",
    //                     Payload {
    //                         items: items,
    //                         total: total.into(),
    //                         errors: errors,
    //                     },
    //                 )
    //                 .unwrap();
    //         };

    //         struct TextReport<'a>(ErrorReport<'a>);

    //         impl<'a> Display for TextReport<'a> {
    //             fn fmt(&self, formatter: &mut Formatter<'_>) -> Result<(), Error> {
    //                 write!(
    //                     formatter,
    //                     "[error] {operation} {path:?}: {error}",
    //                     operation = self.0.operation.name(),
    //                     path = self.0.path,
    //                     error = self.0.error,
    //                 )
    //             }
    //         }

    //         let error_reporter: fn(ErrorReport) = |report| {
    //             let message = TextReport(report).to_string();
    //             println!("{}", message);
    //         };

    //         ProgressAndErrorReporter::new(
    //             progress_reporter,
    //             Duration::from_millis(100),
    //             error_reporter,
    //         )
    //     }
    //     // pub struct MyReporter {}
    //     // impl parallel_disk_usage::reporter::progress_and_error_reporter
    //     let pdu = parallel_disk_usage::app::Sub {
    //         json_output: true,
    //         direction: Direction::BottomUp,
    //         bar_alignment: BarAlignment::Right,
    //         get_data: GET_APPARENT_SIZE,
    //         files: vec,
    //         no_sort: true,
    //         min_ratio: 0.01.try_into().unwrap(),
    //         max_depth: 10.try_into().unwrap(),
    //         reporter: progress_and_error_reporter(app_handle),
    //         bytes_format: BytesFormat::MetricUnits,
    //         column_width_distribution: ColumnWidthDistribution::total(100),
    //     }
    //     .run();
    // });
}

pub fn stop(state: tauri::State<'_, MyState>) {
    if let Ok(mut guard) = state.0.lock() {
        if let Some(child) = guard.take() {
            let _ = child.kill();
        }
    }
}

fn emit_scan_status(app_handle: &tauri::AppHandle, groups: Captures) {
    app_handle
        .emit("scan_status", payload_from_progress_captures(groups))
        .ok();
}

fn emit_scan_failure(app_handle: &tauri::AppHandle, path: &str, message: &str) {
    let payload = ScanFailurePayload {
        path: path.to_string(),
        message: message.to_string(),
    };
    app_handle.emit("scan_failed", payload).ok();
    app_handle
        .emit(
            "scan_restricted_path",
            RestrictedPathPayload {
                path: path.to_string(),
                operation: "scan".to_string(),
                message: message.to_string(),
            },
        )
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_regex_parses_plain_progress() {
        let regex = progress_regex();
        let groups = regex
            .captures("(scanned 12, total 345)")
            .expect("progress is parsed");
        let payload = payload_from_progress_captures(groups);

        assert_eq!(payload.items, 12);
        assert_eq!(payload.total, 345);
        assert_eq!(payload.errors, 0);
    }

    #[test]
    fn progress_regex_parses_pdu_shared_and_error_counts() {
        let regex = progress_regex();
        let groups = regex
            .captures("(scanned 12, total 345, linked 6, shared 7, erred 8)")
            .expect("progress is parsed");
        let payload = payload_from_progress_captures(groups);

        assert_eq!(payload.items, 12);
        assert_eq!(payload.total, 345);
        assert_eq!(payload.errors, 8);
    }

    #[test]
    fn restricted_path_regex_parses_pdu_errors() {
        let regex = restricted_path_regex();
        let groups = regex
            .captures(r#"[error] read_dir "/Users/me/Library/Mobile Documents": Permission denied"#)
            .expect("error is parsed");
        let restricted_path = restricted_path_from_captures(groups);

        assert_eq!(restricted_path.operation, "read_dir");
        assert_eq!(restricted_path.path, "/Users/me/Library/Mobile Documents");
        assert_eq!(restricted_path.message, "Permission denied");
    }

    #[test]
    fn scan_args_include_shared_options_on_supported_platforms() {
        let args = initial_scan_args_for_platform("0.001", true);

        assert!(args.contains(&"--deduplicate-hardlinks".to_string()));
        assert!(args.contains(&"--omit-json-shared-details".to_string()));
        assert!(args.contains(&"--omit-json-shared-summary".to_string()));
        assert!(args.contains(&"--min-ratio=0.001".to_string()));
    }

    #[test]
    fn scan_args_skip_unsupported_shared_options_on_windows() {
        let args = initial_scan_args_for_platform("0.001", false);

        assert!(!args.contains(&"--deduplicate-hardlinks".to_string()));
        assert!(!args.contains(&"--omit-json-shared-details".to_string()));
        assert!(!args.contains(&"--omit-json-shared-summary".to_string()));
        assert!(args.contains(&"--min-ratio=0.001".to_string()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn default_scan_args_skip_unsupported_windows_options() {
        let args = initial_scan_args("0.001");

        assert!(!args.contains(&"--deduplicate-hardlinks".to_string()));
        assert!(!args.contains(&"--omit-json-shared-details".to_string()));
        assert!(!args.contains(&"--omit-json-shared-summary".to_string()));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn default_scan_args_include_shared_options_when_supported() {
        let args = initial_scan_args("0.001");

        assert!(args.contains(&"--deduplicate-hardlinks".to_string()));
        assert!(args.contains(&"--omit-json-shared-details".to_string()));
        assert!(args.contains(&"--omit-json-shared-summary".to_string()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_user_scan_expands_home_dirs_but_excludes_cloud_library_dirs() {
        let root = std::env::temp_dir().join(format!("squirreldisk-scan-{}", std::process::id()));
        let users = root.join("Users");
        let home = users.join("alice");
        let library = home.join("Library");
        fs::create_dir_all(library.join("Caches")).unwrap();
        fs::create_dir_all(library.join("CloudStorage")).unwrap();
        fs::create_dir_all(library.join("Mobile Documents")).unwrap();
        fs::create_dir_all(home.join("Desktop")).unwrap();

        let mut paths = Vec::new();
        assert!(add_user_homes_skipping_cloud_dirs(&users, &mut paths));
        let paths = paths.join("\n");

        assert!(paths.contains(&home.join("Desktop").display().to_string()));
        assert!(paths.contains(&library.join("Caches").display().to_string()));
        assert!(!paths.contains(&library.join("CloudStorage").display().to_string()));
        assert!(!paths.contains(&library.join("Mobile Documents").display().to_string()));

        fs::remove_dir_all(root).unwrap();
    }
}
