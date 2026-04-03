# Phase 117 — Tier 1 Reliability Fixes (ENH-01 through ENH-05)

**Date:** 2026-03-30
**Branch:** master
**Tests:** 1110 passing, 0 failing — clippy clean — fmt clean

---

## Summary

Five targeted reliability fixes identified during the post-Phase-112 forensic
engineering audit (`docs/enhancement-backlog.md`).  No new features; no API
breakage.  All changes are either additive or harden existing behaviour against
silent failure modes.

---

## Changes

### ENH-01 · Mapfile CRC integrity check
**Files:** `crates/ferrite-imaging/Cargo.toml`, `src/error.rs`, `src/mapfile_io.rs`

- Added `crc32fast` workspace dependency to `ferrite-imaging`.
- `serialize()` now hashes each block-data line (without newline) with CRC32 and
  appends `# ferrite_crc32: XXXXXXXX` as a trailing comment.  GNU ddrescue ignores
  unknown comment lines — format remains fully compatible.
- `parse()` detects the tag, records the declared CRC, hashes block-data lines as
  they are parsed, and validates at the end.  Absence of the tag = no validation
  (backwards-compat with existing mapfiles and GNU ddrescue output).
- New `ImagingError::MapfileChecksum { declared: u32, actual: u32 }` variant with
  a user-facing message directing the operator to delete or rename the file.
- **3 new tests:** CRC tag present after serialise; corrupted block line detected;
  pre-CRC mapfile loads cleanly without error.

---

### ENH-02 · NTFS MFT scan cap raised to 1 M records
**File:** `crates/ferrite-filesystem/src/ntfs.rs`

- `MAX_SCAN_RECORDS` raised from `65_536` to `1_048_576`.
  Previous cap silently truncated file enumeration on volumes with more than 65 K
  MFT records (approximately any NTFS volume > 512 GiB with default cluster size).
- Added `tracing::warn!` with `cap` and `volume_records` fields when the raw MFT
  record count exceeds the cap, so large-volume scans are visibly flagged in logs.

---

### ENH-03 · Sparse output — surface filesystem incapability
**Files:** `crates/ferrite-imaging/src/sparse.rs`, `src/engine.rs`;
           `crates/ferrite-tui/src/screens/imaging/mod.rs`, `render.rs`

- `enable_sparse(file: &File)` return type changed from `io::Result<()>` to
  `io::Result<bool>`:
  - Windows: `Ok(true)` when `FSCTL_SET_SPARSE` succeeds; `Ok(false)` when the
    destination filesystem declines (FAT32, exFAT, SMB shares, etc.).
  - Linux/macOS: always `Ok(true)` — holes are created implicitly on seek-past.
- `ImagingEngine` gains `sparse_active: bool` field (set during `new()`) and a
  `pub fn sparse_active(&self) -> bool` accessor.
- New `ImagingMsg::SparseStatus(bool)` sent from the background imaging thread
  immediately after engine creation.
- `ImagingState` gains `sparse_active: Option<bool>` (`None` = not started;
  reset to `None` when a new session begins).
- **TUI Sparse row now shows:**
  - No annotation while idle (pre-start).
  - `(active)` in green once confirmed.
  - `(unavailable — destination FS does not support sparse files; dense output)`
    in amber when sparse was requested but the OS declined — prevents operator
    surprise when imaging to a FAT32 USB stick.

---

### ENH-04 · HFS+ actionable TUI message in file browser
**File:** `crates/ferrite-tui/src/screens/file_browser.rs`

- `start_open()` now checks `self.fs_type == FilesystemType::HfsPlus` immediately
  after `detect_filesystem()` and before spawning the background open thread.
- If true, sets `BrowserStatus::Error` with the message:
  `HFS+ detected — parser not yet implemented. Use the Carving tab (Tab 5) to recover files by signature.`
- Eliminates the confusing generic "unknown filesystem" error and directs the
  operator to the working recovery path.

---

### ENH-05 · FAT32 cluster index defensive bounds check
**File:** `crates/ferrite-filesystem/src/fat32.rs`

- `read_cluster()` now returns `Err(FilesystemError::InvalidStructure)` when
  called with `cluster < 2` (FAT-reserved range).  Prevents the
  `cluster as u64 - 2` subtraction from wrapping to a huge LBA on corrupted
  volumes.
- `cluster_offset()` gains `debug_assert!(cluster >= 2)` for build-time
  validation in debug/test profiles.
- **1 new test:** `read_cluster_rejects_reserved_indices` — verifies that cluster
  indices 0 and 1 return `Err`.

---

### Bonus: pre-existing clippy lint fixed
**File:** `crates/ferrite-tui/src/screens/carving/extract.rs`

- `.filter(…).last()` → `.rfind(…)` on a double-ended iterator (Clippy
  `double_ended_iterator_last` + `filter_next` lints).  No behaviour change.

---

## Enhancement Backlog Updates

Items marked `done` in `docs/enhancement-backlog.md`:
- ENH-01 ✅
- ENH-02 ✅
- ENH-03 ✅
- ENH-04 ✅
- ENH-05 ✅

Remaining open items: ENH-06 through ENH-18.
