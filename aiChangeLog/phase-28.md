# Phase 28 — Streaming Extraction Pipeline

## Overview
Replaced the batch-accumulation scanner model with a streaming callback API to eliminate OOM
risk on large drives (~91M hits projected on 4 TB scans).  Added auto-extract mode, disk-space
monitoring, and UI improvements to handle unbounded hit counts gracefully.

## Tasks

### Task 1 — `scan_streaming` callback API (`ferrite-carver`)
- Replaced `scan_inner` with `scan_impl(progress, on_hits)` private method.
- `scan()` wraps `scan_impl` with a collecting closure (backwards-compatible).
- New `scan_streaming(tx, cancel, pause, on_hits)` public method: streams batches via callback
  per chunk; cancel path returns `Ok(())` (partial hits already delivered).
- Per-chunk hits sorted by byte offset before delivery.
- `total_hits` counter replaces `all_hits.len()` for progress reporting.

### Task 2 — `min_size: u64` on `Signature`
- Added `#[serde(default)] pub min_size: u64` to `Signature` and `RawSig`.
- All test `Signature` literals updated with `min_size: 0`.
- `config/signatures.toml` entries without `min_size` default to 0 (disabled).

### Task 3 — Streaming TUI hit intake
- Added `CarveMsg::HitBatch(Vec<CarveHit>)` replacing `Done(Vec<CarveHit>)`.
- `Done` is now a unit completion signal.
- `DISPLAY_CAP = 100_000`: TUI stores at most 100k hits in `self.hits`.
- `total_hits_found: usize` counts all hits including those beyond the cap.
- Checkpoint flush every 1 000 new displayable hits during scan.

### Task 4 — Auto-extract mode
- `auto_extract: bool` state field; toggled with `x` key.
- `auto_extract_queue: VecDeque<(usize, CarveHit, String)>` — `usize::MAX` sentinel for
  hits beyond `DISPLAY_CAP`.
- `pump_auto_extract()`: drains queue in batches of 500 via `start_extraction_batch()`.
- `start_extraction_batch(work)`: shared coordinator thread used by both manual `E` and
  auto-extract pipeline; concurrency capped at 2–8 threads.
- `usize::MAX` indices silently ignored in `ExtractionStarted`/`Extracted` handlers.
- `ExtractionDone` event re-triggers `pump_auto_extract()` to continue the pipeline.

### Task 5 — Disk-space monitoring
- Added `fs2` workspace dependency.
- `disk_avail_bytes: Option<u64>` polled every ~50 ticks (~5 s at 10 fps) via
  `poll_disk_space()` using `fs2::available_space()`.
- Walks up to nearest existing parent if output dir not yet created.

### Task 6 — UI updates (`render.rs`)
- Layout: added `render_disk_auto_bar` row between scan-range bar and main panels.
- `render_disk_auto_bar`: shows disk free space (red ⚠ warning below 10 GiB) and
  auto-extract toggle hint (`x: auto-extract [ON/off]`).
- `render_compact_scan_progress`: 1-line compact scan status (%, hits, rate) shown inside
  the hits panel while scanning with hits present.
- Hits panel title: `"{n} of {total} total"` when cap exceeded; `[AUTO-EXTRACT]` badge.
- `render_hits_panel` rewritten for clean layout with `after_scan` / `after_extract` /
  `list_area` intermediate variables.

### Task 7 — JSONL checkpoint
- Existing checkpoint module (`checkpoint::append`) logs hits as JSONL during scan.
- Outcome log (`append_outcome`) deferred to Phase 29.

## Files Modified
| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | `fs2 = "0.4"` workspace dep |
| `crates/ferrite-tui/Cargo.toml` | `fs2` dependency |
| `crates/ferrite-carver/src/signature.rs` | `min_size` field on `Signature` / `RawSig` |
| `crates/ferrite-carver/src/scanner.rs` | `scan_impl` + `scan_streaming`; `min_size` in tests |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `DISPLAY_CAP`, `HitBatch`, new state fields |
| `crates/ferrite-tui/src/screens/carving/input.rs` | `scan_streaming` call; `x` key toggle |
| `crates/ferrite-tui/src/screens/carving/events.rs` | `HitBatch` handler; disk-space poll |
| `crates/ferrite-tui/src/screens/carving/extract.rs` | `start_extraction_batch`; `pump_auto_extract` |
| `crates/ferrite-tui/src/screens/carving/render.rs` | `render_disk_auto_bar`; `render_compact_scan_progress`; hits panel rewrite |

## Test Results
- `cargo test --workspace` — all tests pass
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — (run before commit)
