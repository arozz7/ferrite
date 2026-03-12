# Phase 13 — TUI UX Fixes & Windows S.M.A.R.T. Path Translation

## Summary
Bug fixes and UX improvements discovered during first real-device testing on Windows 11.

## Changes

### Bug Fix: Tab double-advance on Windows (`ferrite-tui/src/app.rs`)
- **Root cause:** Windows terminal sends both `KeyEventKind::Press` and `KeyEventKind::Release`
  for every keystroke. The event loop was handling both, so one Tab press advanced the screen
  index twice (e.g. Drives → Imaging, skipping Health).
- **Fix:** Added `key.kind == KeyEventKind::Press` guard in `run_loop` before dispatching
  to `handle_key`.

### UX: Auto-navigate to Health after drive selection (`ferrite-tui/src/app.rs`)
- After pressing Enter to select a drive on the Drives screen, `screen_idx` is now set to `1`
  (Health) automatically. The S.M.A.R.T. query starts immediately on arrival.
- Previously the user had to press Tab manually after selection with no feedback.

### UX: Imaging destination field clarity (`ferrite-tui/src/screens/imaging.rs`)
- Empty Dest field now renders in **yellow** with placeholder text:
  `(not set — press d)  e.g. D:\recovery\disk.img`
- Filled Dest field renders in **green** for clear confirmation.
- Added a hint line at the bottom of the config panel explaining both Dest and Mapfile fields.
- Config panel height increased from 9 to 11 rows to accommodate the hint line.

### UX: Platform-specific smartctl install instructions (`ferrite-tui/src/screens/health.rs`)
- Health error screen now shows OS-specific install guidance:
  - **Windows:** `winget install smartmontools` + PATH note + "Run as Administrator" prompt
  - **Linux:** `apt` / `dnf` / `pacman` commands + `sudo` note

### Bug Fix: S.M.A.R.T. fails on Windows with `\\.\PhysicalDriveN` paths (`ferrite-smart/src/runner.rs`)
- **Root cause:** The AppVeyor/MinGW build of smartctl (v7.5) does not accept
  `\\.\PhysicalDrive0`-style paths — it reports "Unable to detect device type" (exit code 1).
  It requires POSIX-style `/dev/sda` paths instead.
- **Fix:** Added `translate_device_path()` which converts `\\.\PhysicalDriveN` →
  `/dev/sd{letters}` on Windows (0→sda, 1→sdb, …, 25→sdz, 26→sdaa, …).
  Linux paths are passed through unchanged.
- **Also:** Relaxed the exit-code hard-failure check — now attempts JSON parsing even when
  bits 0-1 are set, only failing if stdout is also empty or unparseable. This handles NVMe
  and permission edge cases where smartctl exits non-zero but still emits valid JSON.

## Files Modified
- `crates/ferrite-tui/src/app.rs`
- `crates/ferrite-tui/src/screens/health.rs`
- `crates/ferrite-tui/src/screens/imaging.rs`
- `crates/ferrite-smart/src/runner.rs`

## Test Results
- `cargo test --workspace` — all pass
- Manual: Samsung SSD 830 on Windows 11 — Health tab shows full S.M.A.R.T. data correctly
