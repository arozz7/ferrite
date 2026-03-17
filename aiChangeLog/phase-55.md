# Phase 55 — Carve Hit Integrity Validation + Duplicate Suppression

## Summary
Every extracted file is now tagged with a structural integrity label, and
content-identical hits are deduplicated before extraction.  Together these
eliminate silent failures from fragmented files and remove thumbnail/cache
copies that bloat recovery output.

## A. Post-Extraction Integrity Validation

After each file is extracted, the last ≤ 65 536 bytes of the output file are
read back and checked against format-specific structural rules:

| Tag        | Colour | Meaning                                               |
|------------|--------|-------------------------------------------------------|
| `✓`        | Green  | End-of-file marker found; structural check passed     |
| `~`        | Yellow | Extraction hit the max-size cap (footer not found)    |
| `✗`        | Red    | Bytes present but structural check failed (corrupt)   |
| *(blank)*  | —      | No deep check for this format                         |

Formats with deep structural checks:

| Ext        | Rule                                                 |
|------------|------------------------------------------------------|
| `jpg`      | Last 2 bytes == `FF D9` (End-of-Image)               |
| `png`      | Last 12 bytes == IEND chunk + CRC                    |
| `gif`      | Last byte == `3B` (GIF trailer)                      |
| `pdf`      | `%%EOF` present within last 1 KiB                    |
| `zip`, `ole`, `7z`, `pst` | `PK\x05\x06` (EOCD) present in tail |

All other formats receive `Unknown` (blank tag).

## B. Duplicate Suppression

Before extracting each hit, the worker thread reads the first 4 KiB of device
data at `hit.byte_offset` and hashes it with `DefaultHasher` → `u64`.

- If the fingerprint has already been seen this session → **skip extraction**,
  send `CarveMsg::Duplicate`, show `[DUP]` (dark-gray) in the hit list.
- If new → insert into `seen_fingerprints` set and proceed.

The `seen_fingerprints: Arc<Mutex<HashSet<u64>>>` is shared between `CarvingState`
and the extraction worker thread.  It is reset on every new scan and on
`set_device`.

Duplicate count is shown in the extraction summary as `⊘ N skipped`.

## Files Changed

### `crates/ferrite-carver/src/post_validate.rs` (NEW)
- `pub enum CarveQuality { Complete, Truncated, Corrupt, Unknown }`
- `pub fn validate_extracted(ext, tail, is_truncated) -> CarveQuality`
- Format validators: `validate_jpeg`, `validate_png`, `validate_gif`,
  `validate_pdf`, `validate_zip_eocd`
- 19 unit tests (3-4 per format + truncated flag + unknown)

### `crates/ferrite-carver/src/lib.rs`
- Added `pub mod post_validate; pub use post_validate::CarveQuality;`

### `crates/ferrite-tui/src/screens/carving/mod.rs`
- `HitStatus::Duplicate` variant added.
- `HitEntry.quality: Option<CarveQuality>` field added.
- `CarveMsg::Extracted` gains `quality: CarveQuality` field.
- `CarveMsg::Duplicate { idx }` variant added.
- `CarveMsg::ExtractionDone` gains `duplicates: usize` field.
- `ExtractionSummary.duplicates: usize` field added.
- `CarvingState` gains `seen_fingerprints: Arc<Mutex<HashSet<u64>>>` and
  `duplicates_suppressed: usize`; both reset in `set_device()`.

### `crates/ferrite-tui/src/screens/carving/input.rs`
- `start_scan()` resets `seen_fingerprints` and `duplicates_suppressed`.

### `crates/ferrite-tui/src/screens/carving/extract.rs`
- Added `read_file_tail(path, max_bytes)` helper — reads last N bytes of file.
- Added `hit_fingerprint(device, byte_offset) -> Option<u64>` helper — hashes
  first 4 KiB with `DefaultHasher`.
- `extract_selected`: duplicate check before spawning extraction thread;
  quality check after extraction.
- `start_extraction_batch`: worker thread performs dedup + quality; coordinator
  counts duplicates; `ExtractionDone` includes `duplicates`.

### `crates/ferrite-tui/src/screens/carving/events.rs`
- `CarveMsg::Extracted` handler: stores `quality` on `HitEntry`.
- `CarveMsg::Duplicate` handler: sets `HitStatus::Duplicate`, increments
  `duplicates_suppressed`.
- `CarveMsg::ExtractionDone` handler: passes `duplicates` through to
  `ExtractionSummary`; accumulates in auto-extract mode.

### `crates/ferrite-tui/src/screens/carving/render.rs`
- Hit list: `HitStatus::Duplicate` renders as `[DUP]` (dark-gray).
- Hit list: quality tag `✓`/`✗`/`~` appended after status span.
- Extraction summary: `⊘ N skipped` span added when `duplicates > 0`.

### `crates/ferrite-tui/src/screens/carving/session_ops.rs`
- `load_checkpoint` initialises `quality: None` on restored `HitEntry`s.

## Test Count: 439 → 458 (+19)
