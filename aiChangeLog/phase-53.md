# Phase 53 — Write-Blocker Verification (Pre-flight)

## Summary
Moved the write-blocker check from inside the imaging thread (post-start) to
device-selection time (pre-flight).  The check now runs as soon as a drive is
selected, giving the operator an immediate warning before they even configure
the destination path.

## What Changed

### Problem
The previous inline check fired only after the user pressed `s` to start imaging
and the background thread had already been spawned.  The result appeared in the
UI concurrent with the first progress ticks — too late to be a true pre-flight.

### Solution
1. `ferrite-imaging::write_blocker` module: pure `check(device_path) -> bool`
   function extracted from the inline code (same logic, now testable in isolation).
2. `set_device()` spawns a tiny one-shot background thread immediately after the
   device is set.  The result arrives via a dedicated `wb_rx: Option<Receiver<bool>>`
   channel that is drained by `tick()`.
3. The inline write-blocker block inside `start_imaging_forced()` is removed.
4. `self.write_blocked` is no longer reset to `None` when imaging starts — the
   pre-flight result carries forward into the running session.

## Files Changed

### `crates/ferrite-imaging/src/write_blocker.rs` (NEW)
- `check(device_path: &str) -> bool` — attempts `OpenOptions::new().write(true).open()`.
  Returns `true` (blocked/safe) on error, `false` (writable/warn) on success.
  Empty path returns `true` (safe default).
- 5 unit tests: empty path, nonexistent path, writable temp file, read-only file,
  idempotency.

### `crates/ferrite-imaging/src/lib.rs`
- Added `pub mod write_blocker;`.

### `crates/ferrite-tui/src/screens/imaging/mod.rs`
- Removed `ImagingMsg::WriteBlockerResult(bool)` variant (no longer sent by
  imaging thread).
- Added `wb_rx: Option<Receiver<bool>>` field to `ImagingState`.
- `new()`: initialise `wb_rx: None`.
- `set_device()`: resets `wb_rx = None`, spawns one-shot thread that calls
  `write_blocker::check()` and sends result via `wb_rx`.
- `tick()`: drains `wb_rx` before the main imaging channel loop; clears `wb_rx`
  once the result arrives.
- `start_imaging_forced()`: removed `self.write_blocked = None` reset and the
  inline write-blocker open/send block.
- Tests updated: replaced `write_blocker_result_message_sets_state` with three
  new tests (`preflight_wb_rx_sets_write_blocked`, `preflight_wb_rx_not_blocked_sets_false`,
  `preflight_wb_rx_cleared_after_drain`).

## Test Count: 432 → 439 (+7)
