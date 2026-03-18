# Phase 76: ELF Size Hint

## Summary
Added ELF section/program header walker to derive exact file size for Linux executables.

## Algorithm
1. ELF class @4 (1=32-bit, 2=64-bit), byte order @5 (1=LE, 2=BE)
2. Read section header table offset, entry size, count
3. Read program header table offset, entry size, count
4. Walk program headers: `max(p_offset + p_filesz)`
5. Return `max(section_table_end, max_segment_extent)`

## Changes

### `crates/ferrite-carver/src/size_hint/elf.rs` (new)
- `elf_hint()` — ELF header walker, supports 32/64-bit LE/BE (~100 lines)
- 3 unit tests: 64-bit LE, 32-bit BE, invalid class

### `crates/ferrite-carver/src/size_hint/mod.rs`
- Added `mod elf;` and `SizeHint::Elf` dispatch arm

### `crates/ferrite-carver/src/signature.rs`
- Added `SizeHint::Elf` variant
- Added `"elf"` match in TOML parser `size_hint_kind`

### `config/signatures.toml`
- ELF signature: added `size_hint_kind = "elf"`

## Impact
- ELF: 100 MiB blind extract → exact from section/segment headers

## Verification
- `cargo test --workspace` — 747 tests pass
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
