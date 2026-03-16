# Phase 15 — OLE2 Size Hint & SizeHint Enum Refactor

## Summary
Extended the size-hint mechanism introduced in phase 14 to cover OLE2 Compound Files
(legacy DOC / XLS / PPT).  OLE2 file size cannot be expressed as a simple `value + add`
formula — it requires reading two header fields and multiplying them.  `SizeHint` was
refactored from a struct to an enum to accommodate this without polluting the `Linear`
variant with unused fields.

## Root Cause
OLE2 had `max_size = 500 MiB` and no size hint, so every DOC/XLS/PPT hit was extracted
as 500 MiB regardless of actual file size — the same problem that affected AVI/WAV before
phase 14.

## OLE2 Size Calculation
The OLE2 Compound File Binary Format stores two relevant fields in its 512-byte header:

| Field       | Offset | Type   | Meaning                                     |
|-------------|--------|--------|---------------------------------------------|
| uSectorShift | 30    | u16 LE | `sector_size = 1 << uSectorShift` (9 → 512, 12 → 4096) |
| csectFat    | 44     | u32 LE | Number of FAT sectors                        |

Each FAT sector can reference `sector_size / 4` data sectors, so:

```
max_addressable_sectors = csectFat × (sector_size / 4)
total_file_size         = (max_addressable_sectors + 1) × sector_size
```

The `+1` accounts for the header sector.  This is a tight upper bound: it equals the
actual file size when all addressable sectors are occupied, and is still far smaller than
the 500 MiB hard cap for any realistic document.

**Example:** A small Word doc with `uSectorShift=9`, `csectFat=1`:
- `sector_size = 512`
- `addressable = 1 × 128 = 128 sectors`
- `total = 129 × 512 = 66,048 bytes` (~64 KiB) ✓

## Changes

### `crates/ferrite-carver/src/signature.rs`
- `SizeHint` refactored from `struct` → `enum`:
  - `SizeHint::Linear { offset, len, little_endian, add }` — existing behaviour;
    reads a fixed-width integer and adds a constant (used by RIFF/BMP)
  - `SizeHint::Ole2` — new variant; reads `uSectorShift` + `csectFat` from the
    standard OLE2 header offsets and computes the size bound described above
- TOML parser: `size_hint_kind = "ole2"` selects `SizeHint::Ole2`; existing
  `size_hint_offset` / `size_hint_len` / `size_hint_endian` / `size_hint_add` fields
  still produce `SizeHint::Linear`
- `SizeHint` added to public re-exports in `lib.rs`
- New tests: `load_toml_size_hint_linear`, `load_toml_size_hint_ole2`

### `crates/ferrite-carver/src/scanner.rs`
- `read_size_hint()` updated to `match` on the enum:
  - `Linear` arm: unchanged logic
  - `Ole2` arm: reads `uSectorShift` at +30, `csectFat` at +44; sanity-checks
    `sector_shift` in range 7–16; computes `(csect_fat × (sector_size/4) + 1) × sector_size`
    using saturating arithmetic throughout
- New test: `extract_ole2_size_hint_limits_output` — builds a fake OLE2 header with
  `uSectorShift=9`, `csectFat=2`; verifies extraction stops at
  `(2×128+1)×512 = 131,584 bytes` instead of 500 MiB

### `config/signatures.toml`
- OLE2 entry gains `size_hint_kind = "ole2"` with an explanatory comment

### `crates/ferrite-carver/src/lib.rs`
- Added assertion in `builtin_signatures_parse` that the `ole` signature carries
  `SizeHint::Ole2`

### `crates/ferrite-tui/src/screens/carving.rs`
- Two test `Signature` literals updated: `SizeHint { ... }` → `SizeHint::Linear { ... }`
  (no functional change; required by enum refactor)

## Files Modified
- `crates/ferrite-carver/src/signature.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/lib.rs`
- `config/signatures.toml`

## Test Results
- `cargo test --workspace` — 187 tests pass, 0 failures (3 new tests added)
