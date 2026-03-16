# Phase 31 — Original Timestamps on Carved Files

## Summary
Carved files are now stamped with the original file creation/modification
timestamps when the metadata index can resolve the byte offset to a filesystem
entry.  Cross-platform: `filetime::set_file_times` maps to `utimensat(2)` on
Linux and `SetFileTime` on Windows.

---

## Phase 31A — Timestamp Extraction in Filesystem Parsers

### `ntfs_helpers.rs`
- Added `ATTR_STD_INFO: u32 = 0x10` constant.
- Added `parse_standard_info(raw) -> Option<(Option<u64>, Option<u64>)>`:
  parses the resident `$STANDARD_INFORMATION` attribute and converts
  created/modified Windows FILETIME to Unix timestamps via
  `(ft - 116_444_736_000_000_000) / 10_000_000`.  Returns `None` per field
  when FILETIME is zero or predates the Unix epoch.
- **Tests:** `parse_standard_info_extracts_timestamps`,
  `parse_standard_info_zero_filetime_yields_none`,
  `parse_standard_info_missing_returns_none`.

### `ntfs.rs`
- `scan()`: calls `parse_standard_info(&raw)` alongside `parse_file_info`;
  populates `FileEntry::created` / `::modified` instead of `None`.

### `fat32.rs`
- Added `fat_datetime_to_unix(date, time) -> Option<u64>`: converts FAT32
  directory-entry date/time pairs to Unix timestamps.  Returns `None` for
  zero-date or out-of-range values.  FAT epoch offset = 3 652 days from 1970.
- `build_entries()`: reads `DIR_CrtTime/Date` (bytes 14–17) and
  `DIR_WrtTime/Date` (bytes 22–25); populates `FileEntry::created` /
  `::modified`.
- **Tests:** `fat_datetime_zero_date_is_none`, `fat_datetime_known_date`
  (2000-01-01 → 946 684 800), `fat_datetime_time_fields`.

### `ext4.rs`
- `list_inode()`: added post-processing loop that reads each child inode and
  sets `FileEntry::modified` from `i_mtime` (offset +16) and
  `FileEntry::created` from `i_crtime` (offset +144, ext4 extended, only when
  `inode_size >= 148`), falling back to `i_ctime` (+12).  Guards `ino > 0` to
  avoid underflow on deleted entries with `de_inode == 0`.

---

## Phase 31B — Apply Timestamps After Extraction

### `Cargo.toml` (workspace) + `ferrite-tui/Cargo.toml`
- Added `filetime = "0.2"`.

### `extract.rs`
- Added `use ferrite_filesystem::MetadataIndex`.
- Added `apply_timestamps(path, byte_offset, index)`: looks up offset, reads
  `modified.or(created)`, calls `filetime::set_file_times`.  Silent on error.
- `extract_selected()`: captures `self.meta_index.clone()`; calls
  `apply_timestamps` after each successful single-file extraction.
- `start_extraction_batch()`: captures `self.meta_index.clone()`; threads it
  into each worker; calls `apply_timestamps` after each successful batch item.

---

## Test Results
```
cargo test --workspace        →  231 passed, 0 failed
cargo clippy -- -D warnings   →  clean
```
