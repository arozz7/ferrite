# Phase 114 - Configurable Block Size Per Imaging Pass

## Goal
Replace the single `copy_block_size` config field with `pass_block_sizes: [u64; 5]` so
each of the five imaging passes can use an independently tuned read block size.

## Files Changed

### `crates/ferrite-imaging/src/config.rs`
- Replaced `copy_block_size: u64` with `pass_block_sizes: [u64; 5]`
- Index mapping: 0=Copy, 1=Trim, 2=Sweep, 3=Scrape, 4=Retry
- Defaults: `[512 KiB, 512 B, 512 B, 512 B, 512 B]` (copy gets large block; others default to
  sector-size sentinel for precise recovery)
- `validate()` checks all 5 values are > 0; `_sector_size` param kept for API stability
- New tests: `default_pass_block_sizes_are_valid`, `validate_rejects_zero_block_size`

### `crates/ferrite-imaging/src/passes/copy.rs`
- Uses `config.pass_block_sizes[0]` (was `config.copy_block_size`)

### `crates/ferrite-imaging/src/passes/trim.rs`
- `block_size = config.pass_block_sizes[1].max(sector_size)` — replaces hardcoded sector_size
- Buffer and chunk use `block_size` instead of `sector_size`

### `crates/ferrite-imaging/src/passes/sweep.rs`
- Same pattern as trim, using `config.pass_block_sizes[2]`

### `crates/ferrite-imaging/src/passes/scrape.rs`
- Uses `config.pass_block_sizes[3].max(sector_size)`

### `crates/ferrite-imaging/src/passes/retry.rs`
- Uses `config.pass_block_sizes[4].max(sector_size)`
- Reverse-direction start position updated to use `block_size`

### `crates/ferrite-imaging/src/engine.rs`
- All test `ImagingConfig` literals updated from `copy_block_size: SECTOR as u64` to
  `pass_block_sizes: [SECTOR as u64; 5]`

### `crates/ferrite-tui/src/screens/imaging/mod.rs`
- `block_size_str: String` replaced with `pass_block_size_strs: [String; 5]`
- `EditField::BlockSize` now carries `usize` pass index: `EditField::BlockSize(usize)`
- `b` key enters `EditField::BlockSize(0)`; pressing `b` again while editing cycles
  0 -> 1 -> 2 -> 3 -> 4 -> None
- `field_mut()` dispatches `BlockSize(n)` to `pass_block_size_strs[n]`
- Config build: `std::array::from_fn` parses all 5 KiB strings; empty = per-pass default

### `crates/ferrite-tui/src/screens/imaging/render.rs`
- Replaced single "BlockSz" row with compact "Passes" row showing all 5 pass labels
  and their current values (active one highlighted yellow+bold with cursor)
- Format: `Cpy(512)  Trm(~1S)  Swp(~1S)  Scr(~1S)  Rtr(~1S)  (b to edit/cycle)`

## Behaviour Note
Defaults for passes 1-4 are `512 B` (the sector-size sentinel). Since each pass clamps
to `max(configured, sector_size)`, this preserves the existing sector-precise recovery
behaviour. Users can increase Trim/Sweep sizes for faster (but less precise) operation
on drives with large contiguous bad zones.

## Tests
- All workspace tests pass (`cargo test --workspace`)
- Clippy clean (`cargo clippy --workspace --all-targets -- -D warnings`)
- Format clean (`cargo fmt --check`)
