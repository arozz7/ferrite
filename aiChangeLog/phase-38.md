# Phase 38 — TIFF IFD Walker + RAF Size Hints for RAW Photos

## Goal

Fix oversized carved RAW photo files (e.g. Sony ARW files extracted at 190 MB
instead of ~22 MB) by implementing proper size-hint extraction for all four RAW
photo formats added in Phase 37.

**Root cause:** ARW, CR2, RW2, and RAF have no footer bytes; without a size hint
the carver falls back to `max_size`, writing up to 200 MB of unrelated data after
the actual file.

---

## Changes

### `crates/ferrite-carver/src/signature.rs`

Added two new `SizeHint` variants:

#### `SizeHint::Tiff`
TIFF IFD chain walker for Sony ARW, Canon CR2, and Panasonic RW2.  Supports
both `II` (little-endian) and `MM` (big-endian) byte orders, and the Panasonic
RW2 variant magic (`0x55` instead of `0x2A`).

#### `SizeHint::Raf`
Fujifilm RAF fixed-offset reader.  Reads two big-endian u32 pairs at known
offsets in the RAF header to determine the file extent.

Updated `kind_name()` and `from_toml_str` parsing for both (`"tiff"`, `"raf"`).

---

### `crates/ferrite-carver/src/size_hint.rs`

#### `tiff_size_hint(device, file_offset) -> Option<u64>`

Walks the full TIFF IFD chain to find the maximum byte extent:

1. Reads the 8-byte TIFF header; determines byte order from `II`/`MM` marker.
2. Maintains a queue of IFD offsets (starting with IFD0) and a visited set
   (capped at 64 IFDs to prevent cycles).
3. For each IFD entry:
   - **External data blocks** (`count × type_size > 4`): updates
     `max_extent = max(max_extent, offset + data_size)`.
   - **SubIFD tag `0x014A`**: pushes referenced IFD offsets onto the queue so
     Sony/Canon raw-sensor IFDs are always visited.
   - **Strip/tile/JPEG tags** (`0x0111`, `0x0117`, `0x0144`, `0x0145`, `0x0201`,
     `0x0202`): reads the offset and length arrays (inline or external, SHORT or
     LONG), then pairs them to compute `offset + length` extents.
4. Follows the next-IFD chain link after the last entry.
5. Returns `max_extent` if > 8, else `None` (fall back to `max_size`).

**Safety caps:**
- Maximum 64 IFDs visited (prevents infinite loops on circular references).
- Maximum 1000 entries per IFD (rejects corrupt data).
- Offset/length array reads capped at 512 KB.

#### `raf_size_hint(device, file_offset) -> Option<u64>`

Reads 16 bytes at `file_offset + 84` (big-endian u32 pairs):

| Field | Offset | Meaning |
|-------|--------|---------|
| `jpeg_off` | +84 | JPEG preview start |
| `jpeg_len` | +88 | JPEG preview length |
| `cfa_off`  | +92 | CFA raw sensor data start |
| `cfa_len`  | +96 | CFA raw sensor data length |

Returns `max(jpeg_off + jpeg_len, cfa_off + cfa_len)`.

---

### `config/signatures.toml`

Added `size_hint_kind` to all four RAW photo signatures:

| Format | `size_hint_kind` |
|--------|-----------------|
| Sony ARW   | `"tiff"` |
| Canon CR2  | `"tiff"` |
| Panasonic RW2 | `"tiff"` |
| Fujifilm RAF  | `"raf"` |

---

## Test Count

| Before | After | Delta |
|--------|-------|-------|
| 93     | 99    | +6    |

New tests:

- `tiff_single_strip_extent` — minimal TIFF with one strip; verifies extent = offset + bytecount.
- `tiff_subifd_extent` — IFD0 → SubIFD → large strip; verifies SubIFD walking.
- `tiff_invalid_byte_order_returns_none` — corrupt header returns `None`.
- `raf_cfa_dominates` — CFA extent larger than JPEG; correct max returned.
- `raf_jpeg_dominates` — JPEG extent larger than CFA; correct max returned.
- `raf_all_zero_returns_none` — all-zero header returns `None`.

All 99 unit tests pass. `cargo clippy --workspace --all-targets -- -D warnings` clean.

---

## Effect on ARW Files

A Sony A7 III ARW file (ILCE-7M3) with ~22 MB of compressed raw sensor data will
now be extracted to approximately 22 MB instead of up to 200 MB.  The TIFF IFD
walker follows the `SubIFD` (tag `0x014A`) chain to the raw-data IFD and reads
the `StripOffsets` + `StripByteCounts` entries that point to the compressed sensor
data block.
