# Phase 110 — Sparse Image Output

## Motivation
A 4 TB source drive requires 4 TB of free space for a raw image even when most
sectors contain zeros.  Sparse files let the OS skip allocating disk blocks for
zero runs; a 4 TB drive that is 30 % full may produce only a 1.2 TB image file
on disk.

## Changes

### `crates/ferrite-imaging/Cargo.toml`
Added `windows-sys` as a Windows-only dependency (features:
`Win32_Foundation`, `Win32_System_IO`, `Win32_System_Ioctl`) so
`FSCTL_SET_SPARSE` can be called via `DeviceIoControl`.

### `crates/ferrite-imaging/src/sparse.rs` — new module
- `enable_sparse(file: &File) -> io::Result<()>`:
  On Windows sends `FSCTL_SET_SPARSE` via `DeviceIoControl`; silently
  proceeds on failure (non-NTFS destination).  No-op on Linux/macOS.
- `write_or_skip(file: &mut File, pos: u64, buf: &[u8]) -> io::Result<()>`:
  If `buf` is entirely zero, seeks to `pos + buf.len()` without writing
  (creating a hole).  Otherwise seeks to `pos` and calls `write_all`.

### `crates/ferrite-imaging/src/config.rs`
Added `pub sparse_output: bool` to `ImagingConfig`.  Default: `true`.

### `crates/ferrite-imaging/src/lib.rs`
Added `pub mod sparse` declaration.

### `crates/ferrite-imaging/src/engine.rs`
- `new()`: after opening the output file, if `sparse_output = true` **and**
  the file is newly created (len == 0):
  1. Calls `sparse::enable_sparse(&output)`.
  2. Writes a single zero byte at `device_size - 1` then seeks back to 0,
     pre-setting the file length without allocating all blocks (write a byte
     rather than `set_len` to remain compatible with file systems that expand
     the file on `set_len`).
- `write_block(pos, buf)` — new `pub(crate)` helper:
  Dispatches to `sparse::write_or_skip` when `sparse_output = true`,
  otherwise falls back to a plain seek + `write_all`.
- Added `use std::io::{Seek, SeekFrom, Write}` to support the new helper.
- All test `ImagingConfig` literals updated with `sparse_output: false` to
  keep tests deterministic.

### `crates/ferrite-imaging/src/passes/{copy,trim,sweep,scrape,retry}.rs`
Replaced the repeated `engine.output.seek(…).and_then(|_| write_all(…))`
pattern with `engine.write_block(pos, buf)?`.  Removed the now-unused
`use std::io::{Seek, SeekFrom, Write}` imports from each pass file.

### `crates/ferrite-tui/src/screens/imaging/mod.rs`
- Added `pub sparse: bool` to `ImagingState` (default `true`).
- Added `KeyCode::Char('S')` handler: toggles `sparse` on/off.
- `start_imaging_forced`: passes `sparse_output: self.sparse` to `ImagingConfig`.

### `crates/ferrite-tui/src/screens/imaging/render.rs`
- Added "Sparse  : ON / OFF  (S to toggle — skips zero blocks, saves space
  on NTFS/ext4)" row to the Configuration panel.
- Increased config panel height constraint from 12 → 13 rows.

## Tests Added (in `sparse.rs`)
- `write_or_skip_all_zero_does_not_write_data` — after skip, position = 512; any file content is zero
- `write_or_skip_non_zero_writes_data` — non-zero buffer written and reads back correctly
- `write_or_skip_non_zero_at_nonzero_offset` — zero block skipped, next block written at offset 512
- `write_or_skip_mixed_buffer_writes_entirely` — buffer with one non-zero byte is written in full
- `enable_sparse_does_not_error_on_regular_file` — no-op/FSCTL succeeds without error

## Limitation (documented in UI hint)
Sparse savings depend on destination filesystem support.  FAT32 and exFAT do
not support sparse files; the OS will silently fall back to dense allocation on
those destinations.  The toggle lets users disable sparse mode proactively when
imaging to FAT32 USB sticks.

## Test Counts (after phase)
All workspace tests pass.  ferrite-imaging: 11 → 48 tests (5 new in sparse.rs,
32 pre-existing tests updated with `sparse_output: false`).
