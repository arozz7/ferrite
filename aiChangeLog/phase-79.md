# Phase 79 — Carver extraction quality fixes

## Summary
Fixes five carving quality issues discovered during real-world 4TB drive recovery:
false-positive M2TS extraction, XML files padded with binary junk, JPEG thumbnail
noise, and ZIP internal-entry false positives.

## Changes

### 1. Enforce min_size in extract() — `scanner.rs`
- After resolving `extraction_size` from a size hint, check that it meets the
  signature's `min_size` threshold. If not, skip extraction (return 0).
- Fixes M2TS false positives where `mpeg_ts_size_hint` returns 192 bytes (1 packet)
  but `min_size` is 389.

### 2. TextBound size hint — new `SizeHint::TextBound` variant
- **`size_hint/text_bound.rs`** — scans forward byte-by-byte; stops at null bytes
  or 8+ consecutive non-text bytes. Text = printable ASCII + whitespace + UTF-8 ≥0x80.
- **`signature.rs`** — added `TextBound` variant to `SizeHint` enum + TOML parser.
- **`size_hint/mod.rs`** — dispatch arm for `TextBound`.
- **`config/signatures.toml`** — XML signature now uses `size_hint_kind = "text_bound"`,
  so embedded XMP metadata `<?xml` headers no longer drag in trailing binary data.

### 3. JPEG min_size — `config/signatures.toml`
- Added `min_size = 4096` to both JPEG (JFIF) and JPEG (Exif) signatures.
- Filters out embedded Exif/TIFF thumbnails (typically 1–5 KB).

### 4. ZIP EOCD central directory offset validation — `post_validate.rs`
- `validate_zip_eocd` now accepts `file_size` and checks the EOCD's central
  directory offset (u32 LE at EOCD+16) falls within the extracted file.
- ZIPs carved from internal `PK\x03\x04` entries (where the EOCD belongs to a
  larger parent archive) are now correctly flagged as `Corrupt`.
- ZIP64 (`cd_offset == 0xFFFFFFFF`) is treated as Complete (can't easily validate).
- `validate_extracted` signature updated: added `file_size: u64` parameter.
- Both TUI callers updated to pass extracted byte count.

### 5. Tests
- 6 new tests for `TextBound` size hint (pure text, null stop, binary stop,
  isolated non-text tolerance, empty device, max_size cap).
- 2 new ZIP post_validate tests (CD offset beyond file = Corrupt, within file = Complete).
- All existing post_validate tests updated for new `file_size` parameter.
- **755 tests passing**, clippy clean, fmt clean.

## Files Modified
- `crates/ferrite-carver/src/scanner.rs` — min_size enforcement in extract()
- `crates/ferrite-carver/src/signature.rs` — TextBound variant + TOML parser
- `crates/ferrite-carver/src/size_hint/mod.rs` — TextBound dispatch
- `crates/ferrite-carver/src/size_hint/text_bound.rs` — **NEW** text boundary scanner
- `crates/ferrite-carver/src/size_hint/tests.rs` — 6 new TextBound tests
- `crates/ferrite-carver/src/post_validate.rs` — ZIP CD offset check + file_size param
- `crates/ferrite-tui/src/screens/carving/extract.rs` — pass file_size to post_validate
- `config/signatures.toml` — XML text_bound hint, JPEG min_size
