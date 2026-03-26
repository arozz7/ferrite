# Phase 99 — Unified Thermal Guard with Speed-Based Inference

## Summary

Extended the thermal guard to cover **all three scan engines** (carving, artifact scan, text scan) with both SMART temperature monitoring and speed-based inference.  The guard was also moved from `ferrite-imaging` into `ferrite-core` so every engine can share it without depending on the imaging crate.

---

## Changes

### `crates/ferrite-core/src/thermal.rs` (new)
- Full `ThermalGuard` implementation moved from `ferrite-imaging`
- New `ThermalGuardConfig` fields: `speed_baseline_window`, `speed_throttle_pct`, `speed_sustain`, `speed_sample_interval`
- New `ThermalEvent` variants: `SpeedBaseline(u64)`, `SpeedThrottle`, `SpeedResumed`
- New `ThermalGuard::start` signature: `start(temp_provider, bytes_read: Option<Arc<AtomicU64>>, config, on_event)`
- Guard thread polls at `min(speed_sample_interval, poll_interval)` tick so fast test configs work correctly
- Speed inference: median baseline from first N samples over `speed_baseline_window`; 50% threshold sustained for `speed_sustain`; time-based rest of `10 × speed_sustain` before auto-recovery
- Pause is only triggered when `!pause.load()` to avoid re-triggering during user/back-pressure pauses
- 11 unit tests covering all lifecycle states, speed throttle, brief stall, and edge cases

### `crates/ferrite-core/src/lib.rs`
- Added `pub mod thermal;` and re-exports of `ThermalEvent`, `ThermalGuard`, `ThermalGuardConfig`

### `crates/ferrite-imaging/src/thermal.rs`
- Replaced with a one-line re-export shim: `pub use ferrite_core::thermal::{...};`
- Updated `imaging/mod.rs` call site to pass `None` for `bytes_read` (imaging has its own rate control) and handle new `SpeedThrottle`/`SpeedResumed`/`SpeedBaseline` events

### `crates/ferrite-carver/src/scanner.rs`
- Added `bytes_read: &Arc<AtomicU64>` to `ScanCtx` 5-tuple, `scan_with_progress`, and `scan_streaming`
- `scan_impl` increments `bytes_read` by `read_size` after each chunk

### `crates/ferrite-artifact/src/engine.rs`
- Added `pause: Arc<AtomicBool>` and `bytes_read: Arc<AtomicU64>` to `run_scan`
- Spin-wait on `pause` at the top of each chunk loop (same pattern as other engines)
- `bytes_read.fetch_add(chunk_len)` after each successful `read_chunk`

### `crates/ferrite-textcarver/src/engine.rs`
- Same pattern as artifact engine: `pause` + `bytes_read` params, spin-wait, fetch_add

### `crates/ferrite-tui/src/screens/carving/`
- **`mod.rs`**: Added `CarveMsg::Thermal(ThermalEvent)`, `bytes_read: Arc<AtomicU64>`, `thermal_guard: Option<ThermalGuard>`, `thermal_event: Option<ThermalEvent>` fields; initialized in `new()`, reset in `set_device()`
- **`input.rs`**: `start_scan` resets `bytes_read`/`thermal_guard`/`thermal_event`; creates `ThermalGuard` with SMART closure + shared `bytes_read`; stores guard in `self.thermal_guard`; passes `bytes_read` to `scan_streaming`
- **`events.rs`**: Handles `CarveMsg::Thermal(event)` — `Paused`/`SpeedThrottle` sets `self.pause` and transitions to `Pausing`; `Resumed`/`SpeedResumed` clears pause and restores `Running`
- **`render_progress.rs`**: Shows `[⏸ THERMAL PAUSE — cooling down]` title (amber) in progress gauge when `thermal_event` is `Paused`/`SpeedThrottle`; `🌡 ` prefix in compact status line

### `crates/ferrite-tui/src/screens/artifacts/mod.rs`
- Added `bytes_read: Arc<AtomicU64>`, `pause: Arc<AtomicBool>`, `thermal_guard: Option<ThermalGuard>` fields
- `start_scan` creates `ThermalGuard`, passes `thermal_pause` flag (from `guard.pause_flag()`) and `bytes_read` to `ferrite_artifact::run_scan`

### `crates/ferrite-tui/src/screens/text_scan/mod.rs`
- Same pattern as artifacts

---

## Design Decisions
- **Quick Recover excluded**: scans are short (<1 min) and random-access, not sequential reads; thermal inference would be noise
- **Speed inference algorithm**: 90-second median baseline → 50% threshold → 60-second sustain → pause; 10-minute rest before auto-recovery on rate basis
- **Zero-rate samples**: do not reset `slow_since` (could be back-pressure idle) but also don't start it; only a confirmed-fast rate (≥ threshold) resets the timer
- **`!pause.load()` guard**: prevents thermal speed-trigger from firing during an already-active user or back-pressure pause
