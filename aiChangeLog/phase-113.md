# Phase 113 — Partitions Tab: GPT Backup Header + Contention Advisory + Image-File Fallback

## Goal
Three linked improvements to the Partitions tab for safer operation alongside the Imager:
1. **GPT backup header recovery** — use the backup GPT header (last LBA) when the primary header (LBA 1) is unreadable or corrupt.
2. **Imaging contention advisory** — warn when the Imaging tab is actively running on the same device.
3. **Image-file fallback** — read the partition table from the partial `.img` file instead of the physical drive during imaging.

## Files Changed

### `crates/ferrite-partition/src/types.rs`
- Added `pub note: Option<String>` field to `PartitionTable` — carries advisory messages to the TUI.

### `crates/ferrite-partition/src/mbr.rs`
- Added `note: None` to `PartitionTable` literal in `parse()`.

### `crates/ferrite-partition/src/gpt.rs`
- Added `note: None` to `PartitionTable` literal in `parse()`.

### `crates/ferrite-partition/src/reconstruct.rs`
- Added `note: None` to `PartitionTable` literal in `from_scan_hits()`.

### `crates/ferrite-partition/src/lib.rs`
- Extracted `try_gpt_at(device, header_lba, disk_size_lba, sector_size)` helper.
- `read_gpt()` now tries primary header (LBA 1) first; if that fails, tries backup at `disk_size_lba - 1` and sets `tbl.note` on success.
- New test: `gpt_backup_header_used_when_primary_corrupt` — zeroed LBA 1, valid backup at LBA 31, verifies note contains "backup header".

### `crates/ferrite-tui/src/screens/imaging/mod.rs`
- Added `is_actively_imaging() -> bool` — returns true when `status == ImagingStatus::Running`.
- Added `partial_image_path() -> Option<String>` — returns `dest_path` when imaging is active and the file already has data.

### `crates/ferrite-tui/src/screens/partition.rs`
- New fields: `imaging_active: bool`, `fallback_image_path: Option<String>`, `used_image_fallback: bool`.
- New method: `set_imaging_context(active, path)` — called every tick from `app.rs`.
- `start_read()` updated: prefers `FileBlockDevice` on the partial image file when `fallback_image_path` is set and the file has data; falls back to physical device otherwise.
- Render: title shows "(reading from partial image file)" when `used_image_fallback`.
- Render: amber advisory row when `imaging_active && !used_image_fallback`.
- Render: yellow note row below summary when `table.note` is `Some`.
- `render_partition_table` extended with `show_contention_warning` parameter.
- Fixed chunk indices for partition table area (shifted by 2 due to new advisory rows).
- New tests: `set_imaging_context_updates_fields`, `fallback_not_used_when_path_is_none`.
- Fixed existing test `PartitionTable` literals to include `note: None`.

### `crates/ferrite-tui/src/app.rs`
- `tick()`: after `self.imaging.tick()`, propagates imaging state to partition:
  ```rust
  let imaging_active = self.imaging.is_actively_imaging();
  let imaging_path = self.imaging.partial_image_path();
  self.partition.set_imaging_context(imaging_active, imaging_path);
  ```

## Tests
- All workspace tests pass (`cargo test --workspace`)
- Clippy clean (`cargo clippy --workspace --all-targets -- -D warnings`)
- Format clean (`cargo fmt --check`)
