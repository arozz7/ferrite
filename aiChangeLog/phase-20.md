# Phase 20 — Extraction Cancel & Pause/Resume Bug Fixes

## Summary
Two interactive-control bugs in the carving screen's bulk extraction:
1. Pressing `c` to cancel an active extraction appeared to hang — the UI showed
   "Cancelling…" but the operation did not stop for minutes on large files.
2. Pressing `p` to pause during extraction did nothing at all.

## Root Causes

### Cancel hang
`cancel_extraction()` set `extract_cancel` to `true`, but worker threads only
checked that flag *between* jobs — never while `carver.extract()` was streaming
data.  A single large OGG/OLE file being written could hold the thread for
minutes before the flag was ever seen.

### Pause does nothing
`toggle_pause()` matched on `self.status`, which is `CarveStatus::Done` once
scanning finishes.  Extraction does not change `status` — it tracks progress via
`extract_progress`.  The `_ => {}` arm silently consumed the key event.  Even if
the flag had been toggled, worker threads never checked `pause` during extraction.

## Design

### `CancelWriter<W: Write>`
New wrapper struct added to `carving.rs` that implements `Write` by delegating to
an inner writer, but checks `extract_cancel` on every `write()` call:
```rust
fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    if self.cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "extraction cancelled"));
    }
    self.inner.write(buf)
}
```
Workers now pass `CancelWriter { inner: file, cancel }` to `carver.extract()`.
The carver writes in 1 MiB chunks, so cancellation takes effect within one chunk
(≈1 MB of extra I/O) rather than waiting for the whole file.

Cancelled mid-write results are **not** counted as failures in the completion
summary — they are identified by the `"cancelled"` substring in the error string
and logged at `debug` level instead.

### Pause during extraction
`toggle_pause()` gained an early-return branch:
```rust
if self.extract_progress.is_some() {
    let current = self.pause.load(Ordering::Relaxed);
    self.pause.store(!current, Ordering::Relaxed);
    return;
}
```
Worker threads now receive a clone of `self.pause` and spin-wait between jobs:
```rust
while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
    std::thread::sleep(Duration::from_millis(50));
}
```
The `pause` flag is reset to `false` when `ExtractionDone` arrives so a
stuck-paused state cannot carry over to subsequent scans.

### UI feedback
- Progress bar title changes to `" Extracting [PAUSED — p to resume] "` and
  gauge turns yellow while paused.
- Hits panel title updated to `" … p: pause  c: cancel "` during extraction.
- `extract_all_selected()` resets `pause` to `false` before starting so a
  previously paused run never contaminates a fresh batch.

## Changes

### `crates/ferrite-tui/src/screens/carving.rs`
- Added `use std::io::Write;`
- Added `CancelWriter<W: Write>` struct and `Write` impl
- `extract_all_selected()`: captures `Arc::clone(&self.pause)`, resets it to
  `false` before launch, passes clone into each worker; workers spin-wait on
  `pause`; workers wrap output file in `CancelWriter`
- `toggle_pause()`: added extraction-active branch
- `tick()` `ExtractionDone` arm: added `self.pause.store(false, …)`
- `render_extract_progress()`: added paused colour/title/label variants
- Hits panel title: changed "c: cancel extraction" → "p: pause  c: cancel"

## Files Modified
- `crates/ferrite-tui/src/screens/carving.rs`
- `aiChangeLog/phase-20.md` (this file)

## Test Results
- `cargo test --workspace` — 214 tests pass, 0 failures
- `cargo clippy --workspace -- -D warnings` — clean
