# Phase 106 — Post-Validator Expansion (ISOBMFF + EBML)

## Summary

Added two new file-based post-extraction validators for the two remaining
high-traffic container families that previously returned `CarveQuality::Unknown`.
Also fixed a stale TUI test assertion that expected 139 signatures (should be 140
since Phase 105 added PAR2).

## New validators

### `validate_isobmff_file(path)` — MP4/MOV/M4V/3GP/M4A/HEIC/CR3

Walks up to 64 top-level ISOBMFF boxes and verifies:
1. At least one `ftyp` box is present.
2. At least one `moov` or `mdat` box is present.
3. Every box's declared size fits within the file.
4. Every box type is 4 printable ASCII bytes.

Handles extended 64-bit sizes (`size == 1`) and "extends to EOF" boxes (`size == 0`).

### `validate_ebml_file(path)` — MKV/WebM

Verifies:
1. EBML element ID `0x1A45DFA3` at offset 0.
2. Valid VINT size follows the EBML ID.
3. Segment element ID `0x18538067` follows the EBML header.
4. Valid VINT size (or well-formed unknown-size) follows the Segment ID.

Unknown-size Segment (`0xFF` VINT) is accepted — it is valid in live-streaming MKV.

## Files changed

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/post_validate/binary_validators.rs` | Added `validate_isobmff_file`, `validate_ebml_file`, `read_vint` helper |
| `crates/ferrite-carver/src/post_validate/mod.rs` | Re-exported both new validators |
| `crates/ferrite-carver/src/post_validate/tests_binary.rs` | 11 new unit tests (6 ISOBMFF + 5 EBML) |
| `crates/ferrite-tui/src/screens/carving/extract.rs` | Both dispatch blocks extended to cover mp4/mov/m4v/3gp/m4a/heic/cr3 → isobmff; mkv/webm → ebml |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | Fixed stale assertion: 139 → 140 |
| `docs/ferrite-feature-roadmap.md` | Added Phases 106–109 to roadmap |

## Test results

- `ferrite-carver`: **653 tests** (up from 642), all passing
- Full workspace: all passing
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean
