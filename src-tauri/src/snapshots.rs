use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictedPath {
    pub path: String,
    pub operation: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSnapshot {
    pub path: String,
    pub tree: serde_json::Value,
    pub used: u64,
    pub errors: Option<u64>,
    pub restricted_paths: Option<Vec<RestrictedPath>>,
    pub scanned_at: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSnapshotSummary {
    pub path: String,
    pub used: u64,
    pub errors: u64,
    pub restricted_count: usize,
    pub scanned_at: u64,
}

fn scan_snapshot_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("scan-snapshots");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn snapshot_file(app_handle: &tauri::AppHandle, path: &str) -> Result<PathBuf, String> {
    Ok(scan_snapshot_dir(app_handle)?.join(snapshot_filename(path)))
}

fn snapshot_filename(path: &str) -> String {
    let hash = path
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}.json")
}

#[tauri::command]
pub fn get_scan_snapshot(
    app_handle: tauri::AppHandle,
    path: String,
) -> Result<Option<ScanSnapshot>, String> {
    let file = snapshot_file(&app_handle, &path)?;
    if !file.exists() {
        return Ok(None);
    }

    let snapshot: ScanSnapshot =
        serde_json::from_slice(&fs::read(file).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;

    if snapshot.path == path {
        Ok(Some(snapshot))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub fn list_scan_snapshots(
    app_handle: tauri::AppHandle,
) -> Result<Vec<ScanSnapshotSummary>, String> {
    let dir = scan_snapshot_dir(&app_handle)?;
    scan_snapshot_summaries_from_dir(&dir)
}

fn scan_snapshot_summaries_from_dir(dir: &Path) -> Result<Vec<ScanSnapshotSummary>, String> {
    let entries = fs::read_dir(dir).map_err(|error| error.to_string())?;
    let mut snapshots = Vec::new();

    for entry in entries.flatten() {
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let bytes = match fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let snapshot = match serde_json::from_slice::<ScanSnapshot>(&bytes) {
            Ok(snapshot) => snapshot,
            Err(_) => continue,
        };
        let restricted_count = snapshot.restricted_paths.as_ref().map_or(0, Vec::len);
        snapshots.push(ScanSnapshotSummary {
            path: snapshot.path,
            used: snapshot.used,
            errors: snapshot.errors.unwrap_or(restricted_count as u64),
            restricted_count,
            scanned_at: snapshot.scanned_at.unwrap_or(0),
        });
    }

    snapshots.sort_by(|a, b| b.scanned_at.cmp(&a.scanned_at));
    Ok(snapshots)
}

#[tauri::command]
pub fn save_scan_snapshot(
    app_handle: tauri::AppHandle,
    snapshot: ScanSnapshot,
) -> Result<(), String> {
    let file = snapshot_file(&app_handle, &snapshot.path)?;
    let temp_file = file.with_extension("json.tmp");
    let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;

    fs::write(&temp_file, bytes).map_err(|error| error.to_string())?;
    fs::rename(temp_file, file).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_scan_snapshot(app_handle: tauri::AppHandle, path: String) -> Result<(), String> {
    let file = snapshot_file(&app_handle, &path)?;
    if file.exists() {
        fs::remove_file(file).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_snapshot_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "squirreldisk-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_snapshot(dir: &Path, filename: &str, snapshot: &ScanSnapshot) {
        fs::write(
            dir.join(filename),
            serde_json::to_vec(snapshot).expect("snapshot serializes"),
        )
        .unwrap();
    }

    #[test]
    fn snapshot_filename_is_stable_for_existing_cache_paths() {
        assert_eq!(snapshot_filename("/"), "af63a24c860189fe.json");
        assert_eq!(snapshot_filename("/Users/sirius"), "4b569a8c6d30d7d6.json");
        assert_eq!(
            snapshot_filename("/private/var/db"),
            "794c729b7762e696.json"
        );
    }

    #[test]
    fn scan_snapshot_summaries_skip_bad_files_and_sort_newest_first() {
        let dir = temp_snapshot_dir("summaries");

        write_snapshot(
            &dir,
            "old.json",
            &ScanSnapshot {
                path: "/old".to_string(),
                tree: serde_json::json!({"name": "/old"}),
                used: 10,
                errors: None,
                restricted_paths: Some(vec![RestrictedPath {
                    path: "/old/private".to_string(),
                    operation: Some("read_dir".to_string()),
                    message: Some("Permission denied".to_string()),
                }]),
                scanned_at: Some(100),
            },
        );
        write_snapshot(
            &dir,
            "new.json",
            &ScanSnapshot {
                path: "/new".to_string(),
                tree: serde_json::json!({"name": "/new"}),
                used: 20,
                errors: Some(7),
                restricted_paths: Some(vec![RestrictedPath {
                    path: "/new/private".to_string(),
                    operation: Some("read_dir".to_string()),
                    message: Some("Permission denied".to_string()),
                }]),
                scanned_at: Some(300),
            },
        );
        fs::write(dir.join("broken.json"), b"not json").unwrap();
        fs::write(dir.join("ignored.txt"), b"not a snapshot").unwrap();

        let summaries = scan_snapshot_summaries_from_dir(&dir).unwrap();

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].path, "/new");
        assert_eq!(summaries[0].used, 20);
        assert_eq!(summaries[0].errors, 7);
        assert_eq!(summaries[0].restricted_count, 1);
        assert_eq!(summaries[0].scanned_at, 300);
        assert_eq!(summaries[1].path, "/old");
        assert_eq!(summaries[1].errors, 1);

        fs::remove_dir_all(dir).unwrap();
    }
}
