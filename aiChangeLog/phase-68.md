# Phase 68 — Pre-Validator Hardening (SWF, GZip, TS, M2TS)

## Problem

The carver produced large numbers of false-positive hits for formats with
short or weak magic bytes:

- **SWF** (3-byte ASCII: "FWS"/"CWS"/"ZWS") — validator only checked
  version ∈ [1,50] and file_len ≥ 8.  A random u32 LE at bytes 4-7 is
  almost always ≥ 8, so the validator passed ~20% of random data.

- **GZip** (2-byte magic: `1F 8B`) — validator checked only CM=8 and one
  flag bit, effectively validating 1 byte.  `1F 8B 08` appears inside
  compressed video/audio data by coincidence.

- **M2TS** (4 wildcards + `47`) — only 2 independent sync-byte checks
  (offsets 196, 388).  On a TB drive, probability (1/256)² ≈ 1/65K per
  candidate position produced hundreds of false positives.

- **TS** (single-byte `47`) — same issue as M2TS.

## Solution

### SWF (`validate_swf`) — false-positive rejection: ~97.7% → ~99.9%

| Check | Old | New | Rejection rate |
|-------|-----|-----|----------------|
| Version | [1, 50] | [3, 45] | 17% → 83% |
| file_len | ≥ 8 | [21, 100 MiB] | ~0% → **~97.7%** |
| FWS Nbits | — | byte 8 top-5-bits ∈ [1, 25] | — → ~22% |
| CWS zlib CMF | — | (byte8 & 0x0F) == 8, byte8 >> 4 ≤ 7 | — → ~94% |

### GZip (`validate_gz`) — false-positive rejection: ~99.9%

| Check | Old | New |
|-------|-----|-----|
| CM @2 | == 8 | == 8 (unchanged) |
| FLG @3 | bit 5 == 0 | bit 5 == 0 (unchanged) |
| XFL @8 | — | ∈ {0, 2, 4} (~1.2% random-byte pass rate) |
| OS @9 | — | ∈ [0, 13] ∪ {255} (~5.9% random-byte pass rate) |

Combined XFL × OS: 0.069% of random byte-pairs pass.

### TS / M2TS — 5-sync-byte validation

| Format | Old checks | New checks |
|--------|-----------|------------|
| TS | 3 sync bytes (0, 188, 376) | 3 required + 2 optional (564, 752) |
| M2TS | 3 sync bytes (4, 196, 388) | 3 required + 2 optional (580, 772) |

The 2 extra checks are **conditional**: applied only when the scan buffer has
≥ 753 (TS) or ≥ 773 (M2TS) bytes from the hit position.  This avoids
rejecting legitimate hits near chunk boundaries while eliminating virtually
all false positives on full-buffer positions.

Per-position false-positive probability:
- Old: (1/256)² ≈ 1/65,536
- New: (1/256)⁴ ≈ 1/4,294,967,296

## Files Changed

- `crates/ferrite-carver/src/pre_validate.rs` — tightened `validate_swf`,
  `validate_gz`, `validate_ts`, `validate_m2ts`; 13 new/updated unit tests

## Tests

- 355 ferrite-carver tests pass (13 new).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
