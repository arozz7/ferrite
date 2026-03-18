# Phase 52 — Tier 2 Signatures (53 → 62)

## Summary
Added 9 new file-type signatures: MIDI, AIFF, XZ, BZip2, RealMedia, ICO, Olympus ORF,
Pentax PEF, and Mach-O 64-bit.  Each has a structural pre-validator that eliminates
false positives at scan time.

## New Signatures

| # | Name | Ext | Magic | Validator |
|---|------|-----|-------|-----------|
| 54 | MIDI Audio | `.mid` | `4D 54 68 64` (MThd) | chunk_len==6, format in {0,1,2} |
| 55 | AIFF Audio | `.aif` | `FORM????AIF?` | byte@11 'F'/'C', size>4 |
| 56 | XZ Compressed | `.xz` | `FD 37 7A 58 5A 00` | reserved==0, check type in {0,1,4,0xA} |
| 57 | BZip2 Compressed | `.bz2` | `42 5A 68 ??` | level '1'-'9', block magic @4-9 |
| 58 | RealMedia | `.rm` | `2E 52 4D 46` (.RMF) | version in {0,1}, hdr_size>=18 |
| 59 | Windows ICO | `.ico` | `00 00 01 00` | image count in [1, 200] |
| 60 | Olympus ORF | `.orf` | `49 49 52 4F` (IIRO) | IFD offset in [8, 4096] |
| 61 | Pentax PEF | `.pef` | `49 49 2A 00` (TIFF LE) | "PENTAX " in first 512 bytes |
| 62 | Mach-O 64-bit | `.macho` | `CF FA ED FE` | filetype in [1,12], ncmds in [1,512] |

## Files Changed

### `config/signatures.toml`
- 9 `[[signature]]` entries appended (54–62).

### `crates/ferrite-carver/src/pre_validate.rs`
- 9 new `PreValidate` enum variants: `Midi`, `Aiff`, `Xz`, `Bzip2`, `RealMedia`,
  `Ico`, `Orf`, `Pef`, `MachO`.
- `kind_name()`, `from_kind()`, `is_valid()` dispatch updated accordingly.
- 9 new validator functions: `validate_midi`, `validate_aiff`, `validate_xz`,
  `validate_bzip2`, `validate_realmedia`, `validate_ico`, `validate_orf`,
  `validate_pef`, `validate_macho`.
- 34 new unit tests (3-5 per validator).

### `crates/ferrite-carver/src/lib.rs`
- Signature count assertion updated: 53 → 62.

### `crates/ferrite-tui/src/screens/carving/helpers.rs`
- `sig_group_label` updated:
  - `mid`, `aif` → "Audio"
  - `xz`, `bz2` → "Archives"
  - `ico` → "Images"
  - `orf`, `pef` → "RAW Photos"
  - `rm` → "Video"
  - `macho` → "System"

### `crates/ferrite-tui/src/screens/carving/mod.rs`
- `groups_cover_all_signatures` test assertion updated: 53 → 62.

## TUI Group Distribution (after Phase 52)
- Images: JPEG×2, PNG, GIF, BMP, TIFF×2, WebP, PSD, **ICO** (10)
- RAW Photos: ARW, CR2, NEF, RW2, RAF, HEIC×2, **ORF, PEF** (9)
- Video: MP4, MOV, M4V, 3GP, AVI, MKV, WebM, WMV, FLV, MPG, **RM** (11)
- Audio: MP3, WAV, FLAC, OGG, M4A, **MIDI, AIFF** (7)
- Archives: ZIP, RAR, 7-Zip, GZip, **XZ, BZip2** (6)
- Documents: PDF, XML, HTML, RTF, VCF, ICS, EML (7)
- Office & Email: ZIP-Office, OLE2, PST (3)
- System: SQLite, EVTX, EXE, ELF, VMDK, REGF, VHD, VHDX, QCOW2, **Mach-O** (10)
