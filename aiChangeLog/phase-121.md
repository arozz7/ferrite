# Phase 121 — Tier 3 Enhancement (ENH-09)

**Date:** 2026-03-31
**Branch:** master
**Tests:** 1111 passing, 0 failing — clippy clean — fmt clean

---

## Summary

SMART pending-sector count correlated against mapfile unreadable sectors in
the Health tab, giving the operator a quick sanity-check that the imaging
engine is seeing the same bad regions the drive's self-monitoring has flagged.

---

## Changes

### ENH-09 · SMART pending-sector count correlation
**Files:**
- `crates/ferrite-imaging/src/mapfile_io.rs`
- `crates/ferrite-tui/src/screens/health.rs`
- `crates/ferrite-tui/src/app.rs`

**`ferrite-imaging/src/mapfile_io.rs`**
- Added `pub fn count_unreadable_sectors(path: &Path) -> Option<u64>`:
  - Opens the mapfile, calls `parse(f, 0)` (device_size=0 is sufficient for
    counting purposes).
  - Sums `bytes_with_status(NonTrimmed + NonScraped + BadSector)` and divides
    by 512 to get an approximate sector count.
  - Returns `None` when the file is absent or unparseable (midway through a
    write, wrong path, etc.).

**`crates/ferrite-tui/src/app.rs`**
- `tick()` now propagates `imaging.mapfile_path` → `health.mapfile_path` on
  every tick, keeping them in sync without additional messages.

**`crates/ferrite-tui/src/screens/health.rs`**
- `HealthState` gains `pub mapfile_path: Option<String>` (default `None`).
- `render_health_loaded()` receives `mapfile_path: Option<&str>`.
- When SMART attribute 197 (Current Pending Sector Count) is present **and**
  `mapfile_path` is set:
  - Calls `mapfile_io::count_unreadable_sectors()` inline during render.
  - Appends a `Correlation` row to the Summary panel:
    - `SMART pending = N sectors  |  mapfile unreadable = M sectors`
    - Styled **amber + bold** when the two counts are within 10% of each other
      (indicating the sectors the drive flagged are the same ones the engine
      failed to read — a positive match is reassuring).
    - White when the numbers differ significantly (may indicate partial imaging
      or a different failure mode).
    - Dark grey when the mapfile is unavailable.
  - Hidden entirely when attr 197 is absent (NVMe drives, or the attribute is
    not reported by this drive model).

---

## Enhancement Backlog Updates

Items marked `done` in `docs/enhancement-backlog.md`:
- ENH-09 ✅

Remaining open items: ENH-16 through ENH-18 (Tier 4 research items — XL effort).
