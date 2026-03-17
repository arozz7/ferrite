# Phase 46 — Tier 1 Signature Batch

## Summary
Added 10 high-value signatures (43 → 53 total).

## New Signatures
| Format | Extension | Magic | Pre-validate Logic |
|---|---|---|---|
| WebP | webp | RIFF + WEBP subtype | RIFF size ≥ 4; WEBP at +8 |
| AAC/M4A | m4a | ISOBMFF M4A brand | Box size in [12,512] |
| GZip | gz | 1F 8B | CM=8; reserved flag bit unset |
| EML email | eml | "From " | Next char is printable ASCII |
| ELF executable | elf | 7F ELF | EI_CLASS in {1,2}; EI_DATA in {1,2}; EI_VERSION=1 |
| Registry hive | dat | "regf" | Major version=1; minor in [2,6] |
| Photoshop PSD | psd | "8BPS" | Version in {1,2}; channels in [1,56] |
| VHD | vhd | "conectix" | Disk type in [2,4] |
| VHDX | vhdx | "vhdxfile" | 8-byte magic sufficient |
| QCOW2 | qcow2 | QFI\xFB | Version in {2,3}; cluster_bits in [9,21] |

## Files Changed
- `config/signatures.toml` — 10 new `[[signature]]` blocks appended
- `crates/ferrite-carver/src/pre_validate.rs` — 10 new enum variants, dispatch arms, validator functions, and unit tests
- `crates/ferrite-carver/src/lib.rs` — updated assertion from 43 to 53
- `crates/ferrite-tui/src/screens/carving/helpers.rs` — updated `sig_group_label()` to route new extensions to correct groups
- `crates/ferrite-tui/src/screens/carving/mod.rs` — updated `groups_cover_all_signatures` test assertion from 43 to 53

## TUI Group Assignments
- Images: webp, psd
- Audio: m4a
- Archives: gz
- Documents: eml
- System: elf, dat (regf), vhd, vhdx, qcow2

## Tests
168 unit tests in ferrite-carver (up from 136 pre-phase-46 baseline after phase-40–42 additions).
69 unit tests in ferrite-tui.
All tests passing, clippy clean, fmt applied.
