# Phase 94 — ISO 9660 Size Hint

## Problem
The ISO 9660 signature had no size hint. Without one the extractor falls back
to `max_size = 9_395_240_960` (8.75 GiB), writing gigabytes of trailing garbage
from whatever data follows the ISO on disk. Windows' virtual DVD driver reads the
Primary Volume Descriptor (PVD), finds e.g. a 700 MiB volume, but the file on
disk is 8.75 GiB — the mismatch causes the mount to fail (Windows reports the
image as "in use" or "invalid").

## Fix — `SizeHint::Iso9660`

### New file
- `crates/ferrite-carver/src/size_hint/iso9660.rs`
  - `iso9660_hint(device, file_offset) -> Option<u64>`
  - Reads the PVD at sector 16 (file offset 32 768)
  - `volume_space_size` — u32 LE at PVD+80 (total logical blocks)
  - `logical_block_size` — u16 LE at PVD+128 (typically 2048)
  - Returns `volume_space_size × logical_block_size`
  - Sanity checks: block size must be a power of 2 in [512, 32768];
    volume block count must be nonzero

### Modified files
| File | Change |
|------|--------|
| `crates/ferrite-carver/src/signature.rs` | Added `SizeHint::Iso9660` variant with doc-comment; `kind_name()` → `"iso9660"`; TOML parser arm `"iso9660"` |
| `crates/ferrite-carver/src/size_hint/mod.rs` | Added `mod iso9660;`; dispatch arm `SizeHint::Iso9660 => iso9660::iso9660_hint(...)` |
| `config/signatures.toml` | Added `size_hint_kind = "iso9660"` to the ISO 9660 Disc Image signature |

## Tests (4 new)
- `iso9660_standard_2048_block` — 350 000 blocks × 2048 = 716 800 000 bytes
- `iso9660_zero_volume_blocks_returns_none`
- `iso9660_bad_block_size_returns_none` — block size 999 not a power of two
- `iso9660_nonzero_file_offset` — image carved from a non-zero device offset
