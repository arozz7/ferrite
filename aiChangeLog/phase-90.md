# Phase 90 — TIFF False-Positive Elimination + Extraction Back-Pressure Fix

## Problems

### 1. Spurious `.tif` files from JPEG EXIF blocks
Every JPEG with EXIF data embeds a TIFF structure (starting with `II 2A 00`) inside
its APP1 segment.  The carver was finding this magic and extracting the EXIF block
as a standalone `.tif` file.  The result was a valid TIFF container holding only
metadata (Make, Model, DateTime, ExifIFD pointer) and an embedded JPEG thumbnail —
no primary image data, no pixel dimensions.  These files opened in no viewer.

### 2. Scanner resumed between extraction batches
When auto-extract was active, the scanner paused when a batch started (correct) but
resumed as soon as each batch *finished*, even if hundreds of items remained in the
queue.  This caused brief scan windows between batches, defeating the intent of
back-pressure and generating excess I/O on the output drive between writes.

## Solutions

### TIFF EXIF-block rejection
Added ImageWidth (tag 256) presence check to both `validate_tiff_le` and
`validate_tiff_be`.  After reading the IFD0 entry count, if all entries fit within
the available pre-validation window, the validator walks them and rejects the hit if
no entry has tag 256.  EXIF blocks embedded in JPEGs never carry ImageWidth in IFD0,
so they are eliminated at scan time — no file is written.  If the IFD extends beyond
the available chunk, the validator passes through (safe default — deferred check).

### Scan fully paused until queue drains
Moved back-pressure resume logic exclusively into `pump_auto_extract`:
- **Before:** `ExtractionDone` handler lifted the scan pause unconditionally after
  each batch, then `pump_auto_extract` immediately started the next batch — giving
  the scanner a brief unsuppressed window between every batch.
- **After:** `ExtractionDone` no longer touches the pause flag.  `pump_auto_extract`
  only resumes the scanner when `auto_extract_queue.is_empty()` — i.e., the entire
  queue has drained and no new batch needs to start.  The scanner stays paused for
  the full duration of extraction.

Removed `AUTO_EXTRACT_LOW_WATER` constant (now unused).

## Files Changed

### `crates/ferrite-carver/src/pre_validate.rs`
- `validate_tiff_le`: added IFD0 tag-walk; reject if ImageWidth (256) absent
- `validate_tiff_be`: same check for big-endian TIFF
- `make_tiff_le_header` (test helper): write a minimal tag-256 IFD entry so existing
  tests still pass with the stricter validator
- `tiff_be_valid_accepted`: added tag-256 entry to the BE test buffer
- New tests: `tiff_le_rejects_exif_only_ifd0`, `tiff_le_accepts_when_ifd_beyond_chunk`

### `crates/ferrite-tui/src/screens/carving/events.rs`
- `ExtractionDone` handler: removed unconditional back-pressure lift; added comment
  pointing to `pump_auto_extract` as the sole resume site

### `crates/ferrite-tui/src/screens/carving/extract.rs`
- `pump_auto_extract`: replaced low-water threshold resume with queue-empty resume;
  scanner only unpauses when `auto_extract_queue.is_empty()` after a batch completes
- Removed `AUTO_EXTRACT_LOW_WATER` from imports

### `crates/ferrite-tui/src/screens/carving/mod.rs`
- Removed `AUTO_EXTRACT_LOW_WATER` constant (dead after extract.rs change)
- Updated `backpressure_paused` field doc-comment

## Test Results
- 2 new unit tests — all passing
- Total workspace tests: 880 (was 878) — all passing
- `cargo clippy --workspace -- -D warnings` — clean
