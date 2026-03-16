# Phase 26 — Pause fix, session persistence, carving split, drive select UX, sector map legend

## Summary

Five tasks completed in this phase:

1. **Task 1 (Bug fix):** Carving scan pause — rate and ETA now freeze correctly when paused.
2. **Task 2:** Session persistence — carving output dir and hex viewer LBA survive restarts.
3. **Task 3:** Split `carving/mod.rs` — extraction code moved to `carving/extract.rs`.
4. **Task 4:** Drive Select enhancements — sort toggle (`s`) and filter bar (`/`).
5. **Task 5:** Imaging sector map legend — dynamic legend based on terminal width.

---

## Task 1 — Pause rate/ETA fix

### Problem

Pressing `p` to pause a carving scan showed `[PAUSED]` in the status bar, but the rate
slowly decreased and ETA continued to climb. Root cause: `elapsed` used wall-clock time
(which kept ticking while paused) while `bytes_scanned` froze → rate decayed → ETA grew.

### Changes

#### `crates/ferrite-tui/src/screens/carving/mod.rs`

New fields on `CarvingState`:
- `paused_elapsed: std::time::Duration` — cumulative duration of all completed pause intervals.
- `paused_since: Option<std::time::Instant>` — start of the current pause interval (None when running).

Both initialised to `Duration::ZERO` / `None` in `new()` and reset to zero in `set_device()` and `start_scan()`.

Updated `toggle_pause()`:
- On pause: sets `paused_since = Some(Instant::now())`.
- On resume: accumulates `paused_since.elapsed()` into `paused_elapsed`; clears `paused_since`.

#### `crates/ferrite-tui/src/screens/carving/render.rs`

`render_progress()` (scan stats line) now computes:
```
paused_secs = paused_elapsed + paused_since.elapsed()  // includes current interval
active_secs = (wall_elapsed - paused_secs).max(0.001)
```
- Rate and ETA use `active_secs` instead of raw `wall_elapsed`.
- While paused: `rate_str = "— (paused)"`, `eta_str = ""` (no stale estimate shown).
- Elapsed timer also shows active time only.

---

## Task 2 — Session persistence

### Changes

#### `crates/ferrite-tui/src/session.rs`

Two new fields added to `Session` (both `#[serde(default)]` for backwards-compat with old JSON):
- `carving_output_dir: String`
- `hex_last_lba: u64`

Test updated: `save_and_load_roundtrip` now round-trips the new fields; struct literals in both tests updated.

#### `crates/ferrite-tui/src/app.rs`

`new()`: restores `carving.output_dir` (if non-empty) and `hex_viewer.current_lba` from session.

`run_loop()` Session save block: includes `carving_output_dir` and `hex_last_lba`.

---

## Task 3 — carving/mod.rs split

### Changes

#### `crates/ferrite-tui/src/screens/carving/mod.rs`

Removed:
- `fn extract_selected(&mut self)` body (~60 lines)
- `fn extract_all_selected(&mut self)` body (~220 lines)
- `CancelWriter<W>` struct + `Write` impl (~26 lines)

Added:
- `mod extract;` declaration

Imports cleaned up: `VecDeque`, `Write`, `Mutex` removed (no longer used in this file after the move).

Line count: 1 194 → 882 lines (−312 lines).

#### `crates/ferrite-tui/src/screens/carving/extract.rs` (new file, ~220 lines)

Contains all extraction code moved from `mod.rs`:
- `CancelWriter<W: Write>` struct + `Write` impl
- `impl CarvingState { fn filename_for_hit(), pub(super) fn extract_selected(), pub(super) fn extract_all_selected() }`

No visibility changes needed — child modules can access parent private fields directly.

---

## Task 4 — Drive Select sort + filter

### Changes

#### `crates/ferrite-tui/src/screens/drive_select.rs`

New types:
- `enum SortKey { Path, SizeDesc }` with `next()` and `label()` helpers.

New fields on `DriveSelectState`:
- `sort_key: SortKey` — current sort order (default: Path).
- `filter_input: String` — current filter text.
- `filtering: bool` — whether the filter bar is in input mode.

New method:
- `display_indices() -> Vec<usize>` — returns entry indices after applying filter + sort. Filter matches path, model, or serial (case-insensitive). SortKey::SizeDesc sorts largest first.
- `is_filtering() -> bool` — exposed so `app.rs` can suppress `q`-to-quit while filtering.

Key bindings:
- `s` — cycles sort (Path → Size ↓ → Path).
- `/` — opens filter bar; typing refines list; `Enter` closes without clearing; `Esc` clears filter.
- `Backspace` — removes last filter character.

`selected` now indexes the filtered+sorted display list.
`open_selected()` maps `selected` back through `display_indices()` to the real entry.

Block title updated to show current sort and available keys.
Filter bar rendered as a 1-row strip at the bottom of the area when active or when a filter string is set.

#### `crates/ferrite-tui/src/app.rs`

`handle_key()` quit guard: screen 0 (`drive_select.is_filtering()`) added alongside imaging, carving, hex viewer to prevent `q` quitting during filter input.

New tests: `sort_size_desc_orders_largest_first`, `filter_matches_path`, `filter_clear_on_esc`.

---

## Task 5 — Sector map dynamic legend

### Changes

#### `crates/ferrite-tui/src/screens/imaging/render.rs`

`render_sector_map()` now picks the legend string based on `area.width`:
- **≥ 90 cols:** Full: `██ Finished  ░░ Non-tried  ▒▒ Non-trim/scrape  ██ Bad  ▶ Current`
- **≥ 60 cols:** Abbreviated: `██ OK  ░░ Pending  ▒▒ Warn  ██ Bad  ▶ Pos`
- **< 60 cols:** No legend: `Sector Map` only

---

## File Size Summary (after phase)

| File | Lines |
|---|---|
| `carving/mod.rs` | 882 (was 1 194) |
| `carving/extract.rs` | 220 (new) |
| `drive_select.rs` | 354 (was 307) |
| `imaging/render.rs` | 471 (unchanged) |

## Test Results

- `cargo test --workspace` — all tests pass, 0 failures
- `cargo clippy --workspace -- -D warnings` — clean
