# Phase 12 — Change Log

## Summary

Phase 12 adds three enhancements across the carver, filesystem, and imaging subsystems.

---

## 12A — Missing Carving Signatures

**`config/signatures.toml`**
- Added 3 new file signatures (total: 18 → 21):
  - `Windows Event Log` (.evtx) — header `45 4C 46 49 4C 45 00`, max 100 MiB
  - `Outlook PST/OST` (.pst) — header `21 42 44 4E`, max 20 GiB
  - `Email (EML)` (.eml) — header `46 72 6F 6D 20`, max 10 MiB

**`crates/ferrite-carver/src/lib.rs`**
- Updated `builtin_signatures_parse` test assertion from `18` to `21`.

---

## 12B — ext4 Double/Triple-Indirect Block Support

**`crates/ferrite-filesystem/src/ext4.rs`**
- Added private helper `collect_indirect_blocks(block_num, depth)` that follows
  indirect block chains recursively (depth 1=single, 2=double, 3=triple).
- Updated `list_inode()` legacy block-map path to handle i_block[13] (double-indirect)
  and i_block[14] (triple-indirect) for directory traversal.
- Updated `read_file()` legacy block-map path to handle i_block[13] and i_block[14]
  for file content extraction.
- Added 2 new tests:
  - `double_indirect_file_read` — 12 KiB image, inode 3 uses i_block[13], verifies
    2048 bytes read (1024×'A' + 1024×'B').
  - `triple_indirect_file_read` — 13 KiB image, inode 3 uses i_block[14], verifies
    2048 bytes read (1024×'C' + 1024×'D').

Note: `ext4.rs` is now ~900 lines. The file remains single-responsibility (ext4 parser
+ test fixtures). Splitting is deferred to a dedicated refactor phase.

---

## 12C — Reverse Imaging Option

**`crates/ferrite-imaging/src/config.rs`**
- Added `pub reverse: bool` field to `ImagingConfig` (default: `false`).
- Updated `Default` impl to set `reverse: false`.

**`crates/ferrite-imaging/src/passes/copy.rs`**
- When `engine.config.reverse == true`, the copy pass collects chunk positions per
  region, reverses the list, then reads end→start. Writes still go to the correct
  offset in the output file.
- Existing forward path is unchanged.

**`crates/ferrite-imaging/src/engine.rs`**
- Updated all 4 `ImagingConfig { ... }` struct literals in tests to include
  `reverse: false`.
- Added test `reverse_mode_images_entire_device` — verifies all sectors are Finished
  and content matches source when run with `reverse: true`.

**`crates/ferrite-tui/src/screens/imaging.rs`**
- Added `pub reverse: bool` field to `ImagingState` (default: `false`).
- Added `r` key binding to toggle `reverse` in `handle_key`.
- Added `Reverse: [YES/NO]  (r to toggle)` line in the configuration panel.
- Increased config panel height from 8 to 9 to accommodate the new line.
- `start_imaging()` passes `reverse: self.reverse` to `ImagingConfig`.

**`crates/ferrite-tui/src/session.rs`**
- Added `#[serde(default)] pub imaging_reverse: bool` to `Session`.
- Updated tests constructing `Session` with named fields.

**`crates/ferrite-tui/src/app.rs`**
- Load: `app.imaging.reverse = session.imaging_reverse`.
- Save: `imaging_reverse: self.imaging.reverse`.
- Updated help line for screen 2 to include `r: reverse`.

---

## Test Results

- `cargo fmt --all` — clean
- `cargo clippy --workspace -- -D warnings` — clean (0 warnings)
- `cargo test --workspace` — **178 tests passed, 0 failed**
  - ferrite-blockdev: 15
  - ferrite-carver: 20
  - ferrite-core: 2
  - ferrite-filesystem: 27 (includes 2 new indirect tests)
  - ferrite-imaging: 30 (includes 1 new reverse test)
  - ferrite-partition: 27
  - ferrite-smart: 19
  - ferrite-tui: 38
