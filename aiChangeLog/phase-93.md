# Phase 93 — FAT32 Quick Recover: Recursive Deleted-File Walk

## Problem

`Fat32Parser::deleted_files()` only scanned the root directory cluster.  Any
file deleted from a subdirectory (e.g. `WINDOWS\`, `WINDOWS\System32\`) was
completely invisible to Quick Recover, which is why the tab showed zero results
on drives that clearly had deleted content.

## Root Cause

```rust
// BEFORE — root only
fn deleted_files(&self) -> Result<Vec<FileEntry>> {
    let raw = self.raw_dir_entries(self.root_cluster);   // root only!
    ...
}
```

## Fix

**File:** `crates/ferrite-filesystem/src/fat32.rs`

Replaced the flat root-only scan with a BFS walk of the entire live directory
tree:

```
queue: [(root_cluster, "")]
while queue not empty:
    pop (cluster, dir_prefix)
    read raw entries (include_deleted = true)
    for each entry:
        if deleted file  → assign full path, score recovery_chance, collect
        if live directory → push (sub_cluster, full_path) onto queue
```

Key properties:
- **Cycle guard** — `HashSet<u32>` of visited clusters prevents infinite loops
  on corrupt FAT chains
- **Fault-tolerant** — `raw_dir_entries` errors on a bad cluster are skipped
  (`continue`) so one damaged directory doesn't abort the whole scan
- **Correct paths** — entries get full paths like `/WINDOWS/System32/foo.dll`
  instead of the flat `/<name>` that `build_entries` produces
- **Deleted dirs excluded** — we don't recurse into deleted directories; their
  cluster numbers may have been reused after deletion

## New Test

`deleted_files_in_subdirectory_found` — builds a 10-sector FAT32 image with:
- Root: `HELLO.TXT` (live) + `GONE.DAT` (deleted) + `SUBDIR/` (live dir)
- `SUBDIR/`: `SUBDEL.TXT` (deleted)

Asserts that `deleted_files()` returns **2** entries and that one of them has
a path containing `SUBDIR`.

## Test Results

- `cargo fmt --all -- --check` — pass
- `cargo clippy --workspace --all-targets -- -D warnings` — pass (0 warnings)
- `cargo test --workspace` — 885 tests pass (69 in ferrite-filesystem, +1 new)
