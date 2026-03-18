# Phase 40 — Missing Video Format Signatures

## Problem

The carver was missing several common video container formats:
- MOV (QuickTime / iPhone recordings) — extracted as `.mp4` due to shared ISOBMFF magic
- WebM — extracted as `.mkv` due to shared EBML magic
- WMV/ASF, FLV, M4V, 3GP/3G2, and MPEG-PS were not carved at all

## Solution

Added 7 new signatures (36 → 43 total) and updated existing validators to
enforce clean partitioning — no duplicate output files.

### New Signatures

| Name | Extension | Magic | Size Hint | Pre-Validate |
|------|-----------|-------|-----------|--------------|
| QuickTime MOV | `.mov` | `?? ?? ?? ?? ftyp qt  ` (12 B) | `mp4` | `mov` |
| iTunes M4V | `.m4v` | `?? ?? ?? ?? ftyp M4V ` (12 B) | `mp4` | `m4v` |
| 3GPP / 3GPP2 | `.3gp` | `?? ?? ?? ?? ftyp 3g` (10 B) | `mp4` | `3gp` |
| WebM | `.webm` | `1A 45 DF A3` | none | `webm` |
| Windows Media / ASF | `.wmv` | 16-byte ASF Header GUID | none | `wmv` |
| Flash Video | `.flv` | `46 4C 56 01` | none | `flv` |
| MPEG-2/1 Program Stream | `.mpg` | `00 00 01 BA` | none | `mpeg` |

### Updated Validators

- **`validate_mp4`** — now rejects `qt  `, `M4V `, and `3gp*`/`3g2*` brands;
  MOV/M4V/3GP files no longer produce duplicate `.mp4` output.
- **`validate_mkv`** — now accepts only `"matroska"` DocType (was `"matroska"
  || "webm"`); WebM files no longer produce duplicate `.mkv` output.

### New Pre-Validators

- `validate_mov` — checks ftyp box size in [12, 512] (brand already anchored by 12-byte magic)
- `validate_m4v` — same pattern as MOV
- `validate_3gp` — checks full brand starts with `"3gp"` or `"3g2"`; box size in [12, 512]
- `validate_webm` — EBML DocType look-ahead, accepts only `"webm"`
- `validate_wmv` — ASF object size (u64 LE @16) must be ≥ 30 bytes
- `validate_flv` — reserved type-flag bits must be zero; DataOffset (u32 BE @5) must equal 9
- `validate_mpeg` — MPEG-2: top 2 bits of pack byte == `01`; MPEG-1: top 4 bits == `0010`

## Files Changed

- `config/signatures.toml` — 7 new `[[signature]]` entries; MP4 comment updated
- `crates/ferrite-carver/src/pre_validate.rs` — 7 new enum variants + `kind_name` / `from_kind` / `is_valid` arms; 7 new validator functions; `validate_mp4` brand rejection; `validate_mkv` accepts matroska only; 20 new tests (133 total); `mkv_webm_doctype_accepted` renamed `mkv_webm_doctype_rejected`
- `crates/ferrite-carver/src/lib.rs` — assertion updated 36 → 43

## Test Results

- 133 unit tests in ferrite-carver (was 113, +20)
- All workspace tests pass, clippy clean, fmt checked
