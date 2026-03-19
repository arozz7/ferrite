# Phase 80 — False-positive elimination: TIFF, TTF, DICOM, M2TS zero-byte cleanup

## Summary
Eliminates false positives and empty files from real-world 4TB carving:
embedded Exif TIFF headers, TrueType magic inside JPEG data, coincidental
DICOM "DICM" markers, and 0-byte M2TS files left behind by min_size enforcement.

## Changes

### 1. TIFF min_size — `config/signatures.toml`
- Added `min_size = 4096` to both TIFF LE and TIFF BE signatures.
- Exif metadata headers inside JPEGs appear exactly 12 bytes after the JPEG
  header and are only 78–82 bytes — well below the threshold.

### 2. TTF size hint — new `SizeHint::Ttf` variant
- **`size_hint/ttf.rs`** — walks the TrueType table directory, returns
  `max(table_offset + table_length)` across all table records.
- **`signature.rs`** — added `Ttf` variant + TOML parser.
- **`size_hint/mod.rs`** — dispatch arm.
- **`config/signatures.toml`** — TTF signature now uses `size_hint_kind = "ttf"`
  and `min_size = 4096`. False-positive `00 01 00 00` magic inside JPEG Exif
  data resolves to a tiny size from garbage table entries → skipped by min_size.

### 3. TTF pre_validator hardening — `pre_validate.rs`
- Now validates `searchRange` and `entrySelector` consistency with `numTables`.
- `searchRange` must equal `(1 << entrySelector) * 16`.
- `entrySelector` must equal `floor(log2(numTables))`.
- False positives with coincidental `00 01 00 00 00` bytes fail these checks.

### 4. DICOM pre_validator hardening — `pre_validate.rs`
- Now validates the first data element after "DICM" has group `0x0002`
  (File Meta Information) and a valid VR (two uppercase ASCII letters).
- Previously only checked that 8 bytes existed → passed random binary data.
- **`config/signatures.toml`** — reduced `max_size` from 2 GiB to 500 MiB.

### 5. Zero-byte file cleanup — `extract.rs`
- Both single-file and batch extraction paths now check for `Ok(0)` return
  from `extract()` and delete the empty file + send `Skipped` message.
- Previously, min_size enforcement returned 0 bytes but the file was already
  created by `File::create()`, leaving 0-byte files on disk.

### 6. Helper: `read_u32_be` — `size_hint/helpers.rs`
- Added big-endian u32 reader for TTF table directory parsing.

### 7. Tests
- 3 new TTF size_hint tests (two tables, single table, no tables).
- 2 new DICOM pre_validate tests (wrong group, invalid VR).
- Updated `make_ttf` test helper to include full 12-byte header with
  correct searchRange/entrySelector/rangeShift.
- Updated DICOM test data to include group 0x0002 + valid VR.
- **760 tests passing**, clippy clean, fmt clean.

## Files Modified
- `config/signatures.toml` — TIFF min_size, TTF size_hint + min_size, DICOM max_size
- `crates/ferrite-carver/src/signature.rs` — SizeHint::Ttf variant
- `crates/ferrite-carver/src/size_hint/mod.rs` — Ttf dispatch
- `crates/ferrite-carver/src/size_hint/ttf.rs` — **NEW** TrueType table directory walker
- `crates/ferrite-carver/src/size_hint/helpers.rs` — read_u32_be
- `crates/ferrite-carver/src/size_hint/tests.rs` — 3 new TTF tests
- `crates/ferrite-carver/src/pre_validate.rs` — TTF + DICOM hardening + test fixes
- `crates/ferrite-tui/src/screens/carving/extract.rs` — 0-byte file cleanup
