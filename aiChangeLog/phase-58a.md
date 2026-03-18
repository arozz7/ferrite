# Phase 58a — exFAT Full Parser

**Date:** 2026-03-17
**Branch:** master
**Tests:** 668 total (was 658; +10 exFAT unit tests) — all passing
**Clippy:** Clean (-D warnings)

---

## What Changed

### New file

| File | Description |
|---|---|
| `crates/ferrite-filesystem/src/exfat.rs` | `ExFatParser` — full read-only exFAT `FilesystemParser` implementation |

### Modified files

| File | Change |
|---|---|
| `crates/ferrite-filesystem/src/lib.rs` | Added `mod exfat`, `pub use exfat::ExFatParser`; wired into `open_filesystem()` and `build_metadata_index()`; updated test `open_exfat_returns_err_on_truncated_device` (was `open_exfat_returns_unknown_filesystem_error`) |
| `docs/ferrite-feature-roadmap.md` | Marked all phases 45b–64 as ✅ Done in Priority Matrix; updated Executive Summary, signature count (43→99), engineering features table; Phase 58 status → ⬜ Next (Stretch) |

---

## ExFatParser — Implementation Summary

**On-disk structures parsed:**
- Boot sector (VBR): `BytesPerSectorShift` @108, `SectorsPerClusterShift` @109, `FatOffset` @80, `ClusterHeapOffset` @88, `RootDirectoryFirstCluster` @96
- FAT: 32-bit entries; 0x00 = free, ≥0xFFFFFFF8 = end-of-chain
- Directory entry sets (32 bytes each):
  - File primary: type 0x85 (live) / 0x05 (deleted); attributes, timestamps
  - Stream Extension: type 0xC0/0x40; name_length, valid_data_length, first_cluster
  - File Name: type 0xC1/0x41; up to 15 UTF-16LE chars per entry

**Deleted file detection:** Entry type high bit clear (live: 0x8X, deleted: 0x0X / 0x4X)

**RecoveryChance scoring:**
- `High` — first_cluster ≥ 2 and FAT entry is 0x00 (free)
- `Low` — first_cluster ≥ 2 but FAT entry non-zero (reallocated)
- `Unknown` — size = 0 or no valid cluster

**`FilesystemParser` methods implemented:**
- `filesystem_type()` → `FilesystemType::ExFat`
- `root_directory()` — live entries from root dir cluster chain
- `list_directory(path)` — navigate sub-dirs by FAT chain
- `read_file(entry, writer)` — follow FAT chain, write exactly `entry.size` bytes
- `deleted_files()` — re-walk root dir with `include_deleted=true`; score `RecoveryChance`

---

## Tests Added (exfat::tests — 10 tests)

| Test | Asserts |
|---|---|
| `detects_exfat` | Parser constructs OK, `filesystem_type()` = `ExFat` |
| `root_directory_lists_live_file` | Live "HELLO.TXT" entry with correct name/size/flags |
| `read_file_returns_content` | FAT chain walk produces exact 13-byte file content |
| `data_byte_offset_for_regular_file` | cluster_byte_offset(3) = 1536 |
| `deleted_files_found_with_recovery_chance` | "GONE.DAT" found, `RecoveryChance::High` (free cluster) |
| `rejects_non_exfat_device` | Zeroed device returns `Err` |
| `truncated_device_returns_err_not_panic` | 10-byte device returns `Err`, no panic |
| `exfat_ts_zero_is_none` | Zero timestamp → `None` |
| `exfat_ts_known_date` | 2000-01-01 → Unix 946_684_800 |
| `exfat_ts_invalid_month_is_none` | month=0 → `None` |
