# Phase 58b — APFS MVP Parser

**Date:** 2026-03-17
**Branch:** master
**Tests:** 677 total (was 668; +9 APFS unit tests) — all passing
**Clippy:** Clean (-D warnings)

---

## What Changed

### New file

| File | Description |
|---|---|
| `crates/ferrite-filesystem/src/apfs.rs` | `ApfsParser` — full APFS read-only `FilesystemParser` MVP |

### Modified files

| File | Change |
|---|---|
| `crates/ferrite-filesystem/src/detect.rs` | Added APFS detection: `NXSB` magic (0x4253584E) at byte 32 of sector 0 |
| `crates/ferrite-filesystem/src/lib.rs` | Added `mod apfs`, `pub use apfs::ApfsParser`, `FilesystemType::Apfs` variant, `Apfs` → `ApfsParser` in `open_filesystem()` and `build_metadata_index()` |

---

## ApfsParser — Implementation Summary

**On-disk walk:**
1. Container superblock (block 0): verify `NXSB` magic, read `nx_block_size`, `nx_omap_oid`, `nx_fs_oid[0]`
2. Container omap B-tree (at `nx_omap_oid`): walk fixed-KV B-tree to build virtual OID → physical block map
3. Volume superblock (at resolved paddr): verify `APSB` magic, read `apfs_omap_oid`, `apfs_root_tree_oid`
4. Volume omap B-tree (at `apfs_omap_oid`): walk fixed-KV B-tree to build volume OID → physical block map
5. FS root B-tree (at resolved `apfs_root_tree_oid`): walk variable-KV B-tree to collect inodes, dirents, extents

**B-tree support:**
- Omap: fixed-KV, recursive multi-level walk; keeps highest-XID entry per OID
- FS tree: variable-KV (kvloc_t entries), leaf nodes; index node descent via volume omap
- `BTNODE_ROOT` detection to calculate `val_area_end` correctly (subtracts 40-byte `btree_info_t`)

**FS record types parsed:**
- `APFS_TYPE_INODE (3)` → `InodeRecord` (ino, parent_id, mode, size, timestamps)
- `APFS_TYPE_DIR_REC (9)` → `DirentRecord` (parent_ino, name, child_ino)
- `APFS_TYPE_FILE_EXTENT (8)` → `ExtentRecord` (ino, logical_offset, byte_length, phys_block)

**`FilesystemParser` methods implemented:**
- `filesystem_type()` → `FilesystemType::Apfs`
- `root_directory()` — all dirents with parent_ino = 2 (APFS_ROOT_DIR_INO)
- `list_directory(path)` — navigate path components by inode number
- `read_file(entry, writer)` — sort extents by logical offset, read physical blocks
- `deleted_files()` — returns empty `Vec` (APFS reclaims inodes immediately)

**Known limitations (MVP):**
- Single volume only (`nx_fs_oid[0]`)
- No encryption (`crypto_id` ignored)
- No snapshot support
- No undelete (returns empty `Vec`)

**Timestamps:** APFS nanosecond epoch (2001-01-01) converted to Unix seconds via `+978_307_200`

---

## Tests Added (apfs::tests — 9 tests)

| Test | Asserts |
|---|---|
| `detects_apfs` | Parser constructs OK, `filesystem_type()` = `Apfs` |
| `root_directory_lists_file` | "HELLO.TXT" with correct name/size/flags |
| `read_file_returns_content` | Extent walk produces exact 13-byte content |
| `deleted_files_returns_empty` | Always returns empty Vec |
| `rejects_non_apfs_device` | Zeroed device returns `Err` |
| `apfs_ts_zero_is_none` | Zero timestamp → `None` |
| `apfs_ts_positive_converts` | 1 billion ns → Unix 978_307_201 |
| `apfs_ts_negative_is_none` | Negative timestamp → `None` |
| `omap_lookup_finds_entry` | Walk single-entry omap leaf → correct paddr |

---

## Field Offsets Used

| Structure | Field | Offset |
|---|---|---|
| nx_superblock_t | nx_magic | 32 |
| nx_superblock_t | nx_block_size | 36 |
| nx_superblock_t | nx_omap_oid | 168 |
| nx_superblock_t | nx_fs_oid[0] | 232 |
| apfs_superblock_t | apfs_magic | 32 |
| apfs_superblock_t | apfs_omap_oid | 160 |
| apfs_superblock_t | apfs_root_tree_oid | 168 |
| j_inode_val_t | mode | 80 |
| j_inode_val_t | uncompressed_size | 84 |
