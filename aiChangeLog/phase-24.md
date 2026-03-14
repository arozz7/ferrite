# Phase 24 — Imaging Enhancements

## Summary

Four enhancements to the Imaging screen: a real-time sector map visualisation driven by periodic mapfile snapshots, ETA computed from rate and remaining bytes, a manual pause/resume key (`p`), and richer drive info (model, serial, capacity) in the Source line plus a resume indicator that detects an existing mapfile.

## Changes

### `crates/ferrite-imaging/src/progress.rs`

- Added `map_snapshot: Option<Vec<crate::mapfile::Block>>` as the last field of `ProgressUpdate`.  Defaults to `None` on the vast majority of ticks.

### `crates/ferrite-imaging/src/engine.rs`

- Added `snapshot_counter: u32` field to `ImagingEngine`; initialised to `0` in `new()`.
- In `make_progress()`: increments the counter on every call and populates `map_snapshot` with a clone of the full block list every 50th call (i.e., on calls 1, 51, 101, …).  All other calls set `map_snapshot` to `None` to avoid cloning the block list on every sector read.

### `crates/ferrite-tui/src/screens/imaging.rs`

#### New state fields on `ImagingState`
- `sector_map: Vec<ferrite_imaging::mapfile::Block>` — holds the latest snapshot for rendering.
- `user_pause: Arc<AtomicBool>` — flag shared with `ChannelReporter` to pause the imaging thread.
- `user_paused: bool` — UI-side mirror of the flag.
- `imaging_resumed: bool` — set at session start when a mapfile already exists on disk.

#### `ChannelReporter`
- Added `user_pause: Arc<AtomicBool>` field (manual pause, alongside existing thermal `pause`).
- `report()` now spin-waits while *either* `pause` or `user_pause` is set.

#### `tick()`
- Extracts `map_snapshot` from each `Progress` message and stores it in `self.sector_map` when present.

#### `handle_key()`
- Added `p` key: toggles manual pause/resume while `Running` or while already paused.

#### `start_imaging()`
- Detects resume by checking whether the mapfile path is non-empty and the file already exists; stores result in `self.imaging_resumed`.
- Resets `user_pause`, `user_paused`, and `sector_map` before starting a new session.
- Passes `Arc::clone(&self.user_pause)` into `ChannelReporter`.

#### `render()`
- **Source line**: now shows model, serial (when available from `DeviceInfo`), and capacity in GiB alongside the device path.
- **Resume line**: added after the Mapfile config line; shows "YES — continuing from saved mapfile" (cyan bold) or "NO — fresh start" (dark gray).
- **Layout**: config panel height increased from 11 to 12 to accommodate the new Resume line; a new `Constraint::Length(6)` panel for the sector map inserted between the progress bar and stats; stats panel moved to `chunks[3]`.
- **Progress bar title**: changes to `[⚠ THERMAL PAUSE]` or `[⏸ PAUSED — p to resume]` as appropriate.
- **Progress bar style**: turns yellow while user-paused or thermally paused.
- **ETA**: computed from `read_rate_bps` and remaining bytes; formatted as `hh:mm`, `mm:ss`, or `Xs`; appended to the stats line.
- **Sector map panel**: new `render_sector_map()` method renders a coloured block grid using the latest snapshot.  Each terminal cell represents a proportional byte range of the device.  Colours: green (Finished), red (BadSector), yellow (NonTrimmed/NonScraped), dark-gray (NonTried), cyan `▶` (current read head).

### `crates/ferrite-tui/src/app.rs`

- Updated help text for screen 2 to include `p: pause`.

## Files Modified

- `crates/ferrite-imaging/src/progress.rs`
- `crates/ferrite-imaging/src/engine.rs`
- `crates/ferrite-tui/src/screens/imaging.rs`
- `crates/ferrite-tui/src/app.rs`
- `aiChangeLog/phase-24.md` (this file)

## Test Results

- `cargo test --workspace` — 219 tests pass, 0 failures
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` (Phase 24 files) — clean
