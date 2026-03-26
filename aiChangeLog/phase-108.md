# Phase 108 — False-Positive Audit & Validator Hardening

## Summary

Conducted a code-level audit of all 140 signatures to identify those lacking
structural pre-validation. 17 signatures had no `pre_validate` field. After
grouping by FP risk we added validators for the 5 most exposed categories and
confirmed the remainder are already protected by long or highly specific headers.

## Risk Analysis

| Signature(s) | Header length | Assessment | Action |
|---|---|---|---|
| pyc ×7 | 4 bytes (`XX 0D 0D 0A`) | Medium — CRLF-like bytes appear in text | **Added `Pyc` validator** |
| DPX BE + LE | 4 bytes (`SDPX`/`XPDS`) | Medium — ASCII strings in DPX-adjacent data | **Added `Dpx` validator** |
| OpenEXR | 4 bytes (`76 2F 31 01`) | Medium — `v/1\x01` plausible in binary | **Added `Exr` validator** |
| WAV | 12 bytes (`RIFF????WAVE`) | Low — wildcarded subtype already specific | **Added `Wav` validator** (cheap, consistent) |
| AVI | 12 bytes (`RIFF????AVI `) | Low — wildcarded subtype already specific | **Added `Avi` validator** (cheap, consistent) |
| JP2 | 12 bytes (globally unique) | Very low — no action needed | Skipped |
| AFF | 7 bytes | Low — no action needed | Skipped |
| Bitcoin wallet (BDB) | 6 bytes | Low — no action needed | Skipped |
| Parquet | 4 bytes (`PAR1`) | No structural header to validate | Skipped |
| BPG | 4 bytes (`42 50 47 FB`) | Low (`FB` is highly specific) | Skipped |

## New PreValidate Variants (5)

| Variant | Kind string | Rule |
|---|---|---|
| `Wav` | `"wav"` | RIFF chunk_size (u32 LE @4) ≥ 36 |
| `Avi` | `"avi"` | RIFF chunk_size (u32 LE @4) ≥ 12 |
| `Pyc` | `"pyc"` | flags (u32 LE @4) ≤ 3 (only bits 0–1 defined) |
| `Dpx` | `"dpx"` | version @8: `V`, ASCII digit, `.`, `0` (V1.0 or V2.0) |
| `Exr` | `"exr"` | byte @4 == 2; byte @5 upper nibble == 0; bytes @6–7 == 0 |

## Files Changed

- `crates/ferrite-carver/src/pre_validate.rs` — 5 new enum variants, dispatch
  arms, validator functions, 31 new unit tests (wav ×5, avi ×4, pyc ×6, dpx ×5,
  exr ×6 + 5 short-buffer pass-through tests)
- `config/signatures.toml` — `pre_validate` field added to 11 signature entries
  (wav, avi, exr, pyc ×7, dpx ×2)

## Test Count

Before: 1 053 tests
After:  ~1 084 tests (+31)
All tests pass; clippy clean; fmt clean.
