# Phase 2: Disk Imaging Engine

**Date:** 2026-03-10
**Status:** Complete

## Summary

Implemented `ferrite-imaging` — the multi-pass ddrescue-style imaging engine.
Covers the full 5-pass algorithm, GNU ddrescue-compatible mapfile persistence,
and trait-based progress reporting with cancellation support.

## Files Created

```
NEW  crates/ferrite-imaging/Cargo.toml
NEW  crates/ferrite-imaging/src/lib.rs
NEW  crates/ferrite-imaging/src/error.rs        — ImagingError, Result<T>
NEW  crates/ferrite-imaging/src/config.rs       — ImagingConfig
NEW  crates/ferrite-imaging/src/progress.rs     — Signal, ImagingPhase, ProgressUpdate, ProgressReporter, NullReporter
NEW  crates/ferrite-imaging/src/mapfile.rs      — BlockStatus, Block, Mapfile (core state machine)
NEW  crates/ferrite-imaging/src/mapfile_io.rs   — parse/serialize ddrescue format, load_or_create, save_atomic
NEW  crates/ferrite-imaging/src/engine.rs       — ImagingEngine, run() orchestration
NEW  crates/ferrite-imaging/src/passes/mod.rs
NEW  crates/ferrite-imaging/src/passes/copy.rs  — Pass 1: large-block forward copy
NEW  crates/ferrite-imaging/src/passes/trim.rs  — Pass 2: sector-by-sector, stop on first failure → NonScraped
NEW  crates/ferrite-imaging/src/passes/sweep.rs — Pass 3: sector-by-sector NonTried blocks
NEW  crates/ferrite-imaging/src/passes/scrape.rs — Pass 4: every NonScraped sector independently
NEW  crates/ferrite-imaging/src/passes/retry.rs — Pass 5: alternating-direction retries up to max_retries
MOD  Cargo.toml                                 — added ferrite-imaging member
```

## Key Design Decisions

- **Mapfile as sorted Vec**: `update_range` uses `partition_point` for O(log n)
  boundary lookup, splices in prefix/new/suffix blocks, then calls `merge_adjacent`.
  Counts are recomputed per update (O(n)) — acceptable for typical block counts.
- **Passes as free functions**: Each pass receives `&mut ImagingEngine`, avoiding
  the borrow issue of pre-allocated buffers on engine fields. Per-pass allocation
  (5 total) has negligible cost.
- **Cancellation via Signal**: `ProgressReporter::report()` returns `Signal::Continue`
  or `Signal::Cancel`. The engine checks after every sector/chunk.
- **Atomic mapfile saves**: write to `<path>.tmp`, rename. Configurable interval
  (default 30 s). Set `Duration::MAX` in tests to disable.
- **GNU ddrescue interop**: mapfile text format is fully compatible — ferrite can
  hand off to ddrescue and resume, or vice versa.
- **Retry direction alternates**: odd retry attempts iterate sectors in reverse,
  matching ddrescue behavior for marginal head-alignment recovery.

## Verification

- `cargo test --workspace`: 41 tests pass (25 ferrite-imaging + 14 ferrite-blockdev + 2 ferrite-core)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean

## Engine integration tests

| Test | Scenario |
|---|---|
| `clean_device_all_finished` | No errors → 100% Finished |
| `single_always_fail_sector_becomes_bad` | 1 bad sector → 1 BadSector, rest Finished |
| `bad_sector_recovers_with_fail_first_n` | FailFirstN(4) → recovers on retry 2 |
| `multiple_bad_sectors` | 2 bad sectors at offsets 2 and 7 |
| `all_bad_sectors` | All 16 sectors fail → all BadSector |
| `output_content_matches_source` | Sector content byte-by-byte verified in output file |
| `cancellation_returns_error` | Reporter returns Cancel → Err(Cancelled) |
| `resume_skips_finished_blocks` | All-Finished mapfile → copy pass finds no work |
