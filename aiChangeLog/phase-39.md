# Phase 39 — TIFF (LE/BE), Nikon NEF, and Apple HEIC Signatures

## Goal

Add five new photo format signatures:
1. **TIFF (LE)** — generic little-endian TIFF covering plain TIFF, DNG, ORF, and
   any TIFF-based format not handled by a dedicated signature.
2. **TIFF (BE)** — big-endian TIFF from scanners, older Mac software, some cameras.
3. **Nikon NEF** — Nikon RAW (TIFF LE with "NIKON" manufacturer string).
4. **Apple HEIC (heic)** — iPhone HEIC photos with `heic` ftyp brand.
5. **Apple HEIC (heix)** — HEIC with extended features, `heix` ftyp brand.

---

## Design: Avoiding Duplicate Extractions

ARW, CR2, NEF, and generic TIFF all share the same 4-byte LE TIFF magic
(`49 49 2A 00`).  To avoid extracting the same file under multiple extensions,
the validators form a strict partition:

| Signature | Condition |
|-----------|-----------|
| `arw`     | "SONY" string present in first 512 bytes |
| `cr2`     | `CR\x02\x00` marker at offset +8 |
| `nef`     | "NIKON" string present in first 512 bytes |
| `tiff_le` | None of the above — catches DNG, ORF, plain TIFF, etc. |

HEIC/HEIX share the `?? ?? ?? ?? ftyp` prefix with MP4.  Both the MP4 and HEIC
signatures will fire on HEIC files, producing a `.mp4` version (valid ISOBMFF
container) and a `.heic` version with the correct extension for photo apps.

---

## Changes

### `crates/ferrite-carver/src/pre_validate.rs`

Four new `PreValidate` variants: `TiffLe`, `TiffBe`, `Nef`, `Heic`.

#### `validate_tiff_le()`
- IFD0 offset (u32 LE @ +4) must be in [8, 65536].
- Reject if `CR\x02\x00` at offset +8 (Canon CR2).
- Reject if "SONY" found in first 512 bytes (Sony ARW).
- Reject if "NIKON" found in first 512 bytes (Nikon NEF).
- IFD entry count at IFD0 must be in [1, 500] (if within chunk).

#### `validate_tiff_be()`
- IFD0 offset (u32 BE @ +4) must be in [8, 65536].
- IFD entry count at IFD0 must be in [1, 500] (if within chunk).

#### `validate_nef()`
- IFD0 offset (u32 LE @ +4) must be in [8, 4096].
- "NIKON" must appear somewhere in the first 512 bytes.

#### `validate_heic()`
- ftyp box size (u32 BE @ pos) must be in [12, 512].
- Requires 8 bytes available before validating (shorter → benefit of doubt).

**New tests (14):** `tiff_le_plain_accepted`, `tiff_le_rejects_sony_arw`,
`tiff_le_rejects_nikon_nef`, `tiff_le_rejects_canon_cr2`,
`tiff_le_bad_ifd_offset_rejected`, `tiff_be_valid_accepted`,
`tiff_be_bad_ifd_offset_rejected`, `nef_with_nikon_string_accepted`,
`nef_without_nikon_string_rejected`, `nef_bad_ifd_offset_rejected`,
`heic_valid_box_size_accepted`, `heic_heix_brand_accepted`,
`heic_too_small_box_rejected`, `heic_oversized_box_rejected`.

---

### `config/signatures.toml`

Five new `[[signature]]` entries (31 → 36 total):

| Format | Magic | min_size | max_size | size_hint |
|--------|-------|----------|----------|-----------|
| TIFF LE | `49 49 2A 00` | — | 1 GiB | tiff |
| TIFF BE | `4D 4D 00 2A` | — | 1 GiB | tiff |
| Nikon NEF | `49 49 2A 00` | 1 MB | 200 MB | tiff |
| HEIC (heic) | `?? ?? ?? ?? ftyp heic` | 16 KB | 100 MB | mp4 |
| HEIC (heix) | `?? ?? ?? ?? ftyp heix` | 16 KB | 100 MB | mp4 |

---

### `crates/ferrite-carver/src/lib.rs`

Updated `builtin_signatures_parse` assertion: 31 → 36.

---

## Test Count

| Before | After | Delta |
|--------|-------|-------|
| 99     | 113   | +14   |

All 113 unit tests pass. `cargo clippy --workspace --all-targets -- -D warnings` clean.
