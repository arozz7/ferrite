# Phase 47c — Low Read-Rate Alert + Amber TUI Indicator

## Summary
Adds a visual amber alert to the imaging screen when the rolling read rate drops
below 5 MB/s (sustained).  The threshold matches the existing `⚠ SLOW` text
label; this phase promotes that text to a proper amber colour and propagates the
state to the progress bar.

## Changes

### ferrite-tui — `src/screens/imaging/render.rs`

**`is_low_rate` computation**
- Derived from `latest.read_rate_bps > 0 && read_rate_bps < 5 MiB/s`.
- Computed once per render frame from the latest progress snapshot; no new
  persistent state required.

**Progress bar (`bar_style` + `bar_title`)**
- `bar_style`: `ImagingStatus::Running if is_low_rate` arm added — bar turns
  amber (`Color::Yellow`) when rate is slow but not yet paused (paused states
  take priority since they're more severe).
- `bar_title`: `" Progress [⚠ LOW RATE] "` shown when `is_low_rate` is true and
  the engine is not already thermally- or user-paused.

**Rate line (Statistics panel)**
- Extracted from the plain `stats` format string into a dedicated styled `Line`
  composed via `text.push_line`.
- Three cases:
  - `read_rate_bps == 0` → `" Rate: — ETA …"` (plain, no rate yet)
  - `is_low_rate` → `" Rate: X.X MB/s ⚠ SLOW"` span in **amber**, ETA/Temp appended in default style
  - normal → `" Rate: X.X MB/s  ETA …  Temp …"` plain white
- `stats` format string trimmed to the first line only
  (`Finished / Bad / Non-tried / Elapsed`); rate+ETA+temp now live on their own
  styled line below.
