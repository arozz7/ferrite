# Phase 122 — JPEG Segment Walker + ADTS Hardening + Carving Counter Improvements

## Summary

Two independent improvement sets committed together:

1. **Carver quality (ferrite-carver + signatures.toml):** JPEG segment walker size
   hint; ADTS stream-level validation hardening; `skip_on_failure()` to discard
   false-positive hits when a frame-walking hint returns `None`; live-config sig
   override during extraction so post-session config improvements apply on resume.

2. **TUI carving improvements (ferrite-tui):** Remove `DISPLAY_CAP` (all hits
   stored); add `total_hits_scanned` / `hits_extracted_count` counters; SHA-256
   sidecar removed from carving (noisy, large-session overhead); batch checkpoint
   flush (`append_batch`); back-pressure pause logic fix; `auto_follow_extraction`
   scrolls to active hit.

---

## Changes

### `crates/ferrite-carver/src/signature.rs`
- Added `SizeHint::Jpeg` variant — walks JPEG segments past EXIF APP1 (which
  embeds a thumbnail ending with `FF D9`) to find the true End-of-Image marker.
- Added `SizeHint::skip_on_failure() -> bool` — returns `true` for frame-walking
  hints (`Adts`, `OggStream`, etc.) where `None` means false positive, not
  just unknown size.

### `crates/ferrite-carver/src/scanner.rs`
- **Live sig override:** when a `CarveHit` from a checkpoint has `size_hint: None`
  but the current config has a size hint for that signature name, prefer the live
  config version.  Allows size-hint improvements added after a session was started
  to apply retroactively on resume.
- **Skip on failure:** when a frame-walking size hint returns `None`, the hit is
  now skipped (`Ok(0)`) with a trace log rather than falling back to `max_size`
  extraction, which would produce large false-positive files.

### `crates/ferrite-carver/src/size_hint/adts.rs`
- `MIN_FRAMES` raised from 4 → 8 (dramatically reduces false positives on random data).
- `MAX_FRAMES` reduced from 500 000 → 200 000 (sufficient for 50 MiB AAC files).
- Added **sampling_freq_index consistency check**: all frames in a stream must
  share the same sample-rate index; mismatches terminate the walk.
- Added **reserved SFI guard**: `sfi` values 13–15 are invalid per spec; any frame
  reporting these is rejected immediately.
- New tests: `inconsistent_sfi_terminates_walk`, `invalid_sfi_in_first_frame_returns_none`.
- Fixed: `offset_walk_starts_at_file_offset` test updated to 8 frames (was 4,
  inconsistent with new `MIN_FRAMES`).

### `crates/ferrite-carver/src/size_hint/mod.rs`
- Added `jpeg` sub-module for `SizeHint::Jpeg` walker.

### `crates/ferrite-carver/src/lib.rs`
- Updated `builtin_signatures_parse` test to assert JPEG sigs use `SizeHint::Jpeg`
  and have empty footers (segment walker replaces raw footer scan).
- Added `end_to_end_scan_and_extract` integration test for JPEG Exif + PNG.

### `config/signatures.toml`
- **JPEG sigs (×3):** footers cleared; `size_hint_kind = "jpeg"` added; `min_size`
  set to 4096 to skip embedded thumbnails; `max_size` raised to 100 MiB.
- **SWF compressed variant:** added `size_hint_kind = "linear"` using the 4-byte
  decompressed-size field (correct on-disk size for the compressed container).
- Minor comment and whitespace improvements across other entries.

### `crates/ferrite-tui/src/screens/carving/mod.rs`
- Removed `pub(crate) const DISPLAY_CAP: usize = 100_000` — no hit-list cap.
  All hits are stored and displayed.
- Added `total_hits_scanned: usize` and `hits_extracted_count: usize` fields to
  `CarvingState`.
- Added `auto_follow_extraction: bool` field (default `true`).

### `crates/ferrite-tui/src/screens/carving/events.rs`
- Removed `DISPLAY_CAP` gate — all hits in a `HitBatch` are pushed to `self.hits`.
- `total_hits_scanned` updated from `ScanProgress::hits_found` on each progress tick.
- `hits_extracted_count` incremented for `Extracted`, `Duplicate`, `Skipped`,
  `SkippedCorrupt` messages.
- `auto_follow_extraction`: when enabled, `ExtractionStarted` scrolls the hit
  list to the active extraction index.
- Checkpoint flush switched from per-hit `checkpoint::append` to
  `checkpoint::append_batch` (one file-open per 1000-hit batch).

### `crates/ferrite-tui/src/screens/carving/extract.rs`
- Removed SHA-256 sidecar generation (`write_sha256_sidecar`) from carving
  extraction — sidecar files added significant I/O overhead and disk usage during
  large-session carving; the imaging module's `.sha256` sidecar is unaffected.
- Back-pressure pause fix: `pump_auto_extract` now unconditionally calls
  `self.pause.store(false)` when lifting back-pressure; if status had transitioned
  to `Paused`/`Pausing` (user-initiated or thermal) it is restored to `Running`
  and `paused_elapsed` is updated.

### `crates/ferrite-tui/src/screens/carving/render.rs`
- Status bar shows **Found / Extracted / Pending** counters using `total_hits_scanned`
  and `hits_extracted_count`.
- `auto_follow_extraction` hint (`F: follow`) shown in status bar.

---

## Test delta

| Crate | Before | After | New tests |
|---|---|---|---|
| ferrite-carver | 700 | 706 | `inconsistent_sfi_terminates_walk`, `invalid_sfi_in_first_frame_returns_none`, `end_to_end_scan_and_extract` + updated assertions |
| ferrite-tui | 131 | 136 | (checkpoint tests added in fix-122b) |
