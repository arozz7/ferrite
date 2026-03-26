# Phase 104 — Size-Hint Walkers for CAB, DEX, WOFF2, DjVu

**Date:** 2026-03-25
**Branch:** master
**Status:** Complete

## Summary

Added `SizeHint::Linear` entries to four existing signatures so the carver
extracts the correct number of bytes instead of falling back to `max_size`.
No new signatures — this is a quality improvement for Phase 100–101 additions.

| Format | Size field | Endian | Add | Before | After |
|--------|-----------|--------|-----|--------|-------|
| CAB | `cbCabinet` u32 @8 | LE | 0 | 512 MiB cap | exact |
| DEX | `file_size` u32 @32 | LE | 0 | 100 MiB cap | exact |
| WOFF2 | `length` u32 @8 | BE | 0 | 50 MiB cap | exact |
| DjVu | IFF chunk_size u32 @8 | BE | +12 | 200 MiB cap | exact |

## Changes

### `config/signatures.toml`
Four signature entries updated with `size_hint_offset` / `size_hint_len` /
`size_hint_endian` / `size_hint_add` fields (all map to existing
`SizeHint::Linear` via the TOML fallback path — no new Rust enum variants
required).

### `crates/ferrite-carver/src/size_hint/tests.rs`
Added 8 new unit tests (2 per format):
- `cab_size_hint_reads_cbcabinet`, `cab_size_hint_zero_returns_zero`
- `dex_size_hint_reads_file_size`, `dex_size_hint_large_value`
- `woff2_size_hint_reads_length`, `woff2_size_hint_big_endian_bytes`
- `djvu_size_hint_single_page`, `djvu_size_hint_multi_page`

## Test Results

- **637 tests** in ferrite-carver (up from 629; +8 new size-hint tests)
- All workspace tests pass; clippy clean with `-D warnings`
