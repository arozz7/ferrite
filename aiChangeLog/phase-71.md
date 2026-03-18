# Phase 71 — RAR Continuation Volume Rejection

## Problem

The RAR carver extracted multi-volume RAR continuation volumes that are
useless without the preceding volumes.  WinRAR reports "You need to start
extraction from a previous volume" and "No files to extract".

Root cause: the `Rar!\x1a\x07` magic appears at the start of **every**
volume in a multi-volume RAR set, not just the first.  The old validator
only checked the format byte (0x00 = RAR4, 0x01 = RAR5) and did not
inspect the volume flags.

## Solution

### RAR4 archive header flag check

For RAR4 files (format byte = 0x00), the archive header begins at offset 7:

| Offset | Field | Check |
|--------|-------|-------|
| 9 | HEAD_TYPE | must be 0x73 (archive header) |
| 10–11 | HEAD_FLAGS (u16 LE) | if bit 0 (MHD_VOLUME) is set, bit 8 (MHD_FIRSTVOLUME) must also be set |

This rejects continuation volumes while accepting standalone archives and
first volumes.

### Verification against real carved files

Out of 9 carved RAR files from a 4 TB drive:
- 4 were continuation volumes (flags & 0x0001 ≠ 0, flags & 0x0100 == 0) → now rejected
- 5 were standalone or first volumes → correctly accepted

## Files Changed

- `crates/ferrite-carver/src/pre_validate.rs` — extended `validate_rar` with
  HEAD_TYPE + volume flag checks; 8 new unit tests
- `aiChangeLog/phase-71.md`

## Tests

- 386 ferrite-carver tests pass (8 new).
- `cargo clippy -p ferrite-carver --all-targets -- -D warnings` clean.
