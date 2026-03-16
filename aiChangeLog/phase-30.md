# Phase 30 — Code Health: File Splits, DRY, Safety

## Summary
Three-part code-health pass enforcing the 600-line hard limit, eliminating a
duplicated helper, and capping a potentially large heap allocation.

---

## Phase 30a — Eliminate `.unwrap()` on Untrusted Disk Data
- Audited all carver and filesystem crates for `.unwrap()` calls on data from
  untrusted device reads; replaced with safe alternatives (`?`, `unwrap_or`,
  `unwrap_or_else`, explicit `if let`).

---

## Phase 30b — Split Files Exceeding 600-Line Hard Limit

| Original file | Lines before | Action | Result |
|---|---|---|---|
| `ferrite-carver/src/scanner.rs` | 1 479 | Extract I/O helpers → `carver_io.rs`; search helpers → `scan_search.rs` | 504 / 598 / 79 |
| `ferrite-filesystem/src/lib.rs` | 674 | Extract probe/detect → `detect.rs`; OffsetDevice → `offset_device.rs` | 504 |
| `ferrite-filesystem/src/ntfs.rs` | 890 | Extract record helpers → `ntfs_helpers.rs` | 597 / 260 |
| `ferrite-filesystem/src/ext4.rs` | 1 218 | Extract dir parser → `ext4_dir.rs`; test module → `ext4_tests.rs` via `#[path]` | 565 / 60 / 599 |
| `ferrite-tui/src/screens/carving/render.rs` | 734 | Extract progress widgets → `render_progress.rs` | 525 / 222 |
| `ferrite-tui/src/screens/carving/preview.rs` | 789 | Extract ZIP/PDF/SQLite/PE parsers + rendering → `preview_more.rs` | 471 / 321 |
| `ferrite-tui/src/screens/hex_viewer.rs` | 647 | Extract test module → `hex_viewer_tests.rs` via `#[path]` | 517 / 129 |

New files added:
- `ferrite-carver/src/carver_io.rs`
- `ferrite-carver/src/scan_search.rs`
- `ferrite-carver/tests/size_hints.rs`
- `ferrite-filesystem/src/detect.rs`
- `ferrite-filesystem/src/offset_device.rs`
- `ferrite-filesystem/src/ntfs_helpers.rs`
- `ferrite-filesystem/src/ext4_dir.rs`
- `ferrite-filesystem/src/ext4_tests.rs`
- `ferrite-tui/src/screens/carving/render_progress.rs`
- `ferrite-tui/src/screens/carving/preview_more.rs`
- `ferrite-tui/src/screens/hex_viewer_tests.rs`

---

## Phase 30c — DRY + Sparse-Run Allocation Cap

### DRY: `fmt_size` removed from `preview_more.rs`
- `preview_more::fmt_size` was identical to `helpers::fmt_bytes`; deleted
  `fmt_size` and routed the single call site to `super::helpers::fmt_bytes`.

### Safety: NTFS sparse-run write loop capped
- `ntfs_helpers::read_run_list` previously allocated `vec![0u8; zeros_needed]`
  for sparse runs, which could be arbitrarily large for a corrupt or adversarial
  image.
- Added `MAX_SPARSE_ZEROS = 64 KiB` constant; the zero-fill now loops in
  bounded chunks, capping peak allocation regardless of run length.

---

## Phase 30d — Test Coverage: Preview Parsers

Added four unit tests in `preview_more::tests`:
- `parse_zip_extracts_file_names` — verifies filename extraction from a
  synthetic local-file-header.
- `parse_pdf_extracts_title_and_author` — verifies `/Title` and `/Author`
  extraction from a minimal PDF dictionary fragment.
- `parse_sqlite_extracts_page_info` — verifies page-size and encoding fields
  from a 100-byte SQLite header stub.
- `parse_pe_detects_x86_64_architecture` — verifies machine type and section
  count parsing from a synthetic PE32+ header.

---

## Test Results
```
cargo test --workspace   → 231 passed, 0 failed
cargo clippy -- -D warnings → 0 errors
cargo fmt --check        → clean
```
