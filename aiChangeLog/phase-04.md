# Phase 4: Partition Table Recovery

**Date:** 2026-03-10
**Status:** Complete

## Summary

Implemented `ferrite-partition` — pure-Rust MBR/GPT parsing, filesystem
signature scanning, and partition table reconstruction from scan hits. No
external C libraries; all byte manipulation is done with `byteorder` +
`crc32fast` + `uuid`.

## Files Created

```
NEW  crates/ferrite-partition/Cargo.toml
NEW  crates/ferrite-partition/src/lib.rs          — public API, read_partition_table()
NEW  crates/ferrite-partition/src/error.rs        — PartitionError, Result<T>
NEW  crates/ferrite-partition/src/types.rs        — PartitionEntry, PartitionTable, FsType, FsSignatureHit
NEW  crates/ferrite-partition/src/mbr.rs          — MBR parser + is_protective_mbr()
NEW  crates/ferrite-partition/src/gpt.rs          — GPT parser + CRC32 validation + UTF-16LE name parsing
NEW  crates/ferrite-partition/src/scanner.rs      — NTFS/FAT16/FAT32/ext4 magic-byte scanner
NEW  crates/ferrite-partition/src/reconstruct.rs  — PartitionTable from FsSignatureHit list
MOD  Cargo.toml                                   — added ferrite-partition to members and workspace deps
```

## Key Design Decisions

- **Pure-bytes API for mbr/gpt**: `mbr::parse(data: &[u8], ...)` and
  `gpt::parse(header: &[u8], entries: &[u8], ...)` are pure functions that
  take pre-read byte slices. Device I/O is coordinated only in `read_partition_table()`.
  This makes tests trivial — no mock device needed for the parser modules.
- **GPT protective MBR detection**: `is_protective_mbr()` checks partition type
  `0xEE` at MBR slot 0. `read_partition_table()` branches on this to select GPT path.
- **GPT CRC validation**: Both header CRC32 (with field zeroed during compute) and
  partition array CRC32 are validated before any entries are parsed.
- **UUID mixed-endian**: GPT stores GUIDs in mixed-endian. `Uuid::from_bytes_le()`
  handles the conversion correctly (first 3 components LE, last 2 BE).
- **Scanner granularity**: `ScanOptions::step` controls scan density. Use
  `sector_size` (512 B) for exhaustive scans; `1 << 20` (1 MiB) for fast
  alignment-based scans. Tests use 512-byte steps with `FileBlockDevice`.
- **ext4 superblock coverage**: The ext4 magic (`0x53 0xEF`) sits at byte 1080
  relative to partition start (superblock at 1024, s_magic at +56). The scanner
  reads `ceil(1082 / sector_size)` sectors per position to cover all three
  filesystem types in one pass.
- **Reconstruction heuristic**: `from_scan_hits()` assigns end LBA as
  `next_partition_start - 1`, with the last partition extending to disk end.
  No attempt to read actual filesystem metadata — kept simple for recovery context.

## Verification

- `cargo test --workspace`: 86 tests pass (27 ferrite-partition + 18 ferrite-smart + 25 ferrite-imaging + 14 ferrite-blockdev + 2 ferrite-core)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean

## Test Coverage (ferrite-partition, 27 tests)

| Module | Tests |
|---|---|
| `mbr` | parses_single_entry, skips_empty_entries, non_bootable_entry, invalid_signature_returns_error, buffer_too_small_returns_error, start_byte_and_size_bytes, protective_mbr_detected, non_protective_mbr |
| `gpt` | parse_empty_table, parse_single_entry, nil_entries_are_skipped, wrong_signature_returns_error, header_crc_mismatch_returns_error, array_crc_mismatch_returns_error, utf16_name_parsed_correctly |
| `scanner` | detects_ntfs_at_start, detects_fat32_at_start, detects_fat16_at_start, detects_ext4_superblock_magic, no_signature_returns_empty, detects_signature_at_second_sector_with_step, scan_respects_start_and_end_byte |
| `reconstruct` | empty_hits_produces_empty_table, single_hit_extends_to_disk_end, multiple_hits_assign_correct_boundaries, sequential_indices_assigned, recovered_entries_are_not_bootable |
