# Phase 59 — PhotoRec Quick-Win Batch (11 new signatures, 62 → 73)

## Summary
Added 11 new file format signatures across 10 format families, expanding the carving
database from 62 to 73 signatures. All changes are in `ferrite-carver` (pre-validators
+ TOML) and `ferrite-tui` (group routing). No new crates.

## New Signatures

| Format | Extension | Magic | Pre-validator |
|--------|-----------|-------|---------------|
| Canon CR3 | `cr3` | `?? ?? ?? ?? 66 74 79 70 63 72 78 20` | `Cr3` — ftyp box size [12,512] |
| Sony SR2 | `sr2` | `49 49 2A 00 08 00 00 00` | `Sr2` — "SONY" + tag 0x7200 in IFD0 |
| EPUB e-Book | `epub` | `50 4B 03 04` | `Epub` — mimetype fname + epub+zip content |
| OpenDocument (ODT/ODS/ODP) | `odt` | `50 4B 03 04` | `Odt` — mimetype fname + opendocument content |
| Outlook MSG | `msg` | `D0 CF 11 E0 A1 B1 1A E1` | `Msg` — OLE2 ByteOrder + `__substg1.0_` in first 4 KiB |
| WavPack Audio | `wv` | `77 76 70 6B` | `WavPack` — ck_size > 0, version in [0x0402,0x0410] |
| CorelDRAW CDR | `cdr` | `52 49 46 46 ?? ?? ?? ?? 43 44 52 ??` | `Cdr` — version suffix byte valid |
| SWF (FWS — uncompressed) | `swf` | `46 57 53` | `Swf` — version [1,50], length ≥ 8 |
| SWF (CWS — zlib) | `swf` | `43 57 53` | `Swf` — same validator |
| SWF (ZWS — LZMA) | `swf` | `5A 57 53` | `Swf` — same validator |
| Kodak DCR | `dcr` | `49 49 2A 00` | `Dcr` — "Kodak"/"KODAK" in first 512B; rejects other TIFF RAWs |

## Modified Files

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/pre_validate.rs` | +9 enum variants, +9 `kind_name`/`from_kind`/`is_valid` entries, +9 validator fns, +34 unit tests; `validate_arw` updated to reject SR2; `validate_tiff_le` updated to reject DCR |
| `config/signatures.toml` | +11 `[[signature]]` entries |
| `crates/ferrite-carver/src/lib.rs` | Assertion updated: 62 → 73 |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | `sig_group_label` updated: cr3/sr2/dcr→RAW, wv→Audio, epub/odt/cdr→Documents, msg→Office&Email, swf→Video |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `groups_cover_all_signatures` test updated: 62 → 73 |

## Design Decisions
- **SR2 vs ARW disambiguation**: `validate_sr2` requires private tag 0x7200 at an IFD0 entry boundary (stride 12); `validate_arw` rejects any file that has that tag — clean mutual exclusion without false positives
- **ODT validator covers all ODF subtypes**: checks `opendocument` substring, matching ODT/ODS/ODP/ODG/ODB; all extracted with `.odt` extension
- **SWF uses one shared validator for 3 magic patterns**: `validate_swf` operates on version/length bytes regardless of compression type
- **DCR + tiff_le mutual exclusion**: `validate_tiff_le` now rejects files with "Kodak"/"KODAK" in first 512 bytes; `validate_dcr` requires it
- **CDR RIFF size-hint**: reuses the same `size_hint_offset/len/endian/add` pattern as WAV/AVI

## Test Count
- 549 tests total (up from 517) — 34 new tests in ferrite-carver pre_validate + 1 TUI assertion update
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo test --workspace` — all passing
