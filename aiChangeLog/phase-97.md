# Phase 97 — Health Screen UX: Verdict Context + USB Fix

## Problem
The Health tab showed `✗ CRITICAL` with no enough context for the user to
understand *why* or whether the drive was genuinely in danger.  Additional
problems: USB/removable drives almost always returned WARNING or CRITICAL due
to the spin-up time attribute being checked on non-spinning drives.

## Changes

### `crates/ferrite-tui/src/screens/health.rs`

**Summary panel — expanded and enriched:**
- Dynamic height: `base_lines + reason_count` so reasons never clip
- Added **Firmware** and **Size | Rotation** (SSD / N RPM / —) lines
- Added **bad-sector LBA count** inline when non-zero ("X bad-sector LBAs in error log")
- Reason bullets are now colour-coded: red for CRITICAL reasons, yellow for WARNING
- USB bridge note when `rotation_rate == None`: grey advisory line

**Attribute table — visual severity colouring:**
- New **`Pf` column** — `!` (yellow) for prefailure attributes, `·` (grey) for informational
- Informational (`prefailure == false`) rows dimmed to dark grey unless flagged
- Per-row foreground colour driven by `attr_row_color()`:
  - **Red** when `value ≤ thresh` (drive's own manufacturer failure threshold)
  - **Red** when raw value ≥ our critical threshold (IDs 5 / 197 / 198 / 3)
  - **Yellow** when raw value ≥ our warning threshold
- New **Status column** replaces the old `Failed?` column:
  - `val≤thr!` (red bold) — drive's own threshold crossed
  - `past` / `now` — smartctl's `when_failed` string
  - `critical` / `warning` — our raw-value threshold verdict
  - blank — healthy

**Table title** now includes a brief column legend:
`Pf=prefailure  Val/Wst/Thr=normalised(higher=better)`

### `crates/ferrite-smart/src/verdict.rs`
- **Spin_Up_Time (ID 3) skipped for non-spinning drives** — only checked when
  `rotation_rate > 0` (confirmed spinning HDD).  USB flash drives, external
  SSDs, and bridge-connected drives with `rotation_rate == None` or `Some(0)`
  no longer receive spurious CRITICAL/WARNING verdicts from this attribute.

### `crates/ferrite-smart/src/lib.rs`
- Re-exported `CountThresholds` so TUI can use it directly.
