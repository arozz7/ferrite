# Phase 10 — Advanced Imaging Controls

## Summary

Four sub-features added across `ferrite-imaging`, `ferrite-smart`, and `ferrite-tui`.

---

## 10A — LBA Range Selection

### `crates/ferrite-imaging/src/config.rs`
- Added `start_lba: Option<u64>` and `end_lba: Option<u64>` fields to `ImagingConfig`.
- `Default` initialises both to `None`.
- `validate()` now rejects configurations where `start_lba >= end_lba`.

### `crates/ferrite-imaging/src/engine.rs`
- `ImagingEngine::new()` marks bytes outside `[start_lba, end_lba)` as `Finished` in the mapfile immediately after creation, so all passes skip those regions.
- `mapfile` binding changed to `mut` to allow the range updates.
- All existing tests updated to include `start_lba: None, end_lba: None` in explicit `ImagingConfig` constructions.
- New tests: `start_lba_marks_prefix_finished`, `end_lba_marks_suffix_finished`.

### `crates/ferrite-tui/src/screens/imaging.rs`
- `EditField` gains `StartLba` and `EndLba` variants.
- `ImagingState` gains `start_lba_str: String` and `end_lba_str: String`.
- `set_device()` resets both strings.
- `field_mut()` routes the new variants.
- `handle_key()` maps `l` → `StartLba`, `e` → `EndLba`.
- `start_imaging()` parses both strings and passes them to `ImagingConfig`.
- Config panel renders `Start` and `End` rows; panel height raised to 8.

### `crates/ferrite-tui/src/app.rs`
- Help text for screen 2 updated to mention `l: start LBA` and `e: end LBA`.

---

## 10B — Session Persistence

### `crates/ferrite-tui/Cargo.toml`
- Added `serde` and `serde_json` to `[dependencies]`.
- Added `tempfile` to `[dev-dependencies]`.

### `crates/ferrite-tui/src/session.rs` (new)
- `Session` struct with `imaging_dest`, `imaging_mapfile`, `imaging_start_lba`, `imaging_end_lba`.
- `Session::load()` reads `ferrite-session.json`; returns `Default` on any error.
- `Session::save()` writes `ferrite-session.json` (silently ignores I/O errors).
- Tests: `load_missing_file_returns_default`, `save_and_load_roundtrip`.

### `crates/ferrite-tui/src/lib.rs`
- Added `pub mod session;`.

### `crates/ferrite-tui/src/app.rs`
- `App::new()` loads session on startup and applies fields to `ImagingState`.
- `run_loop()` saves session to disk before returning.

---

## 10C — S.M.A.R.T. Bad LBA → Mapfile Pre-population

### `crates/ferrite-smart/src/types.rs`
- Added `bad_sector_lbas: Vec<u64>` to `SmartData`.

### `crates/ferrite-smart/src/parser.rs`
- Added `RawAtaErrorLog`, `RawAtaErrorLogSummary`, `RawAtaErrorEntry` structs.
- Added `ata_smart_error_log: Option<RawAtaErrorLog>` to `RawSmartctl`.
- Added `extract_lba_from_error_description()` helper (parses `"at LBA = 0xHHHH"` pattern).
- `parse()` now extracts `bad_sector_lbas` from the error log table.
- Tests updated: `missing_fields_use_defaults` asserts `bad_sector_lbas` is empty.
- New test: `parse_error_log_extracts_lba`.

### `crates/ferrite-imaging/src/engine.rs`
- Added `pub fn sector_size(&self) -> u32`.
- Added `pub fn pre_populate_bad_sectors(&mut self, sector_size: u64, bad_lbas: &[u64])`.
- New tests: `pre_populate_marks_known_bad_sectors`, `pre_populate_out_of_range_is_ignored`.

### `crates/ferrite-tui/src/screens/imaging.rs`
- Imaging background thread captures `device_path_for_smart` before moving `device`.
- After engine creation, queries S.M.A.R.T. and calls `pre_populate_bad_sectors` if bad LBAs present.

---

## 10D — Configurable Copy Block Size in TUI

### `crates/ferrite-tui/src/screens/imaging.rs`
- `EditField` gains `BlockSize` variant.
- `ImagingState` gains `block_size_str: String`.
- `field_mut()` routes `BlockSize`.
- `handle_key()` maps `b` → `BlockSize`.
- `start_imaging()` parses `block_size_str` as KiB (defaults to 512 KiB).
- Config panel renders `BlockSz` row; panel height raised to 8 (combined with 10A).

---

## Test Results

All 145 tests across the workspace pass. Zero clippy warnings. Formatting clean.
