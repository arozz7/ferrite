# Phase 77: RAR Size Hint (Block Walker)

## Summary
Added RAR block walker supporting both RAR4 (fixed-width blocks) and RAR5
(variable-length integer blocks) to derive exact archive size.

## Algorithm

### RAR4
1. Skip 7-byte signature
2. Walk blocks: `HEAD_TYPE` (1), `HEAD_FLAGS` (2), `HEAD_SIZE` (2)
3. If `HEAD_FLAGS` bit 15 set: read `ADD_SIZE` (u32 LE) → data = HEAD_SIZE + ADD_SIZE
4. Stop at `HEAD_TYPE == 0x7B` (end-of-archive) or invalid block
5. Safety cap: 100,000 blocks

### RAR5
1. Skip 8-byte signature (`Rar!\x1a\x07\x01\x00`)
2. Walk blocks: Header CRC32 (4) + Header size (vint) + Header type (vint) + Header flags (vint)
3. If flags bit 1: Data size (vint) follows — add to block total
4. Stop at Header type == 5 (end-of-archive) or invalid block

## Changes

### `crates/ferrite-carver/src/size_hint/rar.rs` (new)
- `rar_hint()` — dispatches to `rar4_walk()` or `rar5_walk()`
- `read_rar5_vint()` — RAR5 variable-length integer decoder
- 5 unit tests: RAR4 end-of-archive, RAR4 with data block, RAR5 end-of-archive,
  vint parsing, non-RAR data

### `crates/ferrite-carver/src/size_hint/mod.rs`
- Added `mod rar;` and `SizeHint::Rar` dispatch arm

### `crates/ferrite-carver/src/signature.rs`
- Added `SizeHint::Rar` variant
- Added `"rar"` match in TOML parser `size_hint_kind`

### `config/signatures.toml`
- RAR signature: added `size_hint_kind = "rar"`

## Impact
- RAR: 500 MiB blind extract → exact from block walk

## Verification
- `cargo test --workspace` — 747 tests pass
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
