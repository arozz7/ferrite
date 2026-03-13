# Phase 11 — TUI Feature Expansion

## Summary

Six features added to the ferrite-tui crate covering a new hex-viewer screen,
carve-hit validation, duplicate suppression, write-blocker verification,
partition-table export, and recovery report generation.

---

## 11A — Sector Hex Viewer (screen index 6)

**New file:** `crates/ferrite-tui/src/screens/hex_viewer.rs`

- `HexViewerState` with fields: `device`, `current_lba`, `lba_input`, `editing`, `data`
- `set_device()` resets to LBA 0 and loads the first sector
- `load_sector()` reads one sector using `AlignedBuffer` and stores bytes in `self.data`
- Key bindings: `↑/↓` navigate sectors, `g` enters LBA-edit mode, `Enter` jumps, `Esc` cancels
- Render: classic `OOOOOOOO  HH … HH  |ASCII|` hex-dump layout, 16 bytes per row
- 2 unit tests: `set_device_resets_to_lba_zero`, `g_key_enters_edit_mode`

**Modified:** `crates/ferrite-tui/src/screens/mod.rs` — added `pub mod hex_viewer`

**Modified:** `crates/ferrite-tui/src/app.rs`
- `SCREEN_NAMES` expanded from 6 → 7 entries (added `" Hex "`)
- `App` struct gains `hex_viewer: HexViewerState`
- Device propagation in screen 0 now calls `self.hex_viewer.set_device(dev)`
- Screen 6 routed in `handle_key`, `render`; `q`-quit guard updated for screen 6 edit mode
- `screen_count_matches_names` test updated from 6 → 7

---

## 11B — Carve Hit Validation

**Modified:** `crates/ferrite-tui/src/screens/carving.rs`

- New types: `HitStatus` enum (`Unextracted`, `Extracting`, `Ok { bytes }`, `Truncated { bytes }`)
- New type: `HitEntry { hit: CarveHit, status: HitStatus }`
- `CarvingState.hits` changed from `Vec<CarveHit>` to `Vec<HitEntry>`
- `CarveMsg` extended with `Extracted { idx, bytes, truncated }` variant
- `CarvingState.tx: Option<Sender<CarveMsg>>` added; kept alive after `Done` so extraction results flow back
- `start_scan()` stores `tx` in `self.tx`; `rx` kept alive after `Done` for extraction messages
- `extract_selected()` sets `Extracting` immediately, spawns thread, sends `Extracted` on completion
- Truncation logic: footer empty → always `Ok`; has footer and `bytes >= max_size` → `Truncated`; else → `Ok`
- `render_hits_panel()` shows per-hit coloured status suffixes
- 2 new tests: `hit_entry_starts_unextracted`, `all_hits_start_as_unextracted`
- All 7 pre-existing tests kept passing

---

## 11C — Duplicate Carve Hit Suppression

**Modified:** `crates/ferrite-tui/src/screens/carving.rs`

- `hash_hit_prefix(device, offset) -> [u8; 32]`: reads up to 4096 bytes (sector-aligned) via `AlignedBuffer`, returns SHA-256 digest
- `dedup_hits(hits, device) -> Vec<CarveHit>`: keeps first occurrence of each unique hash
- Called in the scan background thread before sending `CarveMsg::Done`
- Import added: `sha2::{Digest, Sha256}`, `std::collections::HashMap`
- 1 new test: `dedup_removes_duplicate_hashes`

---

## 11D — Write-Blocker Verification

**Modified:** `crates/ferrite-tui/src/screens/imaging.rs`

- `ImagingMsg::WriteBlockerResult(bool)` variant added
- `ImagingState.write_blocked: Option<bool>` field added (`None`=unchecked, `Some(true)`=safe, `Some(false)`=warning)
- `set_device()` resets `write_blocked` to `None`
- `start_imaging()` resets `write_blocked` to `None` before starting
- In imaging thread, before `engine.run()`: attempts `OpenOptions::new().write(true).open(device_path)`; sends `WriteBlockerResult(false)` if open succeeds (not blocked), `WriteBlockerResult(true)` if it fails (blocked or denied)
- `tick()` handles `WriteBlockerResult` → sets `self.write_blocked`
- Stats panel renders: `checking…` (DarkGray), `OK` (Green), `WARNING — not blocked!` (Red)
- 1 new test: `write_blocker_result_message_sets_state`

---

## 11E — Partition Table Export

**Modified:** `crates/ferrite-tui/src/screens/partition.rs`

- `PartitionState.table` made `pub`
- `PartitionState.export_status: Option<String>` field added
- `KeyCode::Char('w')` in `handle_key` calls `export_partition_table()`
- `export_partition_table()`: MBR → reads 1 sector → `ferrite-partition.bin`; GPT → reads 34 sectors → `ferrite-partition.bin`; Recovered → writes text summary → `ferrite-partition.txt`
- Title updated to include `w: export` when `PartitionStatus::Done`
- `render_partition_table()` now accepts `export_status: Option<&str>` and renders it below the table
- Help bar text for screen 3 updated in `app.rs`
- 2 new tests: `w_key_when_no_device_does_nothing`, `w_key_when_no_table_does_nothing`

---

## 11F — Recovery Report Export

**New file:** `crates/ferrite-tui/src/screens/report.rs`

- `generate_report(device_info, smart, imaging_dest, imaging_mapfile, partition_table, carve_hit_count) -> String`
- Sections: Device, S.M.A.R.T., Imaging, Partitions, Carving
- Timestamp via `chrono::Local::now()`
- 2 tests: `report_contains_device_path`, `report_handles_no_smart_data`

**Modified:** `crates/ferrite-tui/src/screens/health.rs`

- `HealthState.last_smart_data: Option<SmartData>` field added (`pub`)
- Populated in `tick()` when `HealthMsg::Data` arrives

**Modified:** `crates/ferrite-tui/src/screens/mod.rs` — added `pub mod report`

**Modified:** `crates/ferrite-tui/src/app.rs`

- `App.report_status: Option<String>` field added
- Global `Shift+R` (`KeyCode::Char('R')`) handler calls `generate_report_to_file()`
- `generate_report_to_file()` collects data from all screens, writes `ferrite-report.txt`
- Help bar shows `report_status` when set

**Modified:** `crates/ferrite-tui/Cargo.toml` — added `chrono = { workspace = true }`

---

## Files Created

| File | Purpose |
|---|---|
| `crates/ferrite-tui/src/screens/hex_viewer.rs` | 11A — Sector Hex Viewer screen |
| `crates/ferrite-tui/src/screens/report.rs` | 11F — Recovery report generator |
| `aiChangeLog/phase-11.md` | This file |

## Files Modified

| File | Changes |
|---|---|
| `crates/ferrite-tui/src/screens/mod.rs` | +hex_viewer, +report modules |
| `crates/ferrite-tui/src/screens/carving.rs` | 11B HitEntry/HitStatus, 11C dedup |
| `crates/ferrite-tui/src/screens/partition.rs` | 11E export, pub table |
| `crates/ferrite-tui/src/screens/imaging.rs` | 11D write-blocker |
| `crates/ferrite-tui/src/screens/health.rs` | 11F last_smart_data |
| `crates/ferrite-tui/src/app.rs` | Hex screen, report key, SCREEN_NAMES 6→7 |
| `crates/ferrite-tui/Cargo.toml` | +chrono dep |

## Test Results

```
cargo test --workspace   175 passed, 0 failed
cargo clippy -- -D warnings   clean
cargo fmt --check   clean
```
