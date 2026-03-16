# Phase 14 — RIFF Signature Fix: Wildcard Headers & Embedded Size Hints

## Summary
Fixed a critical carving bug where AVI and WAV files were extracted at 4 GiB each regardless
of their true size, filling the output drive with gigabytes of near-empty files.  Root cause
was two independent defects in the signature engine: identical 4-byte RIFF magic shared by
both formats (no subtype discrimination), and no mechanism to read the file size embedded in
the RIFF header.  Fixed by adding `??` wildcard bytes to the header pattern language and a
`size_hint` field that reads the actual length from a declared offset.

## Root Cause Analysis

### Defect 1 — AVI and WAV matched the same header
Both formats begin with the 4-byte sequence `52 49 46 46` ("RIFF").  The subtype discriminator
("AVI " or "WAVE") lives at bytes 8–11, but bytes 4–7 are a variable-length size field and
could not be expressed with the previous exact-match-only pattern language.  Every RIFF hit
therefore matched both signatures, producing duplicate hits and mis-labeled files.

### Defect 2 — Extractor ignored the embedded file size
RIFF stores the payload length as a `u32` little-endian integer at bytes 4–7; total file size
= `value + 8`.  Without reading this field the extractor fell back to `max_size` (4 GiB),
writing 4 GiB of device data per hit even when the actual AVI was a few megabytes.

## Changes

### `crates/ferrite-carver/src/signature.rs`
- `Signature.header` changed from `Vec<u8>` to `Vec<Option<u8>>`:
  - `Some(b)` = exact byte match
  - `None`    = wildcard (`??` in TOML) — matches any byte at that position
- Added `SizeHint` struct: `{ offset, len, little_endian, add }`
  - `offset` — byte offset within the file where the size field starts
  - `len`    — field width: 2, 4, or 8 bytes
  - `little_endian` — byte order
  - `add`    — constant added to the parsed value (RIFF needs +8)
- Added `size_hint: Option<SizeHint>` to `Signature`
- Added `parse_hex_pattern()` — parses `??` tokens as `None`, hex bytes as `Some(b)`
- TOML loader wires the new optional fields: `size_hint_offset`, `size_hint_len`,
  `size_hint_endian`, `size_hint_add`
- New tests: `parse_hex_pattern_wildcards`, `parse_hex_pattern_no_wildcards`,
  `load_toml_size_hint`

### `crates/ferrite-carver/src/scanner.rs`
- `find_all()` updated to wildcard-aware matching:
  - Finds the first fixed (non-`None`) byte in the header for the `memchr` anchor
  - After a candidate position is found, calls `header_matches()` which skips `None`
    positions and checks `Some(b)` positions exactly
- Added `header_matches(header, data, pos)` helper
- Added `read_size_hint(device, file_offset, hint)` — reads `hint.len` bytes at
  `file_offset + hint.offset` and returns `parsed_value + hint.add`
- `extract()` now calls `read_size_hint` when `sig.size_hint.is_some()`, using the
  result (clamped to `max_size`) as the extraction length instead of `max_size`
- New tests:
  - `scan_wildcard_header_matches_riff_subtypes` — verifies AVI and WAV are found
    separately at the correct offsets
  - `extract_size_hint_limits_output` — verifies a RIFF file with payload=100 is
    extracted as exactly 108 bytes, not 2 GiB

### `config/signatures.toml`
- **WAV**: header extended from `52 49 46 46` → `52 49 46 46 ?? ?? ?? ?? 57 41 56 45`
  (`RIFF????WAVE`); added `size_hint_offset=4, size_hint_len=4, size_hint_endian=le,
  size_hint_add=8`; `max_size` kept at 2 GiB as a hard cap
- **AVI**: header extended from `52 49 46 46` → `52 49 46 46 ?? ?? ?? ?? 41 56 49 20`
  (`RIFF????AVI `); same size hint as WAV; `max_size` reduced from 4 GiB → 2 GiB
- **BMP**: added `size_hint_offset=2, size_hint_len=4, size_hint_endian=le, size_hint_add=0`
  — BMP stores total file size at offset 2, so extraction is now exact rather than always
  writing the 50 MiB cap

### `crates/ferrite-carver/src/lib.rs`
- Updated `builtin_signatures_parse` assertions for the new `Vec<Option<u8>>` header type
- Added assertions that AVI and WAV carry wildcard bytes at position 4 and have a `size_hint`

### `crates/ferrite-tui/src/screens/carving.rs`
- Updated two test `Signature` literals to use `Vec<Option<u8>>` headers and include
  the new `size_hint: None` field

## Files Modified
- `config/signatures.toml`
- `crates/ferrite-carver/src/signature.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/lib.rs`
- `crates/ferrite-tui/src/screens/carving.rs`

## Test Results
- `cargo test --workspace` — 184 tests pass, 0 failures (5 new tests added)
- Manual observation: AVI hits on a 20 GiB drive were producing 4 GiB extractions;
  after fix they extract to the correct size read from the RIFF header
