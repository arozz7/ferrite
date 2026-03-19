# Phase 81 — Skip-corrupt extraction mode

## Summary
Adds a `skip_corrupt` toggle (Shift+C) that automatically deletes files
post_validate marks as `Corrupt` during extraction.  This eliminates
fragment files left behind by footer-based carving — e.g. ZIP/EPUB entries
whose EOCD central directory offset points beyond the extracted file.

## Changes

### 1. New `skip_corrupt` toggle — `CarvingState`
- `skip_corrupt: bool` — user preference, persisted across device changes
  and session save/restore.
- `skipped_corrupt_count: usize` — running counter, reset on device change.

### 2. Extraction enforcement — `extract.rs`
- **Single extraction path:** after `post_validate`, if `Corrupt` +
  `skip_corrupt` → delete file, send `SkippedCorrupt` message.
- **Batch extraction path:** same logic in the worker loop; separate
  `skipped_corrupt` counter forwarded to `ExtractionDone`.

### 3. New message variants
- `CarveMsg::SkippedCorrupt { idx }` — parallel to `CarveMsg::Skipped`.
- `WorkerMsg::SkippedCorrupt { idx }` — internal batch worker variant.

### 4. Event handling — `events.rs`
- Handles `CarveMsg::SkippedCorrupt` — sets `HitStatus::Skipped` on the
  entry and increments `skipped_corrupt_count`.
- `ExtractionDone` now carries `skipped_corrupt` field; summary
  accumulates it in auto-extract mode.

### 5. UI — `input.rs` + `render_progress.rs` + `render.rs`
- `Shift+C` toggles `skip_corrupt` (parallel to `t` for skip-truncated).
- Options bar shows `C: skip-corrupt [ON/off]` next to skip-trunc toggle.
- Extraction summary shows `⊘ N skipped (corrupt)` when count > 0.

### 6. Session persistence — `carving_session.rs` + `session_ops.rs`
- `CarvingSession.skip_corrupt: bool` — `#[serde(default)]` for backward
  compatibility with existing session files.
- `build_session` / `restore_from_session` round-trip the new field.

### 7. `ExtractionSummary` — `mod.rs`
- Added `skipped_corrupt: usize` field.

## Files Modified
- `crates/ferrite-tui/src/screens/carving/mod.rs` — state, messages, summary
- `crates/ferrite-tui/src/screens/carving/extract.rs` — enforcement + WorkerMsg
- `crates/ferrite-tui/src/screens/carving/events.rs` — message handling
- `crates/ferrite-tui/src/screens/carving/input.rs` — Shift+C key binding
- `crates/ferrite-tui/src/screens/carving/render.rs` — summary span
- `crates/ferrite-tui/src/screens/carving/render_progress.rs` — toggle display
- `crates/ferrite-tui/src/screens/carving/session_ops.rs` — session round-trip
- `crates/ferrite-tui/src/carving_session.rs` — skip_corrupt field

## Test Results
- **760 tests passing**, clippy clean, fmt clean.
