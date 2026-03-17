# Phase 62 — Non-Zero Offset Scan Infrastructure (ISO, DICOM, TAR — 3 new signatures, 94 → 97)

## Summary
Added `header_offset: u64` to the `Signature` struct, enabling the scanner to detect
file formats whose magic bytes appear at a non-zero offset within the file (ISO 9660 at
32769, DICOM at 128, TAR at 257). The scanner now shifts the reported `CarveHit.byte_offset`
back by `header_offset` so extraction always begins at the true file start. Three new
signatures were added using this infrastructure.

## Infrastructure Change

### `ferrite_carver::Signature` — new `header_offset: u64` field

```
pub header_offset: u64   // default 0; "magic appears at this offset within the file"
```

- `#[serde(default)]` on both the public field and the `RawSig` TOML-side field → no
  existing TOML entries need updating; missing `header_offset` defaults to 0.
- All existing struct literals in tests and production code updated with `header_offset: 0`.

### `scan_search::find_all()` — non-zero-offset emission

When the scanner finds magic at absolute position `magic_abs`:
```
byte_offset = magic_abs - sig.header_offset
```
Hits where `magic_abs < sig.header_offset` (file start would precede device start) are
silently skipped.

## New Signatures

| Format | Extension | Magic | Offset | Pre-validator |
|--------|-----------|-------|--------|---------------|
| ISO 9660 | `iso` | `43 44 30 30 31` ("CD001") | 32769 | `Iso` — version byte @5 == 1 |
| DICOM | `dcm` | `44 49 43 4D` ("DICM") | 128 | `Dicom` — ≥ 8 bytes at magic position |
| TAR (ustar) | `tar` | `75 73 74 61 72 00` ("ustar\0") | 257 | `Tar` — version @6 in {"00", "  "} |

## Modified Files

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/signature.rs` | `Signature`: +`header_offset: u64`; `RawSig`: +`header_offset: u64 (default)`; TOML builder passes it through; +2 tests |
| `crates/ferrite-carver/src/scan_search.rs` | `find_all()`: emit `byte_offset = magic_abs - header_offset`; skip if file precedes device start; +3 tests for offset infrastructure |
| `crates/ferrite-carver/src/pre_validate.rs` | +3 enum variants (`Iso`, `Dicom`, `Tar`), +3 validators, +8 unit tests |
| `config/signatures.toml` | +3 `[[signature]]` entries with `header_offset` field |
| `crates/ferrite-carver/src/lib.rs` | Assertion updated: 94 → 97 |
| `crates/ferrite-carver/src/scanner.rs` | All test `Signature` literals: +`header_offset: 0` |
| `crates/ferrite-carver/src/carver_io.rs` | All test `Signature` literals: +`header_offset: 0` |
| `crates/ferrite-carver/src/size_hint.rs` | Test `Signature` literal: +`header_offset: 0` |
| `crates/ferrite-carver/tests/size_hints.rs` | All `Signature` literals: +`header_offset: 0` |
| `crates/ferrite-tui/src/screens/carving/user_sigs.rs` | `to_signature()`: +`header_offset: 0` |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | Test literal: +`header_offset: 0`; assertion: 94 → 97 |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | `sig_group_label`: iso/tar→Archives, dcm→System |

## Design Decisions

- **`#[serde(default)]` on both struct and TOML sides**: Adding the field with `default`
  means zero changes to the 94 existing TOML entries — backward compatible.
- **Skip before-device-start hits**: When `header_offset > 0` and the magic is found near
  the beginning of a scan chunk (before the offset can be subtracted), the hit is silently
  dropped. This is correct because we cannot extract bytes that precede the device start.
  In practice, this only affects the very first chunk and only for files that start before
  byte 0 of the device, which is physically impossible for valid files.
- **DICOM routed to System group**: Medical imaging files are specialized system/forensic
  assets — more akin to EVTX/VMDK/LUKS than to user documents.
- **TAR "ustar\0" vs GNU "ustar  "**: The TOML magic includes the null byte, so only
  POSIX-compliant ustar archives match. The validator also accepts the `"  "` version field
  (GNU tar with ustar indicator) to cover both variants. Old V7 TAR (no magic at 257) is
  not detectable by magic-based carving.

## Test Count
- 620 tests total (up from 608) — 8 new pre_validate tests + 3 scan_search infrastructure
  tests + 2 signature.rs TOML tests
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo test --workspace` — all passing
