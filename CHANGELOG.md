# Changelog

## v0.3.12

- Fixed phantom duplicate directories appearing alongside real ones (most visibly `Program Files` on Windows): restricted-path inserts now match existing folders by structure, not by an `isDirectory` flag that pdu's raw output never sets.
- Added a privacy access status check: shows when Full Disk Access (macOS) or admin elevation (Windows) is already granted, and offers a "Relaunch as Administrator" action on Windows to reduce restricted items.

## v0.3.11

- Fixed the right sidebar directory source so active and hover-preview contents cannot be rendered together.
- Clicking a sidebar directory now commits that directory through the same focus path used by the chart.
- Hovering sidebar rows now drives the matching chart hover highlight.
- Added a pdu-backed sidebar e2e smoke test that catches mixed active/preview directory rows.

## v0.3.10

- Removed unused Tauri HTTP and updater plugins, permissions, and updater endpoint configuration.
- Removed stale analytics typing so the app runtime has no external widget, analytics, updater, or HTTP plugin path.
- Removed external badge images from the README.
- Keeps the Windows scan fix and Headway removal from v0.3.8-v0.3.9.

## v0.3.9

- Removed the third-party Headway changelog widget and its external script.
- Keeps the Windows scan fix from v0.3.8.

## v0.3.8

- Fixed Windows scans by skipping `pdu` hardlink/shared-output flags that the Windows build cannot support.
- Added regression tests for platform-specific scan helper arguments.

## v0.3.7

- Restored hover preview for directory contents while keeping the sidebar to one directory at a time.
- Kept preview interactions consistent with navigation, drag, and delete behavior.

## v0.3.6

- Kept the sidebar contents anchored to the active directory while preserving hover highlighting.
- Made directory rows navigate even when a scanned directory has no loaded child entries.

## v0.3.5

- Fixed sidebar directory navigation so hovering chart slices no longer swaps the active file list.
- Merged duplicate scan tree entries that resolve to the same displayed directory.
