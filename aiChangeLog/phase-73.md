# Phase 73: Truncation-Skip Option

## Summary
Added `skip_truncated` toggle to the carving screen that automatically deletes
truncated files after extraction and marks them as skipped.

## Changes

### `crates/ferrite-tui/src/screens/carving/mod.rs`
- Added `HitStatus::Skipped` variant
- Added `CarveMsg::Skipped { idx }` message variant
- Added `skipped_trunc: usize` field to `CarveMsg::ExtractionDone` and `ExtractionSummary`
- Added `skip_truncated: bool` and `skipped_trunc_count: usize` to `CarvingState`

### `crates/ferrite-tui/src/screens/carving/extract.rs`
- Added `WorkerMsg::Skipped { idx }` variant
- After post-extraction quality check: if `skip_truncated` and quality is `Truncated`,
  delete the output file and send `Skipped` message instead of `Completed`
- Coordinator accumulates `skipped_trunc` count in `ExtractionDone`
- Applied to both batch extraction and single-file `extract_selected()` paths

### `crates/ferrite-tui/src/screens/carving/events.rs`
- Handle `CarveMsg::Skipped`: set hit status to `Skipped`, increment counter

### `crates/ferrite-tui/src/screens/carving/input.rs`
- `t` key toggles `skip_truncated` on/off

### `crates/ferrite-tui/src/screens/carving/render.rs`
- Display `[SKIP]` in dark gray for skipped hits
- Show `⊘ N skipped (trunc)` in extraction summary

### `crates/ferrite-tui/src/screens/carving/render_progress.rs`
- Status bar shows `t: skip-trunc [ON]` (green) or `t: skip-trunc [off]` (gray)

### `crates/ferrite-tui/src/carving_session.rs`
- Added `skip_truncated: bool` with `#[serde(default)]` for backward compatibility

### `crates/ferrite-tui/src/screens/carving/session_ops.rs`
- Save/restore `skip_truncated` in session persist/restore

## Verification
- `cargo test --workspace` — 747 tests pass
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
