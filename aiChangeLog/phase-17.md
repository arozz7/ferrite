# Phase 17 — JPEG Thumbnail Fix & PDF Last-Footer Mode

## Summary
Two extraction-quality issues discovered during real-drive testing:

1. **JPEG thumbnails instead of full photos** — the 3-byte magic `FF D8 FF`
   matched embedded EXIF thumbnails (stored inside the JPEG file itself) as
   well as standalone photos, producing hundreds of tiny 160×160 / 96×96
   extractions.
2. **PDF files unable to open** — `%%EOF` can appear inside binary streams
   embedded in a PDF, causing the extractor to stop too early and produce a
   structurally incomplete file.

## Root Causes

### JPEG thumbnails
Camera JPEGs embed a thumbnail inside the EXIF/APP1 block.  The thumbnail is
itself a valid JPEG and starts with `FF D8 FF`, which our carver found as a
separate hit.  Standalone photo files almost always have `FF D8 FF E0` (JFIF)
or `FF D8 FF E1` (Exif) as their 4-byte prefix.  Embedded thumbnails
typically use `FF D8 FF DB` (quantization table) — a different 4th byte.

### PDF early truncation
PDF binary streams (embedded fonts, images, attachments) can contain the byte
sequence `25 25 45 4F 46` (`%%EOF`) in their payload.  Our first-match footer
logic stopped there, producing a truncated file that PDF readers rejected.
Additionally, PDFs edited and re-saved (incremental updates) contain multiple
`%%EOF` markers and require the last one to be complete.

## Changes

### `config/signatures.toml`
- Replaced the single `"JPEG Image"` entry (`FF D8 FF`) with two entries:
  - `"JPEG Image (JFIF)"` — header `FF D8 FF E0` (standard / internet JPEGs)
  - `"JPEG Image (Exif)"` — header `FF D8 FF E1` (camera photos with metadata)
- `"PDF Document"`: added `footer_last = true` to use last-footer extraction
- Signature count: **27 → 28**

### `crates/ferrite-carver/src/signature.rs`
- `Signature` struct: added `pub footer_last: bool` field
- TOML `RawSig`: added `#[serde(default)] footer_last: bool`
- Parser wires `footer_last` through to `Signature`
- New tests: `load_toml_footer_last_defaults_false`,
  `load_toml_footer_last_explicit_true`

### `crates/ferrite-carver/src/scanner.rs`
- `extract()`: added branch — when `sig.footer_last` is set, calls the new
  `stream_until_last_footer()` instead of `stream_until_footer()`
- New function `stream_until_last_footer()`: reads the full extraction window
  in `EXTRACT_CHUNK` pieces, then uses `memmem::rfind` to locate the last
  footer occurrence and writes exactly up to and including it.  Falls back to
  writing all bytes if no footer found (same as no-footer mode).
- `sig()` test helper: added `footer_last: false`
- All inline `Signature` literals in tests: added `footer_last: false`
- New tests: `extract_footer_last_stops_at_last_occurrence`,
  `extract_footer_last_no_footer_writes_all`

### `crates/ferrite-carver/src/lib.rs`
- `builtin_signatures_parse`: updated count assertion 27 → 28
- Added spot-checks for both JPEG variants (4-byte headers confirmed)
- Added assertion `pdf.footer_last == true`
- `end_to_end_scan_and_extract`: updated embedded JPEG to use `FF D8 FF E1`
  so it matches the new Exif signature

### `crates/ferrite-tui/src/screens/carving.rs`
- Two inline `Signature` test literals updated: added `footer_last: false`

## Files Modified
- `config/signatures.toml`
- `crates/ferrite-carver/src/signature.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/lib.rs`
- `crates/ferrite-tui/src/screens/carving.rs`
- `aiChangeLog/phase-17.md` (this file)

## Test Results
- `cargo test --workspace` — 201 tests pass, 0 failures (4 new tests added)
