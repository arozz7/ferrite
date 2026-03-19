# Phase 88 — Imaging UX Hardening

## Problem addressed

Users experienced three friction points during imaging of critically damaged drives:

1. **Double-backslash paths** — Windows path completion and some terminal emulators
   insert `\\` between path components, which caused "access denied" errors when
   Ferrite forwarded the raw string to `CreateFileW` / `std::fs::File::open`.

2. **No auto-filename** — Users had to provide a full file path (`m:\disk.img`);
   providing just a directory (`m:\restore`) or leaving the field empty produced
   an error rather than a sensible default.

3. **14-minute undetected hang** — On drives with critical I/O errors (e.g. the
   USB-attached Seagate ST8000DM004 with a fatal read at offset 0x0), the first
   `ReadFile` call could hang inside the USB xHCI driver far beyond the 30-second
   software timeout.  The TUI showed 0 % progress and gave no indication that
   the engine was stuck rather than starting up.

## Solutions

### 1 · Path normalisation (`normalize_path`)

A `normalize_path(path: &str) -> String` helper in `imaging/mod.rs` collapses
consecutive backslashes to a single `\` before the path is used:

- `m:\\\\restore\\\\disk.img` → `m:\restore\disk.img`
- `\\.\PhysicalDrive9` — preserved verbatim (UNC device-path prefix)
- `\\?\Volume{...}` — preserved verbatim

Called on `dest_path` and `mapfile_path` at the start of `start_imaging()`.

### 2 · Auto-filename generation + conflict resolution

If `dest_path` is empty, ends with `\`/`/`, or resolves to an existing directory,
`start_imaging()` now auto-generates a filename:

```
<serial>_<YYYYMMDD>.img        e.g.  ST8000DM004_ABC12345_20260319.img
```

- Serial is sanitised (non-alphanumeric → `_`; empty serial → `disk_<date>`).
- A companion mapfile is auto-generated (`.map` extension) unless the user
  already set one.
- If the auto-generated filename already exists, `unique_path()` appends `_1`,
  `_2`, … before the extension until a free name is found.

The Dest placeholder in the Configuration panel now reads
`(empty — press s to auto-generate, or d to set)`.

### 3 · Watchdog display in Statistics panel

Two new fields on `ImagingState`:

| Field | Type | Purpose |
|---|---|---|
| `last_progress_instant` | `Option<Instant>` | Timestamp of most recent `Progress` message; initialised to `now()` when imaging starts |
| `watchdog_secs` | `u64` | Seconds since last progress; computed by `tick()` each frame |

`tick()` updates `watchdog_secs` from the elapsed time since `last_progress_instant`.
When a `Progress` message arrives, `last_progress_instant` is refreshed and
`watchdog_secs` is reset to 0.  Watchdog is suppressed (zero) during thermal pause
and manual pause.

When `watchdog_secs ≥ 10` the Statistics panel shows:

```
⚠ No read progress for 47s — drive may be unresponsive (try Reverse mode with r, or cancel with c)
```

### 4 · OVERLAPPED drain timeout increase

`ferrite-blockdev/src/windows.rs` — drain wait after `CancelIo` increased from
5 000 ms to **30 000 ms**.  USB host controllers (xHCI) may hold a cancelled I/O
at the hardware level for much longer than the application-level timeout; the
increased drain window reduces the probability of returning with a dangling
stack-allocated `OVERLAPPED` pointer.  A comment documents the USB driver
hazard for future maintainers.

## Files modified

| File | Change |
|------|--------|
| `crates/ferrite-blockdev/src/windows.rs` | Drain timeout 5 000 → 30 000 ms + safety comment |
| `crates/ferrite-tui/src/screens/imaging/mod.rs` | `normalize_path()`, `unique_path()`, `last_progress_instant`, `watchdog_secs`, updated `start_imaging()` |
| `crates/ferrite-tui/src/screens/imaging/render.rs` | Dest placeholder text, hint line, watchdog alert in Statistics panel |

## Tests added (10 new)

- `normalize_path_collapses_double_backslash`
- `normalize_path_single_backslash_unchanged`
- `normalize_path_preserves_unc_device_prefix`
- `normalize_path_preserves_unc_question_prefix`
- `normalize_path_empty_string`
- `unique_path_returns_original_when_not_exists`
- `watchdog_secs_zero_on_new_state`
- `watchdog_secs_zero_when_idle_after_tick`
- `watchdog_resets_to_zero_on_progress_message`

## Bug fixes (found during real-world testing)

### smartctl blocks imaging thread on dead drives

**Root cause:** `ferrite_smart::query()` calls `Command::output()` which blocks
until `smartctl` exits.  On a drive that does not respond to ATA commands
(e.g. completely failed USB drive), `smartctl` itself hangs for many minutes
waiting for a drive response.  The imaging thread was calling this on the hot
path, before `engine.run()` was ever reached — so zero `Progress` messages were
sent and the UI appeared frozen for 20+ minutes with no explanation.

**Fix:** The pre-populate SMART query is now run in a dedicated sub-thread with a
30-second `recv_timeout`.  If `smartctl` doesn't return in time, pre-population
is silently skipped and imaging proceeds immediately.

### Watchdog never rendered on first-read hangs

**Root cause:** The watchdog line was inside `if let Some(u) = &self.latest {…}`.
When the imaging thread is stuck before any `Progress` message (e.g. during the
SMART query or the very first `ReadFile`), `latest` is `None` and the `else`
branch rendered a static "press s to start" message — suppressing the watchdog
entirely.

**Fix:** The `else` branch now:
- Shows "Imaging started — waiting for first sector read…" when Running
- Renders the watchdog alert (`⚠ No read progress for Xs…`) whenever
  `watchdog_secs ≥ 10`, regardless of whether any progress data has arrived

## Test results

- All tests passing (869 unit tests, up from 860)
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — clean
