# Phase 37 — MKV Hardening + RAW Photo Signatures (ARW, CR2, RW2, RAF)

## Goal

1. **MKV DocType look-ahead** — tighten the Matroska/WebM validator to reject
   any EBML stream whose DocType is not `matroska` or `webm`.
2. **RAW photo carving** — add four new file signatures for Sony ARW, Canon CR2,
   Panasonic RW2, and Fujifilm RAF, each with format-specific pre-validators.

---

## Changes

### `crates/ferrite-carver/src/pre_validate.rs`

#### MKV — DocType look-ahead

`validate_mkv()` rewritten:
- Existing check: EBML VINT leading byte is non-zero (kept).
- New: search the first 80 bytes for the EBML DocType element (ID `0x42 0x82`).
  - If found, decode the VINT length and verify the value is `matroska` or `webm`.
  - If found but value incomplete (chunk boundary) → benefit of doubt.
  - If full 80-byte window searched and no DocType found → rejected.
- **New tests (5):** `mkv_matroska_doctype_accepted`, `mkv_webm_doctype_accepted`,
  `mkv_unknown_doctype_rejected`, `mkv_no_doctype_in_full_window_rejected`,
  `mkv_short_buffer_benefit_of_doubt`.

#### New `PreValidate` variants

Added `Arw`, `Cr2`, `Rw2`, `Raf` to the enum, `kind_name()`, `from_kind()`,
and `is_valid()` dispatch.

#### `validate_arw()` — Sony ARW

- `II\x2A\x00\x08\x00\x00\x00` is the 8-byte magic (TIFF LE + IFD at 8).
- Validator checks IFD entry count in [5, 50].
- Searches for `SONY` within the first 512 bytes — always present in the Make
  IFD value of genuine ARW files; absent in non-Sony TIFF files.
- **New tests (3):** `arw_with_sony_string_accepted`,
  `arw_without_sony_string_rejected`, `arw_implausible_entry_count_rejected`.

#### `validate_cr2()` — Canon CR2

- Magic already includes `CR\x02\x00` at bytes 8–11 (highly distinctive).
- Validator confirms IFD offset at +4 is in [8, 4096].
- **New tests (2):** `cr2_plausible_ifd_offset_accepted`,
  `cr2_zero_ifd_offset_rejected`.

#### `validate_rw2()` — Panasonic RW2

- `II\x55\x00` magic (Panasonic TIFF variant — `0x55` instead of `0x2A`).
- Validator checks IFD offset in [8, 4096] and IFD entry count in [3, 50].
- **New tests (2):** `rw2_valid_accepted`, `rw2_bad_ifd_offset_rejected`.

#### `validate_raf()` — Fujifilm RAF

- `FUJIFILMCCD-RAW` (15 bytes) — one of the most distinctive RAW magics.
- Validator checks: space at offset 15, 4 ASCII digit version string at 16–19.
- **New tests (3):** `raf_valid_accepted`, `raf_missing_space_rejected`,
  `raf_non_digit_version_rejected`.

### `config/signatures.toml`

Four new `[[signature]]` entries:

| Format | Magic | min_size | max_size |
|--------|-------|----------|----------|
| Sony ARW | `49 49 2A 00 08 00 00 00` (8B) | 1 MB | 200 MB |
| Canon CR2 | `49 49 2A 00 ?? ?? ?? ?? 43 52 02 00` (12B) | 1 MB | 120 MB |
| Panasonic RW2 | `49 49 55 00` (4B) | 1 MB | 150 MB |
| Fujifilm RAF | `46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57` (15B) | 1 MB | 300 MB |

### `crates/ferrite-carver/src/lib.rs`

Updated `builtin_signatures_parse` assertion: 27 → 31.

---

## Test Count

| Before | After | Delta |
|--------|-------|-------|
| 78     | 93    | +15   |

All 93 unit tests pass. `cargo clippy --all-targets -D warnings` clean.

---

## Why These Formats

- **MKV**: 4-byte EBML magic is not unique enough — the DocType field within
  the header unambiguously identifies Matroska vs. WebM vs. everything else.
- **ARW**: Primary RAW format for Sony Alpha cameras (A7, A9, ZV, RX series).
  TIFF-based but distinguished by `SONY` string in Make IFD entry.
- **CR2**: Canon RAW format for DSLRs up to 5DS R / 80D era. The `CR\x02\x00`
  marker at bytes 8–11 makes it trivially identifiable.
- **RW2**: Panasonic Lumix RAW format. The `0x55` magic byte instead of TIFF's
  `0x2A` makes it immediately distinct from all other TIFF variants.
- **RAF**: Fujifilm RAW format (X-series, GFX). `FUJIFILMCCD-RAW` is a 15-byte
  magic — essentially immune to false positives.
