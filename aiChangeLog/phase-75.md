# Phase 75: PE/EXE Size Hint

## Summary
Added PE section table walker to derive exact file size for Windows executables.

## Algorithm
1. Read `e_lfanew` (u32 LE @60) → PE signature offset
2. Validate `PE\0\0` at `e_lfanew`
3. Read `NumberOfSections` (u16 LE @`e_lfanew+6`), `SizeOfOptionalHeader` (u16 LE @`e_lfanew+20`)
4. Section table at `e_lfanew + 24 + SizeOfOptionalHeader`
5. Walk sections (40 bytes each): `max(PointerToRawData + SizeOfRawData)`
6. Safety: cap at 256 sections

## Changes

### `crates/ferrite-carver/src/size_hint/pe.rs` (new)
- `pe_hint()` — PE section table walker (~80 lines)
- 3 unit tests: basic two sections, invalid lfanew, bad PE signature

### `crates/ferrite-carver/src/size_hint/mod.rs`
- Added `mod pe;` and `SizeHint::Pe` dispatch arm

### `crates/ferrite-carver/src/signature.rs`
- Added `SizeHint::Pe` variant
- Added `"pe"` match in TOML parser `size_hint_kind`

### `config/signatures.toml`
- EXE signature: added `size_hint_kind = "pe"`

## Impact
- EXE: 100 MiB blind extract → exact from section table

## Verification
- `cargo test --workspace` — 747 tests pass
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
