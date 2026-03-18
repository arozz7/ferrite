# Phase 42 — Session Resume: Fix Progress Display and Rate Calculation

## Problem

When a saved session was restored and the scan resumed:

1. **Progress bar jumped to 0%** — even though the scan correctly continued from
   the saved byte offset, the bar showed "0% of remaining window" because
   `scan_start` was set to `resume_from_byte`.  Users interpreted this as the
   scan restarting from scratch.

2. **Scan rate was inflated** — the MB/s calculation divided `p.bytes_scanned`
   (absolute device offset, e.g. 500 GB) by `active_secs` (a few seconds),
   producing impossibly high rates like "50 000 MB/s".

## Root Cause

`ScanProgress::scan_start` equals `config.start_byte = resume_from_byte`, so:

```
progress % = (bytes_scanned − scan_start) / (scan_end − scan_start) = 0%
rate       = bytes_scanned / active_secs   ← absolute offset, not session bytes
```

## Solution

### New field: `scan_window_start`

Added `scan_window_start: u64` to `CarvingState`.  Set in `start_scan` to
`window_start` (the configured LBA × sector_size), which is the **original**
window start before any resume adjustment.

### Progress formula change

Both compact bar and full gauge now compute:

```
covered = bytes_scanned − scan_window_start   (includes already-covered bytes)
window  = scan_end − scan_window_start
frac    = covered / window
```

On resume from 50%, the bar starts at ~50% and continues to 100%.

### Rate formula change

Both compact and full stats lines now compute:

```
bytes_this_session = bytes_scanned − scan_start   (only what this session read)
rate_bps           = bytes_this_session / active_secs
```

This gives an accurate MB/s immediately after resume.

## Files Changed

- `crates/ferrite-tui/src/screens/carving/mod.rs` — added `scan_window_start: u64` field + initialiser
- `crates/ferrite-tui/src/screens/carving/input.rs` — set `self.scan_window_start = window_start` in `start_scan`
- `crates/ferrite-tui/src/screens/carving/render_progress.rs` — progress fraction uses `scan_window_start`; rate uses `bytes_this_session = bytes_scanned − scan_start`; fixed stale `scanned_in_window` variable reference
- `aiChangeLog/phase-42.md` — this file

## Test Results

- All 136 ferrite-carver tests pass
- All 63 ferrite-tui tests pass
- All workspace tests pass, clippy clean
