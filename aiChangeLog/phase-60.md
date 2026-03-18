# Phase 60 — PhotoRec Gap Batch: Tier B New-Infrastructure (13 new signatures, 73 → 86)

## Summary
Added 13 new file format signatures across 12 format families (EMLX deferred — no fixed
magic; requires Phase 62 non-zero-offset infrastructure), expanding the carving database
from 73 to 86 signatures. All changes are in `ferrite-carver` (pre-validators + TOML)
and `ferrite-tui` (group routing). No new crates.

## New Signatures

| Format | Extension | Magic | Pre-validator |
|--------|-----------|-------|---------------|
| Canon CRW | `crw` | `49 49 1A 00 00 00 48 45 41 50 43 43 44 52` | `Crw` — "HEAPCCDR" at offset 6 |
| Minolta MRW | `mrw` | `00 4D 52 4D` | `Mrw` — first block tag at offset 8 in {PRD, TTW, WBG} |
| KeePass 2.x | `kdbx` | `03 D9 A2 9A 67 FB 4B B5` | `Kdbx` — major version @10 in {3, 4} |
| KeePass 1.x | `kdb` | `03 D9 A2 9A 65 FB 4B B5` | `Kdb` — major version @10 in {1, 2} |
| EnCase E01 | `e01` | `45 56 46 09 0D 0A FF 00` | `E01` — segment number @8 == 1 |
| PCAP (LE) | `pcap` | `D4 C3 B2 A1` | `Pcap` — major==2, minor==4 (LE) |
| PCAP (BE) | `pcap` | `A1 B2 C3 D4` | `Pcap` — major==2, minor==4 (BE) |
| Windows Minidump | `dmp` | `4D 44 4D 50 93 A7` | `Dmp` — stream_count @8 > 0 |
| Apple bplist | `plist` | `62 70 6C 69 73 74 30 30` | `Plist` — min length ≥ 34 bytes |
| MPEG-TS | `ts` | `47` | `Ts` — sync bytes at offsets 0, 188, 376 (stride-188) |
| Blu-ray M2TS | `m2ts` | `?? ?? ?? ?? 47` | `M2ts` — sync bytes at offsets 4, 196, 388 (stride-192) |
| LUKS Encrypted | `luks` | `4C 55 4B 53 BA BE` | `Luks` — version @6 (u16 BE) in {1, 2} |
| Sigma X3F | `x3f` | `46 4F 56 62` | `X3f` — major byte @5 (LE) in {2, 3} |

## Modified Files

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/pre_validate.rs` | +12 enum variants, +12 `kind_name`/`from_kind`/`is_valid` entries, +12 validator fns, +34 unit tests |
| `config/signatures.toml` | +13 `[[signature]]` entries |
| `crates/ferrite-carver/src/lib.rs` | Assertion updated: 73 → 86 |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | `sig_group_label` updated: crw/mrw/x3f→RAW Photos, ts/m2ts→Video, kdbx/kdb/e01/pcap/dmp/plist/luks→System |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `groups_cover_all_signatures` test updated: 73 → 86 |

## Design Decisions

- **PCAP two-variant magic**: LE (`D4 C3 B2 A1`) and BE (`A1 B2 C3 D4`) are separate TOML
  entries sharing one `Pcap` validator. The validator detects byte order from the magic bytes
  and validates accordingly — clean mutual exclusion, no false positives.
- **MPEG-TS stride validation**: The `0x47` sync byte appears every 188 bytes. A single-byte
  magic generates many false positives; the `Ts` validator requires three consecutive sync
  bytes at stride-188 offsets (0, 188, 376), which is specific enough to reject random noise.
- **M2TS 5-byte wildcard magic**: Blu-ray M2TS packets start with a 4-byte timestamp (not
  fixed), then `0x47` at offset 4. The TOML magic `?? ?? ?? ?? 47` anchors on the first
  sync byte; the `M2ts` validator confirms stride-192 pattern (offsets 4, 196, 388).
- **EMLX deferred**: Apple EMLX files start with a variable-length ASCII decimal byte count
  followed by `\n`. There is no fixed binary magic at offset 0. Carving requires either
  non-zero-offset magic matching (Phase 62 infrastructure) or a content-search approach.
  Deferred to Phase 62+.
- **KDB vs KDBX disambiguation**: The two formats differ at magic byte 4 (`0x65` vs `0x67`),
  providing clean TOML-level discrimination. Validators only check that the major version
  number matches the expected generation (KDB: 1-2, KDBX: 3-4).
- **X3F version encoding**: The version is stored as a u32 LE at offset 4. In little-endian
  layout, byte 5 carries the major version (2 or 3 for all known Sigma cameras).

## Test Count
- 583 tests total (up from 549) — 34 new tests in ferrite-carver pre_validate + 1 TUI assertion update
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo test --workspace` — all passing
