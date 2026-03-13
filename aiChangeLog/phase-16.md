# Phase 16 — Extended Size Hints: LinearScaled, Sqlite, SevenZip

## Summary
Completed the size-hint audit started in phase 14 (RIFF/BMP) and continued in
phase 15 (OLE2).  Three additional file formats were found to embed a readable
file-size field in their headers; each now has a dedicated `SizeHint` variant
that prevents gross over-extraction.

| Format | Old cap | Typical real size | New behaviour |
|--------|---------|-------------------|---------------|
| SQLite | 10 GiB  | 1 KiB – 100 MiB   | `page_size × db_pages` (two BE fields) |
| EVTX   | 100 MiB | 1 – 50 MiB        | `chunk_count × 65536 + 4096` |
| 7-Zip  | 500 MiB | varies widely     | `32 + NextHeaderOffset + NextHeaderSize` |

## New SizeHint Variants

### `LinearScaled { offset, len, little_endian, scale, add }`
`total_size = parse(data[offset..offset+len]) × scale + add`

Used by **Windows Event Log (EVTX)**:
- `chunk_count` (u16 LE at offset 42)
- Each chunk is exactly 65 536 bytes; the file header is 4 096 bytes
- `total = chunk_count × 65536 + 4096`

### `Sqlite`
Reads two big-endian header fields:
- `page_size` (u16 BE at offset 16): value `1` encodes 65 536
- `db_pages`  (u32 BE at offset 28): 0 means not written (pre-3.7.0 fallback)
- `total = page_size × db_pages`

### `SevenZip`
Reads two u64 LE fields from the 32-byte start header:
- `NextHeaderOffset` (offset 12): bytes from end-of-start-header to encoded header
- `NextHeaderSize`   (offset 20): byte length of the encoded header
- `total = 32 + NextHeaderOffset + NextHeaderSize`

## Changes

### `crates/ferrite-carver/src/signature.rs`
- Added `LinearScaled { offset, len, little_endian, scale, add }` variant to `SizeHint`
- Added `Sqlite` variant
- Added `SevenZip` variant
- Added `impl SizeHint::kind_name()` helper
- TOML parser (`RawSig`): added `size_hint_scale: Option<u64>` field
- TOML parser: `size_hint_kind` now also matches `"linear_scaled"`, `"sqlite"`,
  `"seven_zip"` in addition to the existing `"ole2"`
- New tests: `load_toml_size_hint_sqlite`, `load_toml_size_hint_seven_zip`,
  `load_toml_size_hint_linear_scaled`

### `crates/ferrite-carver/src/scanner.rs`
- `read_size_hint()`: added `LinearScaled`, `Sqlite`, and `SevenZip` match arms
  using saturating arithmetic throughout
- New tests:
  - `extract_linear_scaled_size_hint_limits_output` — EVTX with 3 chunks →
    extraction stops at 200 704 bytes
  - `extract_sqlite_size_hint_limits_output` — 4 096-byte pages × 5 pages →
    stops at 20 480 bytes
  - `extract_seven_zip_size_hint_limits_output` — NextHeaderOffset=1000,
    NextHeaderSize=200 → stops at 1 232 bytes

### `config/signatures.toml`
- SQLite: added `size_hint_kind = "sqlite"` with explanatory comment
- 7-Zip: added `size_hint_kind = "seven_zip"` with explanatory comment
- EVTX: added `size_hint_kind = "linear_scaled"` with offset/len/endian/scale/add

### `crates/ferrite-carver/src/lib.rs`
- Added assertions for SQLite, 7-Zip, and EVTX size hints in
  `builtin_signatures_parse`

## Files Modified
- `crates/ferrite-carver/src/signature.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/lib.rs`
- `config/signatures.toml`
- `aiChangeLog/phase-16.md` (this file)

## Test Results
- `cargo test --workspace` — 197 tests pass, 0 failures (6 new tests added)
