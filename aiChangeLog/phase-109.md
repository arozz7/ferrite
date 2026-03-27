# Phase 109 — Filesystem Recovery Hardening

## Motivation
The existing parsers assumed a readable MBR or GPT partition table. Severely
damaged drives (e.g. LBA 0 unreadable, overwritten MBR) have no valid partition
table at all.  Phase 109 adds three complementary changes to handle this case
gracefully.

## Changes

### 1. `ferrite-partition/src/lib.rs` — `read_partition_table_with_fallback()`
New public function.

Algorithm:
1. Call `read_partition_table()`.
2. If it succeeds and the table has ≥1 entry → return it unchanged.
3. Otherwise (parse error OR empty result) → scan the first 2 GiB at 1 MiB
   alignment steps for filesystem boot-sector / superblock signatures (NTFS,
   FAT16, FAT32, ext4).
4. Reconstruct a `PartitionTable` with `kind = Recovered` from any hits.
   Returns an empty Recovered table when the media is blank/unformatted.

### 2. `ferrite-filesystem/src/lib.rs` — `detect_filesystem_at(device, lba)`
New public function.  Converts `lba` → byte offset via `device.sector_size()`
and delegates to the existing `pub(crate) detect_at()` boot-sector probe.

Useful when the partition table is absent: pair with `ferrite_partition::scan()`
to find candidate LBAs, then call `detect_filesystem_at()` to confirm each hit
before opening a parser.

### 3. `ferrite-tui/src/screens/partition.rs` — Auto-scan on empty table
New `PartitionStatus::AutoScanning` variant.

When `tick()` receives a `PartitionMsg::Table` result for a `Reading`
state and the table is empty, the state machine automatically transitions to
`AutoScanning` and spawns a background scan (same as pressing `s`).  The title
bar shows `"no table found, scanning…"` during this phase.  Non-empty tables
go straight to `Done` as before.

## Tests Added
- `ferrite-partition::tests::fallback_empty_device_returns_empty_recovered_table` — blank device → empty Recovered table, no error
- `ferrite-partition::tests::fallback_finds_ntfs_when_no_partition_table` — NTFS magic at 1 MiB with no MBR → 1 Recovered entry
- `ferrite-partition::tests::fallback_passes_through_valid_mbr_table` — valid MBR → Mbr kind returned, scan not triggered
- `ferrite-filesystem::tests::detect_filesystem_at_lba_zero_ntfs` — NTFS at LBA 0
- `ferrite-filesystem::tests::detect_filesystem_at_nonzero_lba` — NTFS at LBA 2; LBA 0 returns Unknown
- `ferrite-filesystem::tests::detect_filesystem_at_unknown_on_zero_device` — zeroed device → Unknown
- `ferrite-tui::screens::partition::tests::auto_scan_triggered_when_read_returns_empty_table` — Reading + empty table → AutoScanning
- `ferrite-tui::screens::partition::tests::no_auto_scan_when_read_returns_non_empty_table` — non-empty table → Done

## Test Counts (after phase)
All workspace tests pass.  New tests: 8 (3 in ferrite-partition, 3 in ferrite-filesystem, 2 in ferrite-tui).
