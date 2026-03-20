# Phase 92 — Session Resume: Auto-Re-queue Unextracted Hits

## Problem

When a user resumes a completed carving session with auto-extract enabled, pressing `s`
to start the scan called `self.hits.clear()` before discovering that the resume position
was already at the end of the device.  The checkpoint-loaded hits (including all
`Unextracted` entries) were discarded, the scanner produced zero new hits, and
auto-extraction never fired.  The user was forced to manually select all hits and press `E`.

## Fix

**File:** `crates/ferrite-tui/src/screens/carving/input.rs`

Added a short-circuit path at the top of `start_scan()`, executed before `hits.clear()`:

```
was_resumed = resume_from_byte > 0
scan_complete = start_byte >= device.size()
```

When both are true, `start_scan` now:
1. Skips `hits.clear()` — preserves checkpoint-loaded hits
2. Resets cancel/pause atomics and timing state
3. Sets `status = CarveStatus::Done` immediately
4. Creates fresh `tx`/`rx` channels so extraction results can flow back to the TUI
5. If `auto_extract` is on: collects all `HitStatus::Unextracted` hits, pushes them
   into `auto_extract_queue` (preserving their original indices), then calls
   `pump_auto_extract()` to kick off the first batch

The existing scanner thread is not spawned at all in this path.

## Unchanged Behaviour

- Fresh scans (no prior session) are unaffected — `was_resumed` is `false`.
- Incomplete sessions (scan still mid-drive) are unaffected — `start_byte < device_size`.
- Manual extraction (`E` key) still works regardless of session state.

## Test Results

- `cargo fmt --all -- --check` — pass
- `cargo clippy --workspace --all-targets -- -D warnings` — pass (0 warnings)
- `cargo test --workspace` — 883 tests pass
