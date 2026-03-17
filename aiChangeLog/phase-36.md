# Phase 36 — Pre-validator Hardening: False-Positive Reduction

## Goal

Eliminate duplicate / invalid carved files caused by false `CarveHit`s.
Three rounds of targeted fixes were applied to `pre_validate.rs`:

1. **JPEG duplicate thumbnails** — embedded JPEG thumbnails inside EXIF data
   triggered extra hits with identical content to the outer photo.
2. **MP4 false hits from mdat content** — `ftyp` bytes appearing inside encoded
   H.264/H.265 video streams produced hundreds of invalid "MP4 files".
3. **General validator hardening** — three more validators tightened to reduce
   false positives on common consumer-drive data.

---

## Changes

### `crates/ferrite-carver/src/pre_validate.rs`

#### JPEG (JpegJfif + JpegExif)

- Added `jpeg_is_embedded(data, pos)` helper: scans backward from `pos` for a
  JPEG SOI marker (`FF D8`) with no intervening EOI (`FF D9`). If found, the
  hit is a thumbnail nested inside an outer JPEG — rejected.
- Added `jpeg_has_eoi(data)` helper: fast `FF D9` search via `memchr`.
- Both `validate_jpeg_jfif()` and `validate_jpeg_exif()` now call
  `jpeg_is_embedded()` and return `false` when embedded.
- **New tests (4):** `jpeg_jfif_standalone_accepted`,
  `jpeg_jfif_embedded_thumbnail_rejected`, `jpeg_jfif_after_eoi_accepted`,
  `jpeg_exif_embedded_thumbnail_rejected`.

#### MP4 / ISOBMFF

- Added look-ahead in `validate_mp4()`: after validating the ftyp box
  (size ∈ [12,512], printable ASCII brand), reads the next box header.
  - Rejects if next box size < 8 (invalid).
  - Rejects if next box type contains non-alphanumeric/non-space bytes
    (filters H.264 NAL unit start codes and other video bitstream bytes).
  - Skips look-ahead (benefit of doubt) when ftyp is near the end of the
    scan chunk.
- **New tests (5):** `mp4_valid_ftyp_followed_by_moov_accepted`,
  `mp4_valid_ftyp_followed_by_mdat_accepted`,
  `mp4_ftyp_followed_by_garbage_rejected`,
  `mp4_ftyp_followed_by_tiny_next_box_rejected`,
  `mp4_ftyp_no_lookahead_data_accepted`.

#### EXE / PE (Windows Executables)

- `validate_exe()`: after the `e_lfanew` range check [64,16384], added
  look-ahead to verify `PE\x00\x00` at `pos + e_lfanew`.
- A false hit requires random bytes to both satisfy the range check AND place
  `PE\x00\x00` at a specific variable offset — essentially impossible.
- Skips look-ahead when the PE header falls outside the scan chunk.
- **New tests (3):** `exe_valid_pe_signature_accepted`,
  `exe_missing_pe_signature_rejected`, `exe_mz_in_binary_data_rejected`.

#### BMP

- `validate_bmp()`: added two additional field checks before the DIB size:
  - `FileSize` (u32 LE at +2) must be ≥ 26 (smallest possible valid BMP).
  - `PixelDataOffset` (u32 LE at +10) must be ≥ 14 and ≤ `FileSize`.
  All three values must be self-consistent — random binary data rarely
  achieves this.
- **New tests (5):** `bmp_valid_accepted`, `bmp_tiny_file_size_rejected`,
  `bmp_pixel_offset_past_file_size_rejected`,
  `bmp_pixel_offset_before_header_rejected`, `bmp_unknown_dib_size_rejected`.

#### EVTX (Windows Event Logs)

- `validate_evtx()`: added `HeaderSize == 128` check at offset 32.
  This is a fixed constant in every EVTX file; combined with the existing
  `MajorVersion == 3` check, two independent constants must both match.
- **New tests (3):** `evtx_valid_header_accepted`,
  `evtx_wrong_header_size_rejected`, `evtx_wrong_major_version_rejected`.

---

## Test Count

| Before | After | Delta |
|--------|-------|-------|
| 55     | 78    | +23   |

All 78 unit tests pass. `cargo clippy --all-targets -D warnings` clean.

---

## Root-Cause Summary

| File type | Root cause | Fix strategy |
|-----------|-----------|--------------|
| JPEG | Thumbnail embedded in EXIF APP1 has identical SOI magic | Backward scan: SOI with no preceding EOI → reject |
| MP4 | `ftyp` bytes inside H.264/H.265 mdat stream | Look-ahead: next box must have alphanumeric type and size ≥ 8 |
| EXE | `MZ` is a 2-byte magic; appears throughout binary data | Look-ahead: `PE\x00\x00` at the variable `e_lfanew` offset |
| BMP | `BM` is a 2-byte magic | Cross-check FileSize and PixelDataOffset for consistency |
| EVTX | 8-byte magic; only version checked | Add fixed-constant HeaderSize == 128 at offset 32 |
