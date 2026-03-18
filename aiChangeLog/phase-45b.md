# Phase 45b — Quick Deleted-File Recovery Mode

## Summary
Added a dedicated "Quick Recover" tab (index 7) for fast recovery of deleted files
from healthy drives without requiring a full imaging pass.

## Changes

### ferrite-filesystem
- `src/lib.rs` — Added `RecoveryChance` enum (`High`/`Medium`/`Low`/`Unknown`) with `Ord` derive
- `src/lib.rs` — Added `recovery_chance: RecoveryChance` field to `FileEntry`
- `src/ntfs.rs` — Score deleted files in `deleted_files()`: High if `data_byte_offset` + `size` intact, Low if size > 0 but no offset, Unknown otherwise
- `src/fat32.rs` — Score deleted files in `deleted_files()`: Medium if `first_cluster >= 2` (start cluster readable), Low if cluster zeroed, Unknown if empty
- `src/ext4.rs` — Score deleted files in `deleted_files()`: High if `data_byte_offset` + `size` intact, Low if size > 0 but no offset, Unknown otherwise
- `src/ext4_dir.rs` — Added `recovery_chance: RecoveryChance::Unknown` to `FileEntry` construction
- `src/ext4_tests.rs` — Updated both `FileEntry` constructions to include new field

### ferrite-tui
- `src/screens/quick_recover.rs` — New screen module:
  - `QuickRecoverState` struct with device/parser state, multi-select, filter, recovery progress
  - `TargetedParser` wrapper to reuse `spawn_recovery_thread` with a targeted file list
  - Background load thread: detect FS, open parser, enumerate deleted files
  - Sort: High chance first, then Medium, Low, Unknown; within group by name
  - Keybindings: `↑/↓` navigate, `Space` check, `a` check-high, `A` check-all, `Esc` clear, `R` recover, `/` filter
  - Rendering: collapsible filter bar, chance-colored table, gauge progress, summary footer
  - Tests: `RecoveryChance` ordering, `set_device` no-panic, `sort_entries`, `fmt_bytes`, date formatting
- `src/screens/mod.rs` — Added `pub mod quick_recover`
- `src/app.rs` — Added `QuickRecoverState` as 8th tab (index 7):
  - `SCREEN_NAMES` extended to `[&str; 8]`
  - `App` struct gains `pub quick_recover: QuickRecoverState`
  - `App::new()` initialises field
  - Device selection block calls `quick_recover.set_device()` (both normal select and session resume)
  - `tick()` calls `quick_recover.tick()`
  - `render()` routes screen 7 to `quick_recover.render()`
  - `handle_key()` routes screen 7 to `quick_recover.handle_key()` and `is_editing()`
  - `help_line()` handles screen 7
  - Screen-count test updated from 7 to 8

## Recovery Chance Heuristic
The scoring is conservative and honest:
- **High**: cluster/block run-list intact AND `data_byte_offset` resolved — parser can likely read
  the file now without any special carving.
- **Medium**: start cluster/inode known but run-list incomplete (FAT32 chain zeroed after deletion).
  Recovery possible if the cluster was not reallocated.
- **Low**: no locator info available — would require carving by signature to recover.
- **Unknown**: directory, empty file, or assessment not performed.

## Tests Added
- `RecoveryChance` ordering: `High < Medium < Low < Unknown`
- `set_device` with no parseable FS does not panic (thread returns `LoadMsg::Error`, tick returns to `Idle`)
- `sort_entries` puts High-chance entries first
- `fmt_bytes` for all ranges (B / KiB / MiB / GiB)
- `format_unix_date` epoch and a known timestamp
- `screen_count_matches_names` updated to assert 8 screens
