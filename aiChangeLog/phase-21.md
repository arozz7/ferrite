# Phase 21 — Extraction Completion Summary

## Summary
When a bulk extraction finished the progress bar simply disappeared with no
confirmation or outcome metrics.  Phase 21 replaces the vanishing bar with a
persistent summary panel that shows exactly how many files succeeded, how many
were truncated, how many failed, total bytes written, elapsed time, and average
throughput.

## Root Cause
`CarveMsg::ExtractionDone` carried no data.  On receipt, `tick()` set
`extract_progress = None`, which removed the only visible indicator without
leaving anything behind.  Users had to count coloured rows in the hit list
manually to gauge success.

## Design

### `ExtractionSummary` struct
```rust
struct ExtractionSummary {
    succeeded: usize,   // files extracted cleanly (footer found / size hint matched)
    truncated: usize,   // files that hit max_size without finding a footer
    failed: usize,      // I/O or creation errors (cancelled writes excluded)
    total_bytes: u64,
    elapsed_secs: f64,
}
```

### Coordinator thread — outcome accounting
The coordinator already iterated `WorkerMsg::Completed` results.  Three counters
(`succeeded`, `truncated_count`, `failed`) were added to that loop:
- `Ok(bytes)` with truncated flag set → `truncated_count += 1`; otherwise `succeeded += 1`
- `Err(e)` where `e` contains `"cancelled"` → logged at `debug`, not counted
- `Err(e)` otherwise → `failed += 1`, logged at `warn`

`Instant::now()` is captured at coordinator-thread entry so elapsed time is
measured from when the coordinator started (includes directory creation and
worker startup).

`CarveMsg::ExtractionDone` promoted to a struct variant carrying all five fields.
The early-exit path (directory creation failure) sends zeroed counts.

### State
`CarvingState` gained `extract_summary: Option<ExtractionSummary>`.
Set in `tick()` when `ExtractionDone` arrives (only when at least one file was
attempted).  Cleared in `set_device()`, at the start of `extract_all_selected()`,
and on `d` key press.

### Rendering — `render_extraction_summary()`
Occupies the same 3-line slot as the progress bar.  Green border and title
`" Extraction Complete "`.  Single content line:

```
  ✓ 142 extracted   ⚠ 3 truncated   ✗ 1 failed   │  1.2 GiB  │  00:42  4.2 MB/s avg   (d to dismiss)
```

- ✓ count: always shown, green bold
- ⚠ truncated count: yellow bold, omitted when zero
- ✗ failed count: red bold, omitted when zero
- Total bytes, elapsed `HH:MM:SS`, average MB/s (omitted if no data)
- `(d to dismiss)` hint in dark grey

### Dismiss / lifecycle
- `d` key clears `extract_summary` immediately
- Starting a new extraction clears it automatically
- `set_device()` clears it on device change

## Changes

### `crates/ferrite-tui/src/screens/carving.rs`
- `CarveMsg::ExtractionDone`: promoted to struct variant with `succeeded`,
  `truncated`, `failed`, `total_bytes`, `elapsed_secs` fields
- Added `ExtractionSummary` struct
- `CarvingState`: added `extract_summary: Option<ExtractionSummary>`
- `CarvingState::new()`: initialises `extract_summary: None`
- `set_device()`: resets `extract_summary`
- `tick()` `ExtractionDone` arm: constructs and stores `ExtractionSummary`
- `handle_key()`: added `'d'` arm to clear `extract_summary`
- `extract_all_selected()`: resets `extract_summary`, adds `extract_start`
  timestamp, tracks `succeeded` / `truncated_count` / `failed`, sends enriched
  `ExtractionDone`; cancelled mid-write errors not counted as failures
- `render_hits_panel()`: renders summary panel in place of progress bar when
  `extract_summary.is_some()` and no extraction is running
- Added `render_extraction_summary()` method

## Files Modified
- `crates/ferrite-tui/src/screens/carving.rs`
- `aiChangeLog/phase-21.md` (this file)

## Test Results
- `cargo test --workspace` — 214 tests pass, 0 failures
- `cargo clippy --workspace -- -D warnings` — clean
