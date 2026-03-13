# Phase 13 — TUI UX Fixes, Windows S.M.A.R.T., Carving Extraction Engine

## Summary
Bug fixes and UX improvements from first real-device testing on Windows 11, plus a complete
rewrite of the carving extraction pipeline: multi-threaded pool, live progress tracking, and
per-file status indicators.

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

### Bug Fix: Files tab "unknown or unsupported filesystem" (`ferrite-filesystem/src/lib.rs`)
- **Root cause:** `detect_filesystem()` read sector 0 of the whole physical disk and expected a
  filesystem magic there, but sector 0 of a raw disk holds MBR/GPT, not a filesystem header.
- **Fix:** Added `probe_filesystem()` which tries offset 0 first, then walks MBR and GPT partition
  tables to find the first recognisable filesystem and its byte offset.
  Added `OffsetDevice` wrapper so parsers (NTFS/FAT32/ext4) continue to read from their own
  logical offset 0 without modification.
- **New test:** `detect_ntfs_on_mbr_first_partition`

### Feature: Carving scan progress (`ferrite-carver`, `ferrite-tui`)
- `scanner.rs`: Added `ScanProgress { bytes_scanned, device_size, hits_found }`.
  Refactored `scan()` to call a shared `scan_inner()`; added `scan_with_progress(tx, cancel, pause)`
  that sends progress updates via a bounded sync channel and honours cancel/pause atomics.
- `carving.rs`: Added a progress gauge + stats line (MB/s rate, elapsed, ETA) below the hit list.
  Gauge turns yellow with `[PAUSED]` title when paused.
  Added `p` (pause/resume) and `c` (stop) keys; stop preserves partial results.

### Feature: Carving extraction output directory (`ferrite-tui`)
- Added an output directory field (`o` key to edit) to the Carving screen header bar.
  Auto-suggested from the imaging destination path on screen entry.
- All extraction functions (`e` single, `E` selected) now write to this directory.

### Feature: Carving bulk hit selection (`ferrite-tui`)
- Each hit shows `[✓]` / `[ ]` toggle prefix; panel title shows selected count.
- `Space` toggles selection on the focused hit; `a` selects/deselects all.
- `E` extracts all selected hits at once.

### Feature: Text-format carving signatures (`config/signatures.toml`)
- Added 6 structured text-format signatures (total: 21 → 27):
  - **XML** (`<?xml`) — 50 MiB max
  - **HTML** (`<!DOCTYPE`) with `</html>` footer — 10 MiB max
  - **RTF** (`{\rtf1`) — 50 MiB max
  - **vCard** (`BEGIN:VCARD` / `END:VCARD`) — 1 MiB max
  - **iCalendar** (`BEGIN:VCALENDAR` / `END:VCALENDAR`) — 10 MiB max
  - **OLE2 Compound** (`D0 CF 11 E0 …`) — covers legacy DOC/XLS/PPT — 500 MiB max
- Updated `builtin_signatures_parse` assertion from 21 → 27.

### Bug Fix: Bulk extraction hang on large hit counts (`ferrite-tui/src/screens/carving.rs`)
- **Root cause:** `extract_all_selected()` spawned one OS thread per hit. With 30 k+ hits this
  exhausted OS thread limits; threads silently failed and disk I/O thrashed from thousands of
  concurrent random reads.
- **Fix:** Replaced the N-thread free-for-all with a **coordinator + bounded worker pool**:
  - Coordinator thread owns the work queue (`Arc<Mutex<VecDeque>>`) and a private result channel.
  - N worker threads (`min(cpu_cores, 8).max(2)`) each pop one item, extract it, push the result.
  - Coordinator collects results sequentially, forwards `ExtractionProgress` / `Extracted` /
    `ExtractionDone` messages to the TUI channel.
  - Cap of 8 balances SSD throughput vs HDD seek overhead.

### Feature: Extraction progress bar + pulse indicator (`ferrite-tui/src/screens/carving.rs`)
- A cyan progress gauge appears at the top of the Hits panel during bulk extraction showing:
  - Braille spinner (`⠋⠙⠹…`) cycling every 100 ms — stops spinning if extraction stalls.
  - `N / total — current_filename` label updating per file.
  - Stats line: total bytes written · MB/s write rate · elapsed time · file-rate ETA.
- `c` cancels extraction (gauge turns red, shows `Cancelling…` until the worker finishes
  its current file and the coordinator drains).
- Panel title shows live `N extracted` count.

### Feature: Three-state per-file extraction indicators (`ferrite-tui/src/screens/carving.rs`)
- Added `HitStatus::Queued` — file is in the work queue, not yet picked up by a worker.
- `HitStatus::Extracting` is now set by a `CarveMsg::ExtractionStarted { idx }` message sent
  by the worker the instant it begins reading (not at batch-start).
- Visible states in the hit list:
  - *(nothing)* — not selected / not in a batch
  - `[queued]` grey — waiting in the pool queue
  - `[extracting…]` yellow — a worker is actively reading/writing this file right now
  - `[OK 45.2 KiB]` green — extraction complete
  - `[TRUNC 10.0 MiB]` red — hit max_size without finding a footer

## Files Modified
- `crates/ferrite-tui/src/app.rs`
- `crates/ferrite-tui/src/screens/health.rs`
- `crates/ferrite-tui/src/screens/imaging.rs`
- `crates/ferrite-tui/src/screens/carving.rs`
- `crates/ferrite-smart/src/runner.rs`
- `crates/ferrite-filesystem/src/lib.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/lib.rs`
- `config/signatures.toml`

## Test Results
- `cargo test --workspace` — 179 tests pass, 0 failures
- Manual: Samsung SSD 830 on Windows 11 — Health tab shows full S.M.A.R.T. data correctly
- Manual: 30 999-hit carving session — multi-threaded extraction ran to completion with live
  progress; `c` cancel confirmed to stop mid-batch cleanly
