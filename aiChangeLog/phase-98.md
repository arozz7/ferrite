# Phase 98 — Imaging Engine Hardening

## Summary
Performance, robustness, and minor hygiene improvements to `ferrite-imaging`.
No behavioral changes to the 5-pass algorithm itself.

---

## Task A1 — `merge_and_recount`: single-pass merge + count

**File:** `crates/ferrite-imaging/src/mapfile.rs`

Replaced the two-step `merge_adjacent()` (allocates new Vec, scans blocks) +
`recompute_counts()` (scans blocks again) with a single `merge_and_recount()`
that merges adjacent same-status blocks and accumulates byte counts in one O(n)
loop. Eliminates one full Vec allocation and one extra scan per `update_range()`
call — relevant on heavily damaged drives where sector-by-sector passes call
`update_range()` millions of times.

Also inlined the count accumulation into `from_blocks()` directly rather than
calling the old `recompute_counts()`.

---

## Task A2 — Throttle `Instant::elapsed()` in `make_progress()`

**File:** `crates/ferrite-imaging/src/engine.rs`

`make_progress()` is called after every sector read. Previously it called
`Instant::elapsed()` unconditionally on every call. Now the call is gated
behind `snapshot_counter % RATE_CHECK_INTERVAL == 1` (every 100 ticks),
reducing syscall frequency during the sector-by-sector scrape/trim/retry passes.

---

## Task A3 — Named constants

**File:** `crates/ferrite-imaging/src/engine.rs`

Replaced magic literals with named constants:
- `SNAPSHOT_INTERVAL: u32 = 50` — ticks between block-list snapshots
- `RATE_CHECK_INTERVAL: u32 = 100` — ticks between `elapsed()` calls
- `RATE_UPDATE_MIN_SECS: f64 = 1.0` — minimum seconds before rate update fires

---

## Task B1 — Elevate read errors from `debug!` to `warn!`

**Files:** `passes/copy.rs`, `passes/trim.rs`, `passes/sweep.rs`,
`passes/scrape.rs`, `passes/retry.rs`

Hardware read failures (marking sectors NonTrimmed, BadSector) are now logged
at `warn!` level instead of `debug!`. These indicate real events on a damaged
drive and must be visible at the default log verbosity. Successful reads remain
at `debug!`.

---

## Task B2 — `flush_output()` after each writing pass

**File:** `crates/ferrite-imaging/src/engine.rs`

Added `flush_output()` helper that calls `self.output.flush()` (flushes
user-space write buffer to the OS kernel cache). Called in `run()` after each of
the four writing passes: Copy, Trim, Sweep, Scrape. Reduces the risk of
mapfile/image divergence if the process is killed between passes.

---

## Task B3 — Output file lock (`.lock` sidecar)

**Files:** `crates/ferrite-imaging/src/engine.rs`,
`crates/ferrite-imaging/src/error.rs`

`ImagingEngine::new()` now creates `<output_path>.lock` with
`OpenOptions::create_new(true)`. If the file already exists (another session
holds the lock), it returns `ImagingError::OutputLocked { path }`.

`ImagingEngine` implements `Drop` to remove the `.lock` sidecar on engine
destruction (normal completion, cancellation, or panic unwind).

New error variant: `ImagingError::OutputLocked { path: PathBuf }` with a
descriptive message surfaced directly through the existing TUI error path.

New tests:
- `second_engine_on_same_output_returns_locked` — asserts `OutputLocked` when
  two engines target the same file concurrently
- `lock_released_after_engine_drop` — asserts a new engine can be created after
  the first is dropped

Updated `resume_skips_finished_blocks` test to drop the first engine before
creating the second (previously they coexisted, now correctly blocked by the
lock).

---

## Task C1 — Retry pass: clarify reverse-direction loop invariant

**File:** `crates/ferrite-imaging/src/passes/retry.rs`

Added a comment block above the per-region loop explaining:
- Forward (even attempts): `pos` starts at `region.pos`, advances by `chunk`
- Reverse (odd attempts): `pos` starts at `region.end() - sector_size`, retreats
  by `chunk`, exits when `pos < region.pos + chunk`
- Why `chunk = (region.end() - region.pos).min(sector_size)` covers the
  sub-sector-size edge case

---

## Test results
- `cargo test --workspace`: 903 passed, 0 failed (up from 889; +2 new engine tests,
  +existing count growth from other crates)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all`: clean
