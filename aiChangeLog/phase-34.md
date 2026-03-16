# Phase 34 — Scan-Time Pre-Extraction Integrity Validation

## Goal
Reduce the number of carved files that fail to open by validating format-specific structural
constraints at scan time — before a candidate hit is ever recorded as a `CarveHit`.  False
positives that were previously extracted (wasting I/O and producing corrupt output files) are
now silently discarded during the scan pass.

---

## New Module: `crates/ferrite-carver/src/pre_validate.rs`

Introduced the `PreValidate` enum (25 variants, one per supported format) and the
`is_valid(kind, data, pos)` dispatch function.  Each validator:

- Returns **`true`** (accept) when there are not enough bytes to be sure — the magic bytes
  already matched, so the scan gets benefit of the doubt.
- Returns **`false`** only when the header bytes are **definitively wrong**.

| Variant | Check |
|---------|-------|
| `Zip` | Version ≤ 63, filename len in [1, 512], method is known, filename does not end with `/` (rejects directory entries) |
| `JpegJfif` | `JFIF\0` at offset 6 |
| `JpegExif` | `Exif` at offset 6 |
| `Png` | First chunk length == 13 and type == `IHDR` |
| `Pdf` | `-1.x` or `-2.x` version string at offset 4 |
| `Gif` | Byte 4 is `7` or `9`; byte 5 is `a` (GIF87a / GIF89a) |
| `Bmp` | DIB header size @14 ∈ {12, 40, 52, 56, 108, 124} |
| `Mp3` | ID3 version ∈ {2,3,4}; flags low nibble zero; syncsafe size bytes |
| `Mp4` | ftyp box size ∈ [12, 512]; brand bytes printable ASCII |
| `Rar` | Type byte @6 ∈ {0x00 RAR4, 0x01 RAR5} |
| `SevenZip` | Major version byte @6 == 0x00 |
| `Sqlite` | Page size (u16 BE @16) is power-of-2 ∈ [512, 65536] (value 1 = 65536) |
| `Mkv` | EBML VINT leading byte @4 non-zero |
| `Flac` | First metadata block type (lower 7 bits @4) == 0 (STREAMINFO) |
| `Exe` | `e_lfanew` (u32 LE @60) ∈ [64, 16384] |
| `Vmdk` | Version field (u32 LE @4) ∈ {1, 2, 3} |
| `Ogg` | Stream-structure version @4 == 0 **and** BOS flag (bit 1 @5) set |
| `Evtx` | MajorVersion (u16 LE @38) == 3 |
| `Pst` | `wMagicClient` bytes @8-9 == `[0x4D, 0x53]` |
| `Xml` | Byte 5 (after `<?xml`) is space |
| `Html` | `<!DOCTYPE` followed by ` html` or ` HTML` |
| `Rtf` | Byte 6 (after `{\rtf1`) is `\`, space, CR, or LF |
| `Vcard` | `BEGIN:VCARD` followed by CR or LF |
| `Ical` | `BEGIN:VCALENDAR` followed by CR or LF |
| `Ole2` | ByteOrder (u16 LE @28) == 0xFFFE |

---

## Modified: `crates/ferrite-carver/src/signature.rs`

- Replaced `pub pre_validate_zip: bool` with `pub pre_validate: Option<PreValidate>`.
- TOML parser reads optional `pre_validate = "<kind>"` field and calls
  `PreValidate::from_kind(s)`.

## Modified: `config/signatures.toml`

Added `pre_validate = "<kind>"` to all 24 applicable signatures.

## Modified: `crates/ferrite-carver/src/scan_search.rs`

- Removed the now-redundant `zip_local_header_is_file` function.
- Updated `find_all()` dispatch to call `pre_validate::is_valid(kind, data, pos)` via the
  new `Option<PreValidate>` field, using `.is_none_or(...)`.

## Modified: `crates/ferrite-carver/src/lib.rs`

- Added `mod pre_validate`.
- Added `pub use pre_validate::PreValidate` to the crate public API.
- Fixed end-to-end test to include valid `Exif` and `IHDR` bytes required by the new validators.

## Cascade: `pre_validate_zip: false` → `pre_validate: None`

Updated all test `Signature` struct literals across:
- `crates/ferrite-carver/src/carver_io.rs`
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-carver/src/size_hint.rs`
- `crates/ferrite-carver/tests/size_hints.rs`
- `crates/ferrite-tui/src/screens/carving/mod.rs`

---

## Test results

```
cargo test --workspace   → 267 passed, 0 failed
cargo clippy -- -D warnings → clean
cargo fmt --check        → clean
```
