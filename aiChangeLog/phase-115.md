# Phase 115 - Reverse Imaging

## Status: Already implemented in prior phases

Reverse imaging was fully implemented as part of Phase 114's prerequisites:

## What exists

### `crates/ferrite-imaging/src/config.rs`
- `pub reverse: bool` field on `ImagingConfig` (default: `false`)

### `crates/ferrite-imaging/src/passes/copy.rs`
- When `config.reverse` is true, the work list is reversed before iteration so
  the copy pass reads from the last LBA toward the first

### `crates/ferrite-tui/src/screens/imaging/mod.rs`
- `pub reverse: bool` field on `ImagingState`
- `KeyCode::Char('r') => self.reverse = !self.reverse` toggle
- Wired into `ImagingConfig { reverse: self.reverse, ... }` when imaging starts

### `crates/ferrite-tui/src/screens/imaging/render.rs`
- Config panel row: `Reverse : YES/NO  (r to toggle)` — yellow+bold when active

## Use case
Useful when bad sectors are concentrated at the start of the disk (e.g., a
corrupted partition table area on an SMR drive) and user data is at the end.
Imaging from the end recovers data before retrying the damaged start area.

## No new code required for this phase.
