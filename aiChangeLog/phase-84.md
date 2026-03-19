# Phase 84 — Enhanced PNG + JPEG corruption detection

## Problem

1. **Corrupt PNGs marked Complete:** On fragmented drives, overwritten sectors
   produce garbage chunk types (e.g. `0xFF6F00AB`) between the last valid IDAT
   and the IEND footer.  The Phase 83 CRC-32 check only covers the first 8 KiB
   (head buffer), so large IDAT chunks hide the corruption.  Files render as
   partial images (top portion visible, rest black) but pass validation.

2. **Corrupt JPEGs marked Complete:** When intermediate sectors are overwritten
   by data from other files, the JPEG entropy (scan) data contains invalid byte
   sequences.  The existing check only verifies the `FF D9` EOI marker at the
   end, so files with valid headers and EOI but corrupt scan data pass as
   Complete.

## Solution

### PNG tail chunk reverse-walk
After confirming IEND is present, walk backward from IEND in the 64 KiB tail
buffer to find the chunk immediately preceding IEND.  For each candidate
`data_len`, compute `chunk_start = iend_start - 12 - data_len` and check
whether `tail[chunk_start..+4]` matches `data_len` as BE u32 and the chunk
type at `tail[chunk_start+4..+8]` is ASCII-alpha.  When a plausible boundary
is found, verify its CRC-32.  A mismatch means sector-level corruption.

### JPEG entropy data scan validation
In valid JPEG scan data, every `0xFF` byte must be followed by:
- `0x00` (byte-stuffed literal FF)
- `0xD0`–`0xD7` (RST restart markers)
- `0xD9` (EOI)
- `0xFF` (fill padding)

Any other follower (`0xC0`, `0xE0`, etc.) indicates data from another file
has overwritten sectors in the entropy-coded region.  The validator scans
the last 4 KiB of data before EOI for invalid marker sequences.

## Changes

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/post_validate/mod.rs` | PNG: reverse-walk from IEND to verify preceding chunk CRC; JPEG: scan last 4 KiB of entropy data for invalid `0xFF` marker sequences |
| `crates/ferrite-carver/src/post_validate/tests.rs` | Split tests to separate file (was 860 lines, now 365+492) |
| `aiChangeLog/phase-84.md` | This file |

## Tests

### Post-validation — JPEG (5 new)
- `jpeg_complete_with_valid_entropy_data` — byte-stuffed FF + normal bytes → Complete
- `jpeg_complete_with_rst_markers` — RST0–RST7 in scan data → Complete
- `jpeg_corrupt_with_invalid_marker_in_scan_data` — FF E0 in scan data → Corrupt
- `jpeg_corrupt_with_ff_c0_in_scan_data` — FF C0 (SOF0) in scan data → Corrupt
- `jpeg_complete_with_ff_fill_bytes` — consecutive FF fill bytes → Complete

### Post-validation — PNG tail (4 new)
- `png_complete_with_valid_tail_chunk` — valid tEXt chunk before IEND → Complete
- `png_corrupt_with_bad_tail_chunk_crc` — chunk found, CRC mismatch → Corrupt
- `png_complete_when_predecessor_exceeds_tail` — preceding chunk larger than tail buffer → Complete (not flagged)
- `png_corrupt_garbage_before_iend_in_tail` — garbage bytes with matching length but bad CRC → Corrupt
