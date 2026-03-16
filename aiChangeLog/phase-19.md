# Phase 19 — OGG Page-Walk Size Hint & OLE2 Max-Size Raise

## Summary
Carving extractions for OGG files always produced files at `max_size` (4 GiB) and OLE2
compound documents were capped at 500 MiB, preventing recovery of large PST/MDB files.
Phase 19 introduces an Ogg page-walker (`SizeHint::OggStream`) that stops precisely at the
end-of-stream (EOS) Ogg page, and raises the OLE2 hard cap to 2 GiB.

## Root Cause
- **OGG**: The carver had no size hint and no footer for Ogg media, so every hit was
  extracted up to `max_size`. Ogg files are self-delimiting — the last page sets the
  end-of-stream flag (header_type bit 2). Walking pages and stopping there gives the
  exact file size without false padding.
- **OLE2**: The 500 MiB cap was set conservatively. Outlook PST files and Access MDB
  databases commonly exceed 500 MiB. The formula `(csect_fat × (sector_size/4) + 1) ×
  sector_size` already computes an accurate upper bound; raising the cap to 2 GiB lets
  those files be recovered.

## Design

### `SizeHint::OggStream`
New variant added to the `SizeHint` enum in `ferrite-carver/src/signature.rs`.

Algorithm in `read_size_hint()` (`scanner.rs`):
1. Read 27-byte Ogg page header at current position.
2. Verify magic `OggS` at bytes 0–3.
3. Read `num_segments` from byte 26, then the segment table (`num_segments` bytes).
4. Compute `page_size = 27 + num_segments + sum(segment_table)`.
5. If `header_type_flag & 0x04` (EOS bit) is set, return `pos - file_offset + page_size`.
6. Advance `pos += page_size` and repeat.
7. Safety cap: bail after 100,000 pages (prevents infinite loops on corrupt data); returns
   `None` so the caller falls back to `max_size`.

### TOML keyword
`size_hint_kind = "ogg_stream"` in `config/signatures.toml`.

## Changes

### `crates/ferrite-carver/src/signature.rs`
- Added `SizeHint::OggStream` variant
- `kind_name()`: added `OggStream => "ogg_stream"` arm
- TOML parser: added `"ogg_stream"` → `Some(SizeHint::OggStream)` arm
- New test: `load_toml_size_hint_ogg_stream`

### `crates/ferrite-carver/src/scanner.rs`
- Added `SizeHint::OggStream` arm in `read_size_hint()`: walks Ogg page headers,
  returns exact size on EOS page, `None` (→ falls back to `max_size`) if no EOS found
  within 100,000 pages
- Added `build_ogg_page()` test helper
- New tests:
  - `extract_ogg_stream_size_hint_stops_at_eos` — 3-page stream (BOS + middle + EOS);
    verifies extractor writes exactly 315 bytes
  - `extract_ogg_stream_no_eos_falls_back_to_max_size` — single BOS page, no EOS;
    verifies extractor falls back to `max_size` (200 bytes in test)

### `config/signatures.toml`
- OGG: added `size_hint_kind = "ogg_stream"`; reduced `max_size` from 4 GiB → 2 GiB
  (fallback cap, rarely reached now that page walker is active)
- OLE2: raised `max_size` from 500 MiB → 2 GiB to recover large PST/MDB files

### `crates/ferrite-carver/src/lib.rs`
- `builtin_signatures_parse` test: added assertions for OGG `SizeHint::OggStream` variant
  and `OggS` header bytes

## Files Modified
- `crates/ferrite-carver/src/signature.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/lib.rs`
- `config/signatures.toml`
- `aiChangeLog/phase-19.md` (this file)

## Test Results
- `cargo test --workspace` — 214 tests pass, 0 failures (3 new tests added)
- `cargo clippy --workspace -- -D warnings` — clean
