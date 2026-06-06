import { invoke } from "@tauri-apps/api/core";

export interface CachedScan {
  path: string;
  tree: DiskItem;
  used: number;
  errors?: number;
  restrictedPaths?: Array<RestrictedPath>;
  scannedAt?: number;
}

export interface ScanSnapshotSummary {
  path: string;
  used: number;
  scannedAt: number;
  errors: number;
  restrictedCount: number;
}

const scans = new Map<string, CachedScan>();

export const getCachedScan = async (path: string) => {
  const cached = scans.get(path);
  if (cached) {
    return cached;
  }

  const snapshot = await invoke<CachedScan | null>("get_scan_snapshot", {
    path,
  });
  if (snapshot) {
    scans.set(path, snapshot);
  }

  return snapshot;
};

export const hasCachedScan = (path: string) => scans.has(path);

export const listScanSnapshots = () =>
  invoke<Array<ScanSnapshotSummary>>("list_scan_snapshots");

export const setCachedScan = async (scan: CachedScan) => {
  const snapshot = {
    ...scan,
    scannedAt: scan.scannedAt || Date.now(),
    errors: scan.errors || scan.restrictedPaths?.length || 0,
    restrictedPaths: scan.restrictedPaths || [],
  };

  scans.set(scan.path, snapshot);
  await invoke("save_scan_snapshot", { snapshot });
};

export const clearCachedScan = async (path: string) => {
  scans.delete(path);
  await invoke("delete_scan_snapshot", { path });
};
