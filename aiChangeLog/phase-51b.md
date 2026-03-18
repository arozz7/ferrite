# Phase 51b — SMART Thermal Guard Wired into Imaging Engine

## Summary
Extracts the thermal guard from ad-hoc threading code inside the TUI into a
proper `ferrite_imaging::thermal::ThermalGuard` type.  The guard is now
testable, reusable, and its lifetime is tied to the imaging thread rather than
floating independently.  The TUI's `ImagingState` is simplified by removing the
`pause: Arc<AtomicBool>` field it previously owned.

## Changes

### ferrite-imaging — `src/thermal.rs` (new)

**`ThermalGuardConfig`** (Clone, Debug)
- `pause_above_celsius: u32` — default 55 °C
- `resume_below_celsius: u32` — default 50 °C
- `poll_interval: Duration` — default 60 s

**`ThermalEvent`** (Clone, Copy, PartialEq, Eq)
- `Temperature(u32)` — emitted on every poll with a valid reading
- `Paused` — emitted once when crossing the upper threshold
- `Resumed` — emitted once when crossing the lower threshold

**`ThermalGuard`**
- `start(temp_provider, config, on_event) -> Self` — spawns background poll
  thread.  `temp_provider` is `Fn() -> Option<u32> + Send + 'static` so the
  guard is independent of any particular S.M.A.R.T. backend.
- `pause_flag() -> Arc<AtomicBool>` — clone for passing to `ChannelReporter`.
- `is_paused() -> bool` — current paused state.
- `Drop` — sets an internal cancel flag; the thread exits within one sleep step.

**Sleep granularity fix** — the poll loop sleeps in steps of `min(poll_ms, 1000)`
ms so sub-second `poll_interval` values (used in tests) work correctly.

### ferrite-imaging — `src/lib.rs`
- Added `pub mod thermal`.

### ferrite-tui — `src/screens/imaging/mod.rs`

**`ImagingState` simplification**
- Removed `pause: Arc<AtomicBool>` field (was shared with the thermal thread).
- Removed `pause` initialisation from `new()`.
- Removed `self.pause.store(false, …)` from `set_device()`.
- Removed `self.pause.store(false, …)` and `let pause = Arc::clone(&self.pause)`
  from `start_imaging_forced()`.

**Thermal guard moved into imaging thread**
- The ad-hoc `std::thread::spawn` thermal guard is replaced by
  `ThermalGuard::start(…)` created inside the imaging thread.
- `temp_provider` closure calls `ferrite_smart::query` (unchanged behaviour).
- The `on_event` callback forwards `ThermalEvent` variants to the channel as
  `ImagingMsg::Temperature / ThermalPause / ThermalResume` (unchanged TUI messages).
- `ChannelReporter.pause` is now `guard.pause_flag()`.
- Guard is dropped when `engine.run()` returns, stopping the poll thread cleanly.

## Tests added (`ferrite_imaging::thermal::tests`)
- `default_thresholds_are_sensible` — asserts pause > resume, checks values.
- `cool_drive_never_pauses` — 40 °C provider emits temperature events but no Pause.
- `hot_drive_triggers_pause` — 60 °C provider sets `is_paused()` and emits Paused.
- `pause_then_resume_lifecycle` — starts hot, cools, asserts Paused then Resumed.
- `drop_stops_guard_thread` — event count does not grow after drop.
- `no_event_when_provider_returns_none` — `None` provider emits nothing.
