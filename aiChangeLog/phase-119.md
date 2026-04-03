# Phase 119 — Tier 2/3 Enhancements (ENH-08, ENH-11, ENH-12)

**Date:** 2026-03-31
**Branch:** master
**Tests:** 1111 passing, 0 failing — clippy clean — fmt clean

---

## Summary

Three enhancements targeting forensic depth (ADS enumeration), false-positive
reduction (artifact confidence scoring), and UX clarity (partition disk-map).

---

## Changes

### ENH-08 · NTFS Alternate Data Stream enumeration
**Files:**
- `crates/ferrite-filesystem/src/ntfs_helpers.rs`
- `crates/ferrite-filesystem/src/ntfs.rs`

- Added `pub(crate) fn parse_ads_streams(raw: &[u8]) -> Vec<(String, u64, Option<u64>)>`
  to `ntfs_helpers.rs`.  Walks all MFT attributes, collects every named `$DATA`
  (type `0x80`) attribute — i.e. streams with `name_len > 0`.  Returns
  `(stream_name, data_size, first_lcn)` for each.  Both resident and non-resident
  named streams are handled.
- `ntfs.rs` `enumerate_files()`: after creating the primary `FileEntry` for each
  non-directory MFT record, calls `parse_ads_streams()` and appends one additional
  `FileEntry` per named stream.  Entry `name` uses the `filename:streamname` format
  (standard Windows ADS notation); `path` is resolved in the same parent-chain walk
  as regular files.
- No changes to `FileEntry` struct — ADS entries are indistinguishable in storage
  but the colon in the name makes them trivially identifiable to operators.
- Forensic value: `Zone.Identifier` (Internet download origin), browser/email
  attachment metadata, and malware payload streams now appear in the Files tab
  and are available for extraction via the existing path.

---

### ENH-11 · Forensic artifact confidence scoring
**Files:**
- `crates/ferrite-artifact/src/scanner.rs`
- `crates/ferrite-artifact/src/scanners/email.rs`
- `crates/ferrite-artifact/src/scanners/url.rs`
- `crates/ferrite-artifact/src/scanners/iban.rs`
- `crates/ferrite-artifact/src/scanners/ssn.rs`
- `crates/ferrite-artifact/src/scanners/win_path.rs`
- `crates/ferrite-artifact/src/scanners/credit_card.rs`
- `crates/ferrite-artifact/src/export.rs`
- `crates/ferrite-artifact/src/lib.rs`
- `crates/ferrite-tui/src/screens/artifacts/render.rs`
- `crates/ferrite-tui/src/screens/artifacts/mod.rs`

- Added `Confidence { Low, Medium, High }` enum to `scanner.rs`.  Re-exported
  from `ferrite_artifact::Confidence`.
- Added `confidence: Confidence` field to `ArtifactHit`.
- `scan_text_lossy` transform closure signature changed from `Option<String>` to
  `Option<(String, Confidence)>` so each scanner controls confidence at the
  match level.
- **Email scanner:** added `PLACEHOLDER_DOMAINS` constant; matches on
  `example.com`, `example.org`, `example.net`, `test.com`, `test.org`,
  `test.net`, `localhost`, `invalid`, `foo.com`, `bar.com`, `baz.com` →
  `Confidence::Low`.  All other domains → `Confidence::High`.
- **Credit card scanner:** added `context_is_printable(data, start, end) -> bool`
  helper that examines 32 bytes before and after the match; if < 50 % of
  surrounding bytes are printable ASCII (0x20–0x7E) the match is embedded in
  binary and gets `Confidence::Low`.  Text context → `Confidence::High`.
- **URL, IBAN, SSN, Windows path:** all return `Confidence::High` (already have
  strong structural validation).
- `export.rs` CSV format updated: `byte_offset,kind,confidence,value` (header and
  rows); `make_hit` test helper updated.
- TUI hit list: confidence badge appended to each row — `[Med]` in amber,
  `[Low]` in dark grey; `High` is silent to avoid clutter on unambiguous hits.

---

### ENH-12 · Partition disk-map visualisation
**File:** `crates/ferrite-tui/src/screens/partition.rs`

- Added `disk_size_bytes: u64` field to `PartitionState` (captured from
  `device.size()` in `set_device()`).
- Added `RenderContext` struct to consolidate `selected`, `export_status`,
  `show_contention_warning`, and `disk_size_bytes` — keeps `render_partition_table`
  within the 7-argument clippy limit.
- Added `render_disk_map(frame, area, tbl, disk_size_bytes)` function:
  - Width = `area.width` columns.
  - Each partition occupies a proportional run of `█` (U+2588 FULL BLOCK) cells
    using 128-bit integer arithmetic to avoid overflow on large disks.
  - Unpartitioned gaps rendered as `░` (U+2591 LIGHT SHADE) in dark grey.
  - Overlapping LBA ranges highlighted in red — visually flags partition-table
    corruption or recovered-scan artefacts.
  - Seven-colour cycling palette (cyan → green → blue → magenta → yellow →
    light cyan → light green) to distinguish adjacent partitions.
- One-row disk-map bar is inserted between the `table.note` row and the partition
  table in the vertical layout.  Hidden when the table is empty or disk size is
  unknown (prevents layout shifts when no data is loaded).

---

## Enhancement Backlog Updates

Items marked `done` in `docs/enhancement-backlog.md`:
- ENH-08 ✅
- ENH-11 ✅
- ENH-12 ✅

Remaining open items: ENH-09, ENH-14, ENH-15 (Tier 3), ENH-16 through ENH-18 (Tier 4).
