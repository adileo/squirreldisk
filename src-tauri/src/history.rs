use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_REPORT_ROWS: usize = 8;
const SNAPSHOTS_TO_KEEP_PER_ROOT: usize = 20;

#[derive(Deserialize)]
struct ScanNode {
    name: String,
    #[serde(default)]
    data: u64,
    #[serde(default)]
    children: Vec<ScanNode>,
}

#[derive(Clone, Deserialize, Serialize)]
struct ScanHistorySnapshot {
    root_path: String,
    captured_at: u64,
    total_size: u64,
    entries: Vec<ScanHistoryEntry>,
}

#[derive(Clone, Deserialize, Serialize)]
struct ScanHistoryEntry {
    path: String,
    size: u64,
    is_directory: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthEntry {
    pub path: String,
    pub size: u64,
    pub previous_size: u64,
    pub delta: i64,
    pub percent_change: Option<f64>,
    pub daily_rate: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryReport {
    pub root_path: String,
    pub snapshot_count: usize,
    pub current_timestamp: u64,
    pub previous_timestamp: Option<u64>,
    pub total_size: u64,
    pub total_delta: Option<i64>,
    pub top_growth: Vec<GrowthEntry>,
}

pub fn save_snapshot(
    app_handle: tauri::AppHandle,
    root_path: String,
    tree: serde_json::Value,
) -> Result<HistoryReport, String> {
    let tree: ScanNode = serde_json::from_value(tree).map_err(|error| error.to_string())?;
    let snapshot = build_snapshot(root_path, tree)?;
    let dir = history_dir(&app_handle)?;

    write_snapshot(&dir, &snapshot)?;
    prune_snapshots(&dir, &snapshot.root_path)?;

    let snapshots = snapshots_for_root(&dir, &snapshot.root_path)?;
    Ok(
        build_report(&snapshots, &snapshot.root_path).unwrap_or_else(|| HistoryReport {
            root_path: snapshot.root_path,
            snapshot_count: 1,
            current_timestamp: snapshot.captured_at,
            previous_timestamp: None,
            total_size: snapshot.total_size,
            total_delta: None,
            top_growth: Vec::new(),
        }),
    )
}

fn history_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_handle
        .path_resolver()
        .app_data_dir()
        .ok_or_else(|| "Could not resolve application data directory".to_string())?
        .join("scan-history");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn build_snapshot(root_path: String, tree: ScanNode) -> Result<ScanHistorySnapshot, String> {
    let captured_at = now_millis()?;
    let mut entries = Vec::new();
    let root_node_path = node_path(&root_path, None, &tree.name);
    flatten_node(&root_path, &root_node_path, &tree, &mut entries);

    Ok(ScanHistorySnapshot {
        root_path,
        captured_at,
        total_size: tree.data,
        entries,
    })
}

fn flatten_node(
    root_path: &str,
    current_path: &str,
    node: &ScanNode,
    entries: &mut Vec<ScanHistoryEntry>,
) {
    entries.push(ScanHistoryEntry {
        path: current_path.to_string(),
        size: node.data,
        is_directory: !node.children.is_empty(),
    });

    for child in &node.children {
        let child_path = node_path(root_path, Some(current_path), &child.name);
        flatten_node(root_path, &child_path, child, entries);
    }
}

fn node_path(root_path: &str, parent_path: Option<&str>, name: &str) -> String {
    if name == "(total)" {
        return root_path.to_string();
    }

    if Path::new(name).is_absolute() {
        return name.to_string();
    }

    let parent = parent_path.unwrap_or(root_path);
    if parent == "/" {
        format!("/{name}")
    } else {
        Path::new(parent).join(name).display().to_string()
    }
}

fn write_snapshot(dir: &Path, snapshot: &ScanHistorySnapshot) -> Result<(), String> {
    let filename = format!(
        "{}-{}.json",
        root_hash(&snapshot.root_path),
        snapshot.captured_at
    );
    let path = dir.join(filename);
    let temp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(snapshot).map_err(|error| error.to_string())?;

    fs::write(&temp_path, bytes).map_err(|error| error.to_string())?;
    fs::rename(temp_path, path).map_err(|error| error.to_string())
}

fn snapshots_for_root(dir: &Path, root_path: &str) -> Result<Vec<ScanHistorySnapshot>, String> {
    let mut snapshots = Vec::new();

    for entry in fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let bytes = match fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let snapshot = match serde_json::from_slice::<ScanHistorySnapshot>(&bytes) {
            Ok(snapshot) => snapshot,
            Err(_) => continue,
        };

        if snapshot.root_path == root_path {
            snapshots.push(snapshot);
        }
    }

    snapshots.sort_by(|a, b| a.captured_at.cmp(&b.captured_at));
    Ok(snapshots)
}

fn prune_snapshots(dir: &Path, root_path: &str) -> Result<(), String> {
    let mut files = Vec::new();

    for entry in fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let bytes = match fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let snapshot = match serde_json::from_slice::<ScanHistorySnapshot>(&bytes) {
            Ok(snapshot) => snapshot,
            Err(_) => continue,
        };

        if snapshot.root_path == root_path {
            files.push((snapshot.captured_at, entry.path()));
        }
    }

    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(SNAPSHOTS_TO_KEEP_PER_ROOT) {
        let _ = fs::remove_file(path);
    }

    Ok(())
}

fn build_report(snapshots: &[ScanHistorySnapshot], root_path: &str) -> Option<HistoryReport> {
    let current = snapshots.last()?;
    let previous = snapshots
        .iter()
        .rev()
        .skip(1)
        .find(|snapshot| snapshot.captured_at < current.captured_at);

    let mut report = HistoryReport {
        root_path: root_path.to_string(),
        snapshot_count: snapshots.len(),
        current_timestamp: current.captured_at,
        previous_timestamp: previous.map(|snapshot| snapshot.captured_at),
        total_size: current.total_size,
        total_delta: previous
            .map(|snapshot| current.total_size as i64 - snapshot.total_size as i64),
        top_growth: Vec::new(),
    };

    if let Some(previous) = previous {
        let previous_sizes: HashMap<&str, u64> = previous
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.size))
            .collect();
        let days = elapsed_days(previous.captured_at, current.captured_at);

        let mut growth: Vec<GrowthEntry> = current
            .entries
            .iter()
            .filter(|entry| entry.path != current.root_path)
            .filter_map(|entry| {
                let previous_size = *previous_sizes.get(entry.path.as_str()).unwrap_or(&0);
                let delta = entry.size as i64 - previous_size as i64;
                if delta <= 0 {
                    return None;
                }

                Some(GrowthEntry {
                    path: entry.path.clone(),
                    size: entry.size,
                    previous_size,
                    delta,
                    percent_change: percent_change(previous_size, entry.size),
                    daily_rate: days.map(|days| delta as f64 / days),
                })
            })
            .collect();

        growth.sort_by(|a, b| b.delta.cmp(&a.delta));
        growth.truncate(MAX_REPORT_ROWS);
        report.top_growth = growth;
    }

    Some(report)
}

fn percent_change(previous_size: u64, current_size: u64) -> Option<f64> {
    if previous_size == 0 {
        None
    } else {
        Some(((current_size as f64 - previous_size as f64) / previous_size as f64) * 100.0)
    }
}

fn elapsed_days(previous_timestamp: u64, current_timestamp: u64) -> Option<f64> {
    if current_timestamp <= previous_timestamp {
        return None;
    }

    let days = (current_timestamp - previous_timestamp) as f64 / 86_400_000.0;
    if days > 0.0 {
        Some(days)
    } else {
        None
    }
}

fn now_millis() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| error.to_string())
}

fn root_hash(path: &str) -> String {
    let hash = path
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, data: u64, children: Vec<ScanNode>) -> ScanNode {
        ScanNode {
            name: name.to_string(),
            data,
            children,
        }
    }

    fn snapshot(
        captured_at: u64,
        root_path: &str,
        entries: Vec<(&str, u64)>,
    ) -> ScanHistorySnapshot {
        ScanHistorySnapshot {
            root_path: root_path.to_string(),
            captured_at,
            total_size: entries[0].1,
            entries: entries
                .into_iter()
                .map(|(path, size)| ScanHistoryEntry {
                    path: path.to_string(),
                    size,
                    is_directory: true,
                })
                .collect(),
        }
    }

    #[test]
    fn flatten_uses_scan_root_for_total_nodes() {
        let root = node(
            "(total)",
            300,
            vec![node("/Users", 200, vec![node("zhak", 100, Vec::new())])],
        );

        let built = build_snapshot("/".to_string(), root).unwrap();
        let paths: Vec<&str> = built
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        assert_eq!(paths, vec!["/", "/Users", "/Users/zhak"]);
    }

    #[test]
    fn report_ranks_positive_growth_only() {
        let previous = snapshot(
            1_000,
            "/Users",
            vec![
                ("/Users", 1_000),
                ("/Users/a", 400),
                ("/Users/b", 500),
                ("/Users/c", 100),
            ],
        );
        let current = snapshot(
            86_401_000,
            "/Users",
            vec![
                ("/Users", 1_700),
                ("/Users/a", 900),
                ("/Users/b", 300),
                ("/Users/d", 200),
            ],
        );

        let report = build_report(&[previous, current], "/Users").unwrap();

        assert_eq!(report.total_delta, Some(700));
        assert_eq!(report.top_growth.len(), 2);
        assert_eq!(report.top_growth[0].path, "/Users/a");
        assert_eq!(report.top_growth[0].delta, 500);
        assert_eq!(report.top_growth[1].path, "/Users/d");
        assert_eq!(report.top_growth[1].previous_size, 0);
    }
}
