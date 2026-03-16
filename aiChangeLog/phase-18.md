# Phase 18 — Filename Recovery via Filesystem Metadata Correlation

## Summary
After carving, extracted files were named `ferrite_jpg_0x1a0000.jpg` (type +
offset), discarding the original filename entirely.  Phase 18 cross-references
carve hits with the filesystem metadata on the same device so recovered files
are written with their original names whenever possible.

## Root Cause
The carving engine works at the raw-block level and has no knowledge of
filesystems.  It identifies file boundaries by header/footer magic bytes but
cannot know what a file was called.  However, every filesystem (NTFS, FAT32,
ext4) stores both the filename *and* the byte offset of the file's first data
cluster in its own metadata tables.  By indexing those offset→name mappings
we can match carve hits back to their original filenames.

## Design

### `FileEntry.data_byte_offset: Option<u64>`
Added to the `FileEntry` struct in `ferrite-filesystem`.  Each parser now
computes the absolute byte offset (relative to the start of the volume device)
of a file's first data byte:

| Parser | Source |
|--------|--------|
| NTFS   | First LCN from the DATA attribute non-resident run-list × cluster_size |
| FAT32  | `data_offset + (first_cluster − 2) × cluster_size` |
| ext4   | First physical block from extent tree or legacy `i_block[0]` × block_size |

Resident files (NTFS tiny files stored inside the MFT record itself) and
directories always receive `None`.

### `MetadataIndex`
New type in `ferrite-filesystem`:
```rust
pub struct MetadataIndex {
    entries: HashMap<u64, FileMetadata>,
}
```
`FileMetadata` carries `name`, `path`, `size`, `is_deleted`, `created`,
`modified`.  `MetadataIndex::lookup(byte_offset)` does O(1) lookup.

### `build_metadata_index(device)`
Free function that:
1. Probes MBR/GPT to collect all partition byte offsets (plus offset 0).
2. Wraps each partition in an `OffsetDevice` so filesystem parsers always see
   offset 0.
3. Calls `parser.enumerate_files()` on each recognised filesystem.
4. For every `FileEntry` that has `data_byte_offset = Some(vol_off)`, inserts
   `FileMetadata` keyed by `part_offset + vol_off` (absolute device offset).

### `FilesystemParser::enumerate_files()`
New trait method (default: merges `root_directory()` + `deleted_files()`).
`NtfsParser` overrides it to scan **all** MFT records (not just root children)
by calling `self.scan(|_, _, is_dir| !is_dir)`.

## Changes

### `crates/ferrite-filesystem/src/lib.rs`
- `FileEntry`: added `pub data_byte_offset: Option<u64>` field
- Added `FileMetadata`, `MetadataIndex`, `build_metadata_index()`
- Added `FilesystemParser::enumerate_files()` default impl
- Added `partition_byte_offsets_mbr()`, `partition_byte_offsets_gpt()` helpers
- Added `OffsetDevice` wrapper for transparent partition access
- Removed stale `pub use self::…` re-export
- New tests: `metadata_index_lookup_and_empty`, `metadata_index_insert_and_lookup`,
  `build_metadata_index_on_empty_device_returns_empty`

### `crates/ferrite-filesystem/src/ntfs.rs`
- `parse_file_info()`: signature extended to `Option<(String, u64, u64, Option<u64>)>`;
  now also extracts first LCN from the non-resident DATA attribute run-list
- Added `first_lcn_from_run_list(run_list: &[u8]) -> Option<u64>` helper
- `scan()`: computes `data_byte_offset = first_lcn.map(|l| l * cluster_size)` for
  non-directory entries
- `NtfsParser`: added `enumerate_files()` override for full-MFT scan
- New tests: `resident_file_data_byte_offset_is_none`,
  `first_lcn_from_run_list_single_run`, `first_lcn_from_run_list_sparse_returns_none`,
  `first_lcn_from_run_list_empty_returns_none`

### `crates/ferrite-filesystem/src/fat32.rs`
- `build_entries()`: computes `data_byte_offset` using `data_offset + (cluster − 2) × cluster_size`;
  `None` for directories and clusters < 2
- All `FileEntry` literals: added `data_byte_offset` field
- New tests: `data_byte_offset_for_regular_file`, `data_byte_offset_for_directory_is_none`

### `crates/ferrite-filesystem/src/ext4.rs`
- Added `Ext4Parser::first_data_block_byte_offset(inode_num) -> Option<u64>`:
  reads inode, walks extent tree or legacy `i_block[0]`, returns `block × block_size`
- `list_inode()`: enriches non-dir, non-deleted entries with `data_byte_offset`
  after `parse_dir_block()` returns
- `parse_dir_block()`: `FileEntry` literals updated with `data_byte_offset: None`
- Test literals (`double_indirect_file_read`, `triple_indirect_file_read`): added
  `data_byte_offset: None`

### `crates/ferrite-tui/src/screens/carving.rs`
- Added `CarveMsg::MetadataReady(MetadataIndex)` variant
- `CarvingState`: added `meta_index: Option<Arc<MetadataIndex>>` and
  `meta_index_building: bool`
- When `CarveMsg::Done` is received: spawns background thread that calls
  `build_metadata_index(device)` and sends back `MetadataReady`
- `tick()`: handles `MetadataReady` by storing `Arc<MetadataIndex>` and clearing flag
- Added `filename_for_hit(hit, dir) -> String`: looks up metadata index first,
  falls back to generic `ferrite_{ext}_{offset}.{ext}` name
- `extract_selected()` and `extract_all_selected()`: use `filename_for_hit()`
- Hit list display: appends ` (original_name)` when metadata index has a match
- Status bar: shows "done · building filename index…" while index is being built
- `set_device()`: resets `meta_index` and `meta_index_building`
- `concurrency` calculation: replaced `.min(8).max(2)` with `.clamp(2, 8)` (clippy)

### `crates/ferrite-tui/src/screens/file_browser.rs`
- Test `FileEntry` literal: added `data_byte_offset: None`

## Files Modified
- `crates/ferrite-filesystem/src/lib.rs`
- `crates/ferrite-filesystem/src/ntfs.rs`
- `crates/ferrite-filesystem/src/fat32.rs`
- `crates/ferrite-filesystem/src/ext4.rs`
- `crates/ferrite-tui/src/screens/carving.rs`
- `crates/ferrite-tui/src/screens/file_browser.rs`
- `aiChangeLog/phase-18.md` (this file)

## Test Results
- `cargo test --workspace` — 205 tests pass, 0 failures (9 new tests added)
- `cargo clippy --workspace -- -D warnings` — clean
