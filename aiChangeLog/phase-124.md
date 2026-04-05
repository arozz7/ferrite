# Phase 124 — Bulk max_size Reduction & Validator Hardening

## Problem Statement

An 80 GB disk image was producing 808 GB of carved output because:
1. The AAC ADTS size-hint was never loaded (wrong TOML key `size_hint` vs `size_hint_kind`).
2. WMV hit the 4 GiB `max_size` fallback for every corrupt/garbage ASF header
   (94 hits × 4 GiB ≈ 364 GiB alone).
3. Dozens of other signatures had max_size values orders of magnitude larger than any
   real-world file of that type, inflating the output on false positives or partial hits.

## Changes

### 1. AAC TOML key fix (`config/signatures.toml`)

Both `"Raw AAC Audio (MPEG-4 ADTS)"` and `"Raw AAC Audio (MPEG-2 ADTS)"` entries used
`size_hint = "adts"` (wrong key — the Rust loader reads `size_hint_kind`).  The ADTS
frame-walker was silently ignored, bypassing the `is_viable_hit()` pre-flight gate added
in Phase 123 and allowing every false positive to create a file.

**Fix:** `size_hint = "adts"` → `size_hint_kind = "adts"` on both entries.

### 2. FLV improvement (`config/signatures.toml`)

- `max_size` reduced from 500 MiB → 50 MiB.
- Added `min_hit_gap = 1_048_576` (1 MiB) to suppress intra-stream re-hits.

### 3. WAV and AVI RIFF linear size hint (`config/signatures.toml`)

Added four RIFF size-hint fields to WAV and AVI:
```toml
size_hint_offset = 4
size_hint_len    = 4
size_hint_endian = "le"
size_hint_add    = 8
```
The RIFF format stores the total body size as a u32 LE at offset 4; adding 8
(for the 8-byte FORM header) gives the exact file size.  `max_size` also reduced
2 GiB → 200 MiB for both.

### 4. Bulk max_size reductions (`config/signatures.toml`)

| Format | Old max_size | New max_size | Ratio |
|--------|-------------|-------------|-------|
| WMV | 4 GiB | 500 MiB | 8× |
| PCX | 50 MiB | 1 MiB | 50× |
| AIFF | 2 GiB | 200 MiB | 10× |
| FLAC | 2 GiB | 200 MiB | 10× |
| PSD | 2 GiB | 200 MiB | 10× |
| PSB | 2 GiB | 200 MiB | 10× |
| WavPack | 2 GiB | 200 MiB | 10× |
| VHD | 64 GiB | 500 MiB | 128× |
| VHDX | 64 GiB | 500 MiB | 128× |
| QCOW2 | 64 GiB | 500 MiB | 128× |
| VMDK | 10 GiB | 500 MiB | 20× |
| PST/OST | 20 GiB | 500 MiB | 40× |
| WTV | ~4 GiB | 500 MiB | 8× |
| XZ | 2 GiB | 200 MiB | 10× |
| BZip2 | 2 GiB | 200 MiB | 10× |
| RealMedia | 2 GiB | 500 MiB | 4× |
| LUKS | 2 GiB | 500 MiB | 4× |
| E01 | 2 GiB | 500 MiB | 4× |
| PCAP LE | 2 GiB | 200 MiB | 10× |
| PCAP BE | 2 GiB | 200 MiB | 10× |
| Blender | 2 GiB | 200 MiB | 10× |
| InDesign | 2 GiB | 200 MiB | 10× |
| DPX BE | 2 GiB | 200 MiB | 10× |
| DPX LE | 2 GiB | 200 MiB | 10× |
| VDI | 2 GiB | 500 MiB | 4× |
| AFF | 2 GiB | 500 MiB | 4× |
| HDF5 | 2 GiB | 500 MiB | 4× |
| FITS | 2 GiB | 500 MiB | 4× |
| Parquet | 2 GiB | 500 MiB | 4× |
| KDBX | 512 MiB | 10 MiB | 51× |
| KDB | 512 MiB | 10 MiB | 51× |
| Minidump | 512 MiB | 64 MiB | 8× |
| Apple plist | 100 MiB | 10 MiB | 10× |
| pyc ×7 | 100 MiB each | 1 MiB each | 100× |
| WOFF | 50 MiB | 10 MiB | 5× |
| APE | 500 MiB | 200 MiB | 2.5× |
| CDR | 500 MiB | 100 MiB | 5× |
| DjVu | 200 MiB | 50 MiB | 4× |
| XCF | 500 MiB | 100 MiB | 5× |
| OpenEXR | 500 MiB | 200 MiB | 2.5× |
| JPEG 2000 | 500 MiB | 50 MiB | 10× |
| JAR | 500 MiB | 100 MiB | 5× |
| EVT (legacy) | 100 MiB | 20 MiB | 5× |
| PAR2 | 1 GiB | 100 MiB | 10× |
| BPG | 50 MiB | 5 MiB | 10× |

### 5. Strengthened `validate_wmv` (`crates/ferrite-carver/src/pre_validate.rs`)

Previous implementation only checked `object_size >= 30`.  New implementation:
- **object_size**: must be in `[30, 10 MiB]` — rejects corrupt/garbage headers
- **Reserved bytes @24-25**: must be exactly `0x01 0x02` (ASF spec requirement)
- **num_headers @26-29** (u32 LE): must be in `[1, 1023]` — zero or absurd counts rejected

6 new tests: `wmv_oversized_object_rejected`, `wmv_wrong_reserved_bytes_rejected`,
`wmv_zero_num_headers_rejected`, `wmv_excessive_num_headers_rejected`, plus updates to
existing `make_asf_header` helper to set all fields.

### 6. Strengthened `validate_aiff` (`crates/ferrite-carver/src/pre_validate.rs`)

Previous implementation had no upper bound on `form_size`.  New range: `[5, 209_715_192]`
(form_size + 8 = total file size ≤ 200 MiB, matching the reduced max_size).

2 new tests: `aiff_oversized_form_rejected`, `aiff_tiny_form_size_rejected`.

## Files Changed

| File | Change |
|------|--------|
| `config/signatures.toml` | AAC key fix, FLV + WAV + AVI + 44 other max_size changes |
| `crates/ferrite-carver/src/pre_validate.rs` | Hardened `validate_wmv` + `validate_aiff`; 8 new tests |
| `scripts/fix_signatures.py` | Script used to apply bulk TOML changes |

## Test Results

- `cargo test --workspace`: **1135 passed, 0 failed** (718 in ferrite-carver)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all`: formatted
