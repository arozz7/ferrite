# Phase 09 — ext4 Extent Tree, Read-Rate Monitoring, Temperature Guard

## Summary

Three improvements from the senior data-recovery engineer review, targeting
the two remaining 🔴 Critical items and one 🟠 High item:

| ID  | Item                                   | Priority  |
|-----|----------------------------------------|-----------|
| 9A  | ext4 extent tree support               | 🔴 Critical |
| 9B  | Rolling read-rate monitoring           | 🟠 High    |
| 9C  | Temperature monitoring + thermal pause | 🟠 High    |

---

## Changes by Item

### 9A — ext4 Extent Tree Support

**`crates/ferrite-filesystem/src/ext4.rs`**

The ext4 parser previously handled only direct blocks (i_block[0..11]) and
single-indirect blocks.  Since Linux 2.6.23, ext4 defaults to the extents
B-tree (inode flag `EXT4_INODE_EXTENTS = 0x80000`).  Any file written by a
modern Linux system uses extents.

Changes:
- Added constants `EXT4_INODE_EXTENTS: u32 = 0x0008_0000` and
  `EXT4_EXTENT_MAGIC: u16 = 0xF30A`.
- Added `walk_extent_node(&self, data: &[u8]) -> Result<Vec<u64>>`:
  - Validates the 12-byte `ext4_extent_header` magic.
  - `depth == 0` → iterates `ext4_extent` leaf entries (12 bytes each),
    yielding contiguous physical block ranges (`ee_start_hi:lo + 0..ee_len`).
    High bit of `ee_len` (uninitialized extents) is masked out.
  - `depth > 0` → follows `ext4_extent_idx` index entries recursively by
    reading child blocks via `read_block()`.
- `list_inode()`: reads `i_flags` at inode byte 32; if
  `EXT4_INODE_EXTENTS` is set, calls `walk_extent_node(&inode[40..])` to
  collect directory blocks.  Falls back to the existing block-map path
  otherwise.
- `read_file()`: same flag check; extent path iterates physical blocks
  returned by `walk_extent_node`, truncating to `file_size`.

New tests (total: 25 → +3):
- `extent_root_directory_lists_file` — directory inode using extent tree
- `extent_file_read_returns_content` — file inode using extent tree
- `walk_extent_node_bad_magic_returns_error` — invalid header returns `Err`

---

### 9B — Rolling Read-Rate Monitoring

**`crates/ferrite-imaging/src/progress.rs`**

Added `read_rate_bps: u64` field to `ProgressUpdate`.  Zero until at least
one full second has elapsed; then updated as a rolling 1-second average.

**`crates/ferrite-imaging/src/engine.rs`**

- Added three fields to `ImagingEngine`:
  - `last_rate_instant: Instant` — checkpoint timestamp
  - `last_rate_bytes: u64` — `bytes_finished` at last checkpoint
  - `current_rate_bps: u64` — most recent computed rate
- Changed `make_progress` from `&self` to `&mut self` to allow rate updates.
- Every call to `make_progress` checks if ≥ 1 second has elapsed since the
  last checkpoint; if so, computes
  `rate = delta_bytes / elapsed_secs` and stores it in `current_rate_bps`.

**`crates/ferrite-tui/src/screens/imaging.rs`**

Statistics panel now shows:
- `Rate: X.X MB/s` when read rate is available
- `Rate: X.X MB/s ⚠ SLOW` when rate < 5 MB/s (drive struggling)
- `Rate: —` before the first full second of imaging

---

### 9C — Temperature Monitoring + Thermal Pause

**`crates/ferrite-tui/src/screens/imaging.rs`**

Added a thermal guard thread that runs alongside the imaging engine and a
cooperative pause mechanism via `Arc<AtomicBool>`.

New `ImagingMsg` variants:
- `Temperature(u32)` — current drive temperature in °C
- `ThermalPause` — drive exceeded 55 °C; imaging paused
- `ThermalResume` — drive cooled to ≤ 50 °C; imaging resumed

New `ImagingState` fields:
- `pause: Arc<AtomicBool>` — shared thermal pause flag
- `current_temp: Option<u32>` — most recent temperature reading
- `thermal_paused: bool` — whether imaging is currently paused

`ChannelReporter` changes:
- Added `pause: Arc<AtomicBool>` field.
- In `report()`: spin-waits (`thread::yield_now()`) while `pause` is set,
  checking `cancel` every iteration so the user can still abort.

Thermal guard thread (spawned in `start_imaging()`):
- Polls `ferrite_smart::query(device_path, None)` every 60 seconds.
- Temperature ≥ 55 °C: sets `pause = true`, sends `ThermalPause`.
- Temperature ≤ 50 °C (after pause): sets `pause = false`, sends `ThermalResume`.
- Exits when `cancel` is set (checked every second during the 60-second poll
  interval so cancellation is responsive).

Statistics panel additions:
- `Temp: XX°C` when a temperature reading is available.
- `Temp: XX°C ⚠ PAUSED (>55°C)` while thermally paused.

---

## Files Modified

| File | Change |
|------|--------|
| `crates/ferrite-filesystem/src/ext4.rs` | Extent tree support (+3 tests) |
| `crates/ferrite-imaging/src/progress.rs` | `read_rate_bps` field |
| `crates/ferrite-imaging/src/engine.rs` | Rate tracking in `make_progress` |
| `crates/ferrite-tui/src/screens/imaging.rs` | Thermal guard + rate display |
| `aiChangeLog/phase-09.md` | This file |

---

## Test Results

| Crate              | Tests       |
|--------------------|-------------|
| ferrite-blockdev   | 15          |
| ferrite-carver     | 20          |
| ferrite-core       | 2           |
| ferrite-filesystem | 25 (+3)     |
| ferrite-imaging    | 25          |
| ferrite-partition  | 27          |
| ferrite-smart      | 18          |
| ferrite-tui        | 26          |
| **Total**          | **158 (+3)**|

All 158 tests pass.  `cargo clippy --workspace -- -D warnings` and
`cargo fmt --check` are both clean.

---

## Known Limitations (still open)

- ext4 double/triple-indirect blocks not implemented (only direct + single-indirect + extents).
- exFAT and HFS+ are detect-only; no directory browser or file extraction.
- `ImagingPhase::Retry { attempt, max }` displayed without counter values.
- Thermal guard uses `smartctl` — not available in all environments.
