# Phase 111 — Pre-flight Destination Space Check

## Summary
Added a pre-flight free-space check to the Imaging tab so operators are warned
before imaging starts rather than discovering insufficient disk space hours into
a recovery run.

## Changes

### New files
- `crates/ferrite-imaging/src/space_check.rs`
  - `SpaceInfo { available: u64, required: u64 }` with `ratio()` and `sufficient()`
  - `check(dest_path, required) -> Option<SpaceInfo>` — queries free space via
    `fs2::available_space()`; walks up `dest_path` to find the nearest existing
    ancestor (handles paths whose parent directories don't yet exist)
  - 7 unit tests

### Modified files

**`crates/ferrite-imaging/Cargo.toml`**
- Added `fs2 = { workspace = true }` (cross-platform free-space query)

**`crates/ferrite-imaging/src/lib.rs`**
- Added `pub mod space_check`
- Re-exported `SpaceInfo`

**`crates/ferrite-tui/src/screens/imaging/mod.rs`**
- Added `ImagingStatus::ConfirmLowSpace { available, required }` variant
- Added `space_info: Option<SpaceInfo>` field to `ImagingState`
- Added `refresh_space_info()` — computes `required` from device size (or
  LBA range) and calls `space_check::check()`
- `refresh_space_info()` is called from:
  - `set_device()` — after a new device is connected
  - `handle_key()` — when the user exits editing of Dest / StartLba / EndLba
  - `start_imaging()` — after path normalisation and auto-filename, just before
    the space guard
- `start_imaging()` sets `ConfirmLowSpace` when `!sufficient()` then returns
- New `start_imaging_after_space_ok()` method carries the drive-identity check
  and `start_imaging_forced()` call; used by both the normal flow and the
  `ConfirmLowSpace` confirmation handler
- `handle_key()` handles `ConfirmLowSpace`: `Enter/y` → proceed, `Esc/n` → Idle

**`crates/ferrite-tui/src/screens/imaging/render.rs`**
- Config panel `Constraint::Length(13)` → `Constraint::Length(14)` (+1 for space row)
- Added "Dest space" row with colour-coded status:
  - Green  `✓ N free / N required` — sufficient
  - Amber  `⚠ N free / N required` — within 10 % shortfall
  - Red    `✗ N free / N required — insufficient` — clear shortfall
  - Gray   `—` — no device / query failed
- Footer hint switches to space tip when space is insufficient
- Added `ConfirmLowSpace` overlay (yellow modal in progress-bar slot) with
  `Enter/y` to proceed and `Esc/n` to cancel
- Added `fmt_space()` helper (TiB / GiB / MiB / B) and `space_row()` builder

### Clippy fixes (in the same commit)
- `pre_validate.rs:1158` — `frame_len < 7 || frame_len > 8191` →
  `!(7..=8191).contains(&frame_len)`
- `size_hint/adts.rs:67` — same range lint
- `size_hint/adts.rs:96` — test assert range lint

## Tests
- 7 new unit tests in `space_check.rs`
- All existing 1096+ workspace tests still pass
- `cargo clippy --workspace --all-targets -- -D warnings` clean
