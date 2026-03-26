# Phase 101 — Consumer Format Signatures (107 → 115)

**Date:** 2026-03-24
**Branch:** master
**Status:** Complete

## Summary

Added 8 new file-carving signatures for common consumer formats: Raw AAC audio
(two ADTS variants), DjVu documents, OpenEXR HDR images, GIMP XCF canvases,
JPEG 2000, PCX images, and BPG images.  Signature count: **107 → 115**.

## New Signatures

| # | Name | Header | Pre-validate | Max size |
|---|------|--------|--------------|----------|
| 108 | Raw AAC MPEG-4 ADTS | `FF F1` | `aac` | 50 MiB |
| 109 | Raw AAC MPEG-2 ADTS | `FF F9` | `aac` | 50 MiB |
| 110 | DjVu Document | `AT&TFORM` (8 B) | `djvu` | 200 MiB |
| 111 | OpenEXR HDR Image | `76 2F 31 01` | — (unique) | 500 MiB |
| 112 | GIMP XCF Image | `gimp xcf v` (10 B) | `xcf` | 500 MiB |
| 113 | JPEG 2000 | 12-byte sig box | — (unique) | 500 MiB |
| 114 | PCX Image | `0A` | `pcx` (strict) | 50 MiB |
| 115 | BPG Image | `42 50 47 FB` | — (unique) | 50 MiB |

## New Pre-validators

| Variant | Logic |
|---------|-------|
| `Aac` | layer bits (b1 & 0x06) == 0; sampling_freq_index (b2>>2 & 0x0F) ≤ 12 |
| `Djvu` | form type @12–15 ∈ {`DJVU`, `DJVM`, `DJVI`, `THUM`} |
| `Xcf` | version @10–14 == `file\0` or 3 ASCII digits + `\0` |
| `Pcx` | version ∈ {0,2,3,4,5}; encoding ∈ {0,1}; bpp ∈ {1,2,4,8}; xMax≥xMin; yMax≥yMin; reserved@64==0; planes ∈ {1,3,4}; bpl>0 |

## TUI Group Assignments

- `aac` → **Audio**
- `djvu` → **Documents**
- `exr`, `xcf`, `jp2`, `pcx`, `bpg` → **Images**

## Files Changed

| File | Change |
|------|--------|
| `config/signatures.toml` | +8 `[[signature]]` entries |
| `crates/ferrite-carver/src/pre_validate.rs` | +4 enum variants, +4 validators, +26 unit tests |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | sig_group_label: added aac/djvu/exr/xcf/jp2/pcx/bpg |
| `crates/ferrite-carver/src/lib.rs` | assertion 107 → 115 |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | assertion 107 → 115 |
| `aiChangeLog/phase-101.md` | This file |

## Test Results

- **587 tests** in ferrite-carver (up from 561; +26 new validator tests)
- All workspace tests pass; clippy clean with `-D warnings`
