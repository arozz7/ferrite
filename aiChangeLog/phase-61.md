# Phase 61 — PhotoRec Tier B Continued (8 new signatures, 86 → 94)

## Summary
Added 8 new file format signatures across 8 format families, expanding the carving
database from 86 to 94 signatures. All changes are in `ferrite-carver` (pre-validators
+ TOML) and `ferrite-tui` (group routing). No new crates.

## New Signatures

| Format | Extension | Magic | Pre-validator |
|--------|-----------|-------|---------------|
| Monkey's Audio | `ape` | `4D 41 43 20` ("MAC ") | `Ape` — version @6 (u16 LE) in [3930, 4100] |
| Sun AU Audio | `au` | `2E 73 6E 64` (".snd") | `Au` — data_offset @4 ≥ 24; encoding @12 in known set |
| TrueType Font | `ttf` | `00 01 00 00 00` | `Ttf` — numTables @4 (u16 BE) in [4, 50] |
| WOFF Web Font | `woff` | `77 4F 46 46` ("wOFF") | `Woff` — flavor in {TTF/OTF/true}, length ≥ 44, numTables in [1, 50] |
| CHM Help | `chm` | `49 54 53 46 03 00 00 00 60 00 00 00` | `Chm` — 12-byte fully deterministic magic |
| Blender 3D | `blend` | `42 4C 45 4E 44 45 52` ("BLENDER") | `Blend` — pointer-size @7 in {`-`, `_`}; endian @8 in {`v`, `V`} |
| Adobe InDesign | `indd` | `06 06 ED F5 D8 1D 46 E5 BD 31 EF E7 FE 74 B7 1D` | `Indd` — 16-byte GUID, globally unique |
| Windows WTV | `wtv` | `B7 D8 00 20 37 49 DA 11 A6 4E 00 07 E9 5E AD 8D` | `Wtv` — 16-byte GUID, globally unique |

## Modified Files

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/pre_validate.rs` | +8 enum variants, +8 `kind_name`/`from_kind`/`is_valid` entries, +8 validator fns, +25 unit tests |
| `config/signatures.toml` | +8 `[[signature]]` entries |
| `crates/ferrite-carver/src/lib.rs` | Assertion updated: 86 → 94 |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | `sig_group_label` updated: ape/au→Audio, ttf/woff/chm/blend/indd→Documents, wtv→Video |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `groups_cover_all_signatures` test updated: 86 → 94 |

## Design Decisions

- **WOFF flavor validation**: Accepts three valid sfnt flavors — `0x00010000` (TrueType),
  `0x4F54544F` ("OTTO", OpenType with CFF), and `0x74727565` ("true", older Mac TrueType).
  Rejects corrupted WOFF files where the flavor field has been overwritten.
- **TTF 5-byte magic**: The `00 01 00 00` sfVersion is followed by `00` (high byte of
  numTables, always zero for ≤255 tables). The 5th magic byte reduces false positives
  from files that start with `00 01 00 00` (common in binary data).
- **CHM 12-byte magic**: The ITSF header encodes a deterministic 12-byte sequence
  (magic + version 3 + header_length 96). No further validation is needed beyond
  confirming the data is present.
- **Blender pointer/endian bytes**: Blender embeds platform metadata immediately after
  the "BLENDER" magic — `-` for 32-bit pointers, `_` for 64-bit, `v` for little-endian,
  `V` for big-endian. All four combinations are valid; the validator rejects any other byte.
- **InDesign / WTV 16-byte GUIDs**: Both formats use globally unique 128-bit identifiers
  as their file magic. No false positives are possible; validators just confirm length.
- **AU encoding set**: Accepts encodings 1–7 (MULAW, PCM 8/16/24/32-bit, float 32/64-bit)
  and 23–27 (G.721, G.722, G.723 variants). Rejects encoding 0 and any value above 27
  that is not in the known G.7xx range.

## Test Count
- 608 tests total (up from 583) — 25 new tests in ferrite-carver pre_validate + 1 TUI assertion update
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo test --workspace` — all passing
