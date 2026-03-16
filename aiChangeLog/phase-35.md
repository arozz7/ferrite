# Phase 35 — Filesystem-Assisted Recovery

## Goal
Extract deleted (and live) files with their **original folder structure** preserved,
using filesystem metadata (MFT run-lists for NTFS, cluster chains for FAT32, extent
trees for ext4) rather than raw carving.

## What already existed (no changes required)
| Item | Location |
|---|---|
| `FilesystemParser::read_file(entry, writer)` | `ferrite-filesystem/src/lib.rs` |
| `FilesystemParser::enumerate_files()` | `ferrite-filesystem/src/lib.rs` |
| `FileEntry.{path, created, modified}` | `ferrite-filesystem/src/lib.rs` |
| `filetime` workspace dependency | `Cargo.toml` |

## New files
| File | Purpose |
|---|---|
| `crates/ferrite-tui/src/screens/fs_recovery.rs` | Recovery helpers: `RecoveryMsg`, `RecoveryProgress`, `extract_to_recovered()`, `spawn_recovery_thread()` |

## Modified files
| File | Change |
|---|---|
| `crates/ferrite-tui/src/screens/file_browser.rs` | Phase 35 enhancements (see below) |
| `crates/ferrite-tui/src/screens/mod.rs` | Added `pub mod fs_recovery` |

## Changes to `file_browser.rs`

### Storage
- `parser` type changed from `Box<dyn FilesystemParser>` → `Arc<dyn FilesystemParser>`
  so it can be shared with the background recovery thread without cloning.
- New fields: `recovery_rx`, `recovery_progress`, `recovery_cancel`

### New keybindings
| Key | Action |
|---|---|
| `R` (shift) | Recover all deleted files → `recovered/fs/<original_path>` |
| `Esc` | Cancel an in-progress batch recovery |
| `e` (unchanged) | Extract single selected file → `recovered/fs/<original_path>` |

### Enhanced `extract_selected()`
Previously wrote to the current working directory using just the filename.
Now writes to `recovered/fs/<entry.path>`, creating parent directories and
setting mtime/atime from `FileEntry.{modified, created}`.

### New `recover_all_deleted()`
Spawns a background thread via `spawn_recovery_thread()` that:
1. Calls `parser.enumerate_files()` and filters for `is_deleted && !is_dir`
2. Extracts each file, emitting `RecoveryMsg::Progress` updates per file
3. Emits `RecoveryMsg::Done { succeeded, failed }` when complete
4. Checks an `Arc<AtomicBool>` cancel flag between files

### Recovery progress UI
When a recovery is running, the bottom section of the Files tab expands to two rows:
- Row 1: green `Gauge` showing `done / total` with error count
- Row 2: current file path in grey

After recovery finishes, shows a summary: `"N saved, M failed. Output: recovered/fs/"`.

### Path traversal safety
`extract_to_recovered()` sanitises `FileEntry.path`:
- Strips leading `/` and `\`
- Splits on both separators
- Filters out empty segments and `..` components

## Output structure
```
recovered/
  fs/
    Photos/
      IMG_001.jpg          ← original path preserved
      vacation/
        DSC_042.jpg
    Documents/
      resume.docx [deleted]
```

## Test summary
- 7 total new tests (5 in `file_browser`, 2 in `fs_recovery`)
- All 59 `ferrite-tui` tests pass
- `cargo clippy --workspace -- -D warnings`: clean
- `cargo fmt --check`: clean
