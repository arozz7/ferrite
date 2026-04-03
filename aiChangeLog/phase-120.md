# Phase 120 — Tier 3 Enhancements (ENH-14, ENH-15)

**Date:** 2026-03-31
**Branch:** master
**Tests:** 1111 passing, 0 failing — clippy clean — fmt clean

---

## Summary

Two enhancements targeting forensic insight (language detection on recovered text
blocks) and reliability (read-verify mode for unstable drives).

---

## Changes

### ENH-14 · Text-block language detection
**Files:**
- `Cargo.toml` (workspace)
- `crates/ferrite-textcarver/Cargo.toml`
- `crates/ferrite-tui/Cargo.toml`
- `crates/ferrite-textcarver/src/scanner.rs`
- `crates/ferrite-textcarver/src/engine.rs`
- `crates/ferrite-textcarver/src/export.rs`
- `crates/ferrite-tui/src/screens/text_scan/mod.rs`
- `crates/ferrite-tui/src/screens/text_scan/input.rs`
- `crates/ferrite-tui/src/screens/text_scan/render.rs`

- Added `whatlang = "0.16"` to workspace dependencies and to
  `ferrite-textcarver` and `ferrite-tui` crate manifests.
- Added `lang: Option<whatlang::Lang>` field to `TextBlock` in `scanner.rs`.
- `engine.rs` `emit_block()`: after building the block's preview, calls
  `whatlang::detect(&preview).filter(|i| i.confidence() >= 0.5).map(|i| i.lang())`
  to set `lang`.  Confidence threshold of 0.50 suppresses ambiguous hits on
  short or mixed-language blocks.
- `TextScanState` gains `filter_lang: Option<whatlang::Lang>` (default `None`).
- `rebuild_filtered()` now checks both `filter_kind` and `filter_lang`.
- `l` key in Text Scan tab cycles through `None` → most-common-lang → … →
  None using a frequency-sorted list of distinct languages found in the current
  result set.
- Block list row now shows a 3-letter ISO 639-3 language code (e.g. `eng`,
  `deu`) between the quality column and the preview.  Undetected blocks show
  `---`.
- Title bar shows `kind: <filter>  lang: <filter>` and hints updated to
  include `l:lang`.

---

### ENH-15 · Sector read-verify mode
**Files:**
- `crates/ferrite-imaging/src/config.rs`
- `crates/ferrite-imaging/src/passes/copy.rs`
- `crates/ferrite-imaging/src/engine.rs` (test fixtures)
- `crates/ferrite-tui/src/screens/imaging/mod.rs`
- `crates/ferrite-tui/src/screens/imaging/render.rs`

- `ImagingConfig` gains two new fields:
  - `verify_reads: bool` (default `false`) — enables re-read comparison.
  - `verify_passes: u8` (default `1`) — number of additional reads performed
    per block when `verify_reads` is active.
- `passes/copy.rs`: added `verify_read()` helper.  When `verify_reads` is
  true, re-reads the block `verify_passes` times and byte-compares each result
  against the original buffer.  A mismatch or read error marks the block
  `NonTrimmed` (triggers trim-pass re-processing) instead of writing
  potentially corrupt data.  Logic applied to both forward and reverse copy
  paths.  Second `AlignedBuffer` allocated once up-front and reused; zero cost
  when disabled.
- `ImagingState` gains `pub verify_reads: bool` (default `false`).
- `V` key toggles `verify_reads` in the imaging config panel.
- Render: new `Verify` row (between Sparse and Space) shows `ON`/`OFF` in
  yellow/dark-grey with `(V to toggle)` hint.
- Config panel height constraint bumped from 14 → 15 to accommodate the new row.
- 7 test `ImagingConfig` literals in `engine.rs` updated to
  `..ImagingConfig::default()` to avoid breaking on future field additions.

---

## Enhancement Backlog Updates

Items marked `done` in `docs/enhancement-backlog.md`:
- ENH-14 ✅
- ENH-15 ✅

Remaining open items: ENH-09 (Tier 3), ENH-16 through ENH-18 (Tier 4).
