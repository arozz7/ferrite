# Phase 25 — imaging split + async carving preview

## Summary

Two changes in this phase:

1. **Split `imaging.rs` (964 lines) into `imaging/{mod,render}.rs`** to fix the hard-limit violation (600 lines max).
2. **Moved `read_preview()` off the UI thread** in the carving screen — it now runs in a background thread, preventing frame stalls on slow devices.

---

## Task 1 — `imaging.rs` → `imaging/{mod,render}.rs`

### Motivation

`imaging.rs` had grown to 964 lines across phases 22–24, well beyond the 600-line hard limit.  The render methods (`render()` + `render_sector_map()`) were a natural split boundary, mirroring the existing `carving/{mod,render}.rs` structure.

### Changes

#### `crates/ferrite-tui/src/screens/imaging.rs` (deleted)
Replaced by the two files below.

#### `crates/ferrite-tui/src/screens/imaging/mod.rs` (501 lines)
All types, state, and logic:
- `ImagingMsg` enum (private)
- `ImagingStatus` → `pub(crate)` (needs to be visible from child `render` module)
- `EditField` → `pub(crate)` (same reason)
- `ChannelReporter` struct + `ProgressReporter` impl (private)
- `ImagingState` struct — five previously-private fields promoted to `pub(crate)`:
  `edit_field`, `device`, `status`, `latest`, `sector_map`
- All `ImagingState` logic methods: `new`, `set_device`, `is_editing`, `tick`, `handle_key`, `field_mut`, `start_imaging`, `cancel_imaging`
- `compute_sha256()` free function (used in `start_imaging`)
- Full test suite (5 tests)
- `mod render;` declaration

#### `crates/ferrite-tui/src/screens/imaging/render.rs` (471 lines)
Render-only code as an `impl ImagingState` block:
- `pub fn render(&mut self, frame, area)` — full config panel, progress bar, sector map, stats, write-blocker line
- `fn render_sector_map(&self, frame, area)` — coloured block grid
- `fn fmt_bytes(n: u64) -> String` — moved here from `mod.rs` since it is only called in render

Ratatui and `ferrite_imaging::mapfile::BlockStatus` imports moved here from `mod.rs`.

---

## Task 2 — `read_preview()` background thread

### Motivation

`read_preview()` performs a synchronous 64 KiB device read on the UI thread.  On a slow or degraded drive this blocks ratatui's render loop, causing dropped frames.

### Changes

#### `crates/ferrite-tui/src/screens/carving/mod.rs`

##### New `CarvingState` fields
- `preview_rx: Option<mpsc::Receiver<Option<preview::HitPreview>>>` — single-slot bounded channel from the loader thread.
- `pub(crate) preview_loading: bool` — `true` while a thread is in-flight; used by render for the indicator message.

##### `new()`
Initialises both new fields to `None` / `false`.

##### `set_device()`
Resets `preview_rx = None; preview_loading = false`.

##### `handle_key()` (`v` toggle-off branch)
Also clears `preview_rx` and `preview_loading` so a stale in-flight request is dropped.

##### `refresh_preview()`
Replaced the synchronous `read_preview()` call with an async pattern:
1. Set `preview_hit_idx` immediately (guards against duplicate requests on rapid navigation).
2. Clear `current_preview`; set `preview_loading = true`.
3. Create a `sync_channel(1)` and store `rx` in `self.preview_rx`.
4. Spawn thread: calls `preview::read_preview()`, sends result through `tx`.

##### `tick()`
- Main carving loop converted from `return` to `break 'carve` so execution continues after the loop.
- New preview drain block after the loop: `try_recv()` on `preview_rx`; on `Ok` stores `current_preview`, clears `preview_loading` and drops the receiver; on `Disconnected` also clears the loading state.

#### `crates/ferrite-tui/src/screens/carving/render.rs`

The `else` branch of the preview panel (when `current_preview` is `None`) now checks `self.preview_loading`:
- `true`  → `" Loading preview…"` (dim gray)
- `false` → `" No preview available."` (dim gray — shown when preview failed or format unsupported)

---

## File Size Summary (after phase)

| File | Lines |
|---|---|
| `imaging/mod.rs` | 501 |
| `imaging/render.rs` | 471 |
| `carving/mod.rs` | 1 194 ⚠ (pre-existing; target for Phase 26 split) |

## Test Results

- `cargo test --workspace` — 219 tests pass, 0 failures
- `cargo clippy --workspace -- -D warnings` — clean
