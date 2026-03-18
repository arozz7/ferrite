# Phase 78: EBML/MKV/WebM Size Hint

## Summary
Added EBML container size hint that reads the Segment element size from
MKV and WebM files, avoiding the largest blind extraction (8 GiB).

## Algorithm
1. Read EBML header element ID (`0x1A45DFA3`) + VINT size → skip header
2. Read Segment element ID (`0x18538067`) + VINT size
3. If size is "unknown" (all-ones after masking), return None
4. File size = `header_end + 4 (seg ID) + vint_len + segment_size`

## Changes

### `crates/ferrite-carver/src/size_hint/ebml.rs` (new)
- `ebml_hint()` — EBML header + Segment size reader (~100 lines)
- `read_ebml_vint()` — EBML Variable-Size Integer decoder (reusable)
- `is_unknown_size()` — detects all-ones "unknown" VINT values
- 6 unit tests: vint 1-byte, vint 2-byte, unknown detection, MKV basic,
  unknown returns None, no segment returns None

### `crates/ferrite-carver/src/size_hint/mod.rs`
- Added `mod ebml;` and `SizeHint::Ebml` dispatch arm

### `crates/ferrite-carver/src/signature.rs`
- Added `SizeHint::Ebml` variant
- Added `"ebml"` match in TOML parser `size_hint_kind`

### `config/signatures.toml`
- MKV signature: added `size_hint_kind = "ebml"`
- WebM signature: added `size_hint_kind = "ebml"`

## Impact
- MKV/WebM: 8 GiB blind extract → exact from Segment element

## Verification
- `cargo test --workspace` — 747 tests pass
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
