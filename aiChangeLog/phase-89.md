# Phase 89 — Volume Quiesce (Exclusive Drive Access)

## Problem
When a damaged drive is connected, Windows immediately mounts any readable
filesystem and fires background services (Search Indexer, AutoPlay, Explorer
thumbnail cache).  These compete directly for I/O on a stressed drive, causing
100% disk utilisation before Ferrite even begins imaging.

## Solution
Take all Windows volumes on the selected physical disk **offline** via
`IOCTL_VOLUME_OFFLINE` the moment a device is selected.  Raw
`\\.\PhysicalDriveX` access is unaffected by volume-offline state, so all
Ferrite operations (imaging, carving, filesystem browsing) continue normally.
A RAII guard re-onlines every volume automatically on device deselect or app
exit.

## Files Changed

### New
- `crates/ferrite-blockdev/src/volume_guard.rs`
  - `VolumeGuard` — RAII guard; `acquire(disk_number)` offlines volumes, `Drop` re-onlines them
  - `VolsStatus` — `Quiesced(n)` / `Partial{n_ok, n_total}` / `NoVolumes` / `NeedAdmin`
  - `parse_disk_number(path)` — maps `\\.\PhysicalDriveN` → `Some(N)`, image files → `None`
  - Private helpers: `volumes_on_disk`, `query_disk_number`, `try_offline`, `try_online`,
    `open_volume_query`, `open_volume_rw`, `strip_trailing_backslash`
  - IOCTL constants defined from Windows SDK:
    - `IOCTL_STORAGE_GET_DEVICE_NUMBER` = `0x002D_1080`
    - `IOCTL_VOLUME_ONLINE`             = `0x0056_C008`
    - `IOCTL_VOLUME_OFFLINE`            = `0x0056_C00C`
  - 9 unit tests (all `#[cfg(target_os = "windows")]` free — pure logic tested)

### Modified
- `crates/ferrite-blockdev/src/lib.rs`
  - Export `VolumeGuard`, `VolsStatus`, `parse_disk_number` under `#[cfg(target_os = "windows")]`

- `crates/ferrite-tui/src/app.rs`
  - `App.volume_guard: Option<VolumeGuard>` field (Windows only)
  - `App::quiesce_volumes(&mut self, path: &str)` — drops old guard, acquires new one
  - Called from both device-selection paths:
    - Normal drive select (Drives tab `Enter`)
    - Session resume (`SessionMsg::Resume`)

- `crates/ferrite-tui/src/screens/drive_select.rs`
  - `DriveSelectState.vols_status: Option<VolsStatus>` field (Windows only)
  - Status badge rendered above the image-open overlay:
    - Green  — `⬛ VOLUMES OFFLINE: N — background I/O suppressed`
    - Yellow — `⚠ VOLUMES OFFLINE: N/M — system volumes skipped`
    - Gray   — `○ No mounted volumes — drive is already quiet`
    - Red    — `✗ Cannot offline volumes — run Ferrite as Administrator`

## Behaviour

| Scenario | Result |
|---|---|
| Physical drive selected | Volumes offlined immediately; badge shown |
| Image file selected (`f`) | `parse_disk_number` returns `None`; no guard created |
| System/boot volume on same disk | `IOCTL_VOLUME_OFFLINE` fails; skipped with `Partial` status |
| Drive has no mounted volumes | `VolsStatus::NoVolumes` (raw/unformatted drive) |
| Not running as admin | `VolsStatus::NeedAdmin` red badge |
| New device selected | Old guard dropped → previous drive's volumes re-onlined |
| Ferrite exits | `App` dropped → `VolumeGuard` dropped → all volumes re-onlined |

## Test Results
- 9 new unit tests in `volume_guard::tests` — all passing
- Total workspace: 878 tests (was 869) — all passing
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — clean
