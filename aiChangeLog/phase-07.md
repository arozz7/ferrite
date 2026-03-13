# Phase 07 — ferrite-tui: Terminal UI

## Summary

Implemented `ferrite-tui`, a ratatui 0.29 + crossterm 0.28 terminal UI that integrates
all six prior crates into a single interactive binary.  All heavy operations (device
enumeration, S.M.A.R.T. query, imaging, partition reading, filesystem opening, and
file carving) run on background threads via `std::thread::spawn` + `mpsc` channels so
the 50 ms event loop never blocks.

---

## Crate Added

`crates/ferrite-tui/` — new library crate, consumed by the `ferrite` binary.

### File Map

| File | Lines | Purpose |
|---|---|---|
| `src/lib.rs` | 50 | Entry point: `run()`, panic-hook terminal restore, `SIGNATURES_TOML` const |
| `src/app.rs` | ~200 | `App` struct, event loop, screen dispatch, help bar |
| `src/screens/mod.rs` | 6 | Module declarations |
| `src/screens/drive_select.rs` | ~290 | Screen 1: enumerate devices, select with Enter |
| `src/screens/health.rs` | ~290 | Screen 2: S.M.A.R.T. summary + attributes table |
| `src/screens/imaging.rs` | ~290 | Screen 3: dest/mapfile config + live progress gauge |
| `src/screens/partition.rs` | ~230 | Screen 4: MBR/GPT table + scan for lost partitions |
| `src/screens/file_browser.rs` | ~280 | Screen 5: directory navigation + deleted file toggle |
| `src/screens/carving.rs` | ~290 | Screen 6: signature selection + hit list + extract |

---

## Architecture

```
App (app.rs)
├── DriveSelectState  — background enumeration thread → mpsc → tick()
├── HealthState       — background smartctl thread    → mpsc → tick()
├── ImagingState      — background ImagingEngine::run → mpsc → tick()
│   └── ChannelReporter: ProgressReporter impl that forwards updates + AtomicBool cancel
├── PartitionState    — background read/scan thread   → mpsc → tick()
├── FileBrowserState  — background open_filesystem    → mpsc → tick()
└── CarvingState      — background Carver::scan       → mpsc → tick()
```

The 50 ms poll loop calls `tick()` each frame to drain all channels without blocking.

---

## Key Bindings

| Key | Scope | Action |
|---|---|---|
| `Tab` / `Shift-Tab` | Global | Cycle screens forward / backward |
| `q` | Global | Quit (suppressed while editing a text field) |
| `↑` / `↓` | Most screens | Navigate list selection |
| `r` | Drives, Health, Partitions | Refresh / re-query |
| `Enter` | Drives | Select device; propagates Arc to all screens |
| `Enter` | Files | Open selected directory |
| `Backspace` | Files | Go up one directory level |
| `d` | Imaging | Edit destination path |
| `m` | Imaging | Edit mapfile path |
| `s` | Imaging, Carving | Start operation |
| `c` | Imaging, Carving | Cancel operation |
| `Esc` | Imaging (edit mode) | Confirm / exit text field |
| `o` | Files | Open filesystem on selected device |
| `d` | Files | Toggle deleted file visibility |
| `Space` | Carving | Toggle signature enabled/disabled |
| `e` | Carving | Extract selected hit to current directory |
| `←` / `→` | Carving | Switch focus between Signatures / Hits panels |

---

## Platform Helpers (drive_select.rs)

`platform_enumerate()`, `platform_get_info()`, and `platform_open()` are gated with
`#[cfg(target_os = "windows")]` / `#[cfg(target_os = "linux")]` so the crate compiles
on all targets and degrades gracefully (empty list) on unsupported platforms.

---

## Binary Entry Point

`src/main.rs` now calls `ferrite_tui::run()` instead of printing a placeholder.

---

## Test Results

| Crate | Tests |
|---|---|
| ferrite-blockdev | 14 |
| ferrite-carver | 20 |
| ferrite-core | 2 |
| ferrite-filesystem | 18 |
| ferrite-imaging | 25 |
| ferrite-partition | 27 |
| ferrite-smart | 18 |
| **ferrite-tui** | **20** |
| **Total** | **144** |

All 144 tests pass.  `cargo clippy --workspace -- -D warnings` and `cargo fmt --check`
are both clean.

### New Tests (ferrite-tui)

- `app::tab_forward_wraps` — Tab cycles back to screen 0 after the last screen
- `app::tab_backward_wraps` — BackTab from screen 0 goes to the last screen
- `app::quit_key_sets_flag` — 'q' sets `should_quit`
- `app::screen_count_matches_names` — sanity: 6 entries in SCREEN_NAMES
- `drive_select::navigation_does_not_underflow` — Up at index 0 stays at 0
- `drive_select::navigation_does_not_overflow` — Down at last entry stays there
- `drive_select::fmt_bytes_gib` — 2 GiB formats correctly
- `health::set_device_resets_state` — new device clears selection and data
- `health::attr_scroll_does_not_underflow` — Up at row 0 stays at 0
- `imaging::is_editing_initially_false` — starts outside edit mode
- `imaging::d_key_enters_dest_edit_mode` — 'd' activates Dest field
- `imaging::esc_exits_edit_mode` — Esc closes the field
- `imaging::typing_appends_to_dest_path` — characters build the path string
- `partition::selection_does_not_underflow` — Up at row 0 stays at 0
- `file_browser::initial_state_is_idle` — clean initial state
- `file_browser::set_device_resets_state` — new device clears navigation history
- `carving::builtin_signatures_load` — 10 signatures from `config/signatures.toml`
- `carving::all_signatures_enabled_by_default` — all sigs start enabled
- `carving::space_toggles_signature` — Space enables/disables and toggles back
- `carving::selection_does_not_underflow` — Up at sig 0 stays at 0

---

## Known Limitations (MVP)

- `Carver::scan()` has no cancellation callback; the 'c' key marks cancelled on the
  TUI side but the background thread runs to completion before being discarded.
- File extraction (carving screen 'e') saves to the current working directory.
- `FilesystemParser::list_directory()` is called synchronously; large directories on
  real HDDs may cause a brief UI pause.
- `ImagingPhase::Retry { attempt, max }` is displayed as "Retry" without the counter
  (the counter fields are unused to avoid a dead-code warning in the match arm).
