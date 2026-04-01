# Phase 118 — Tier 2 Enhancements (ENH-07, ENH-10, ENH-13)

**Date:** 2026-03-30
**Branch:** master
**Tests:** 1111 passing, 0 failing — clippy clean — fmt clean

---

## Summary

Three targeted enhancements from the post-Phase-112 forensic engineering audit
(`docs/enhancement-backlog.md`): encrypted volume detection and surfacing,
SHA-256 sidecar generation on every carved file extraction, and a pre-flight
zero-size device guard in the carving engine.

---

## Changes

### ENH-07 · Encrypted volume detection
**Files:**
- `crates/ferrite-filesystem/src/lib.rs`
- `crates/ferrite-filesystem/src/detect.rs`
- `crates/ferrite-tui/src/screens/file_browser.rs`

- Added `FilesystemType::Encrypted` variant with doc comment.
- Added `Display` arm: `FilesystemType::Encrypted => "BitLocker (encrypted)"`.
- Updated `open_filesystem()` match: `HfsPlus | Encrypted | Unknown => UnknownFilesystem`.
- `detect_at()` in `detect.rs`: added BitLocker OEM ID check (`-FVE-FS-` at bytes
  `[3..11]`) **before** the NTFS check — BitLocker replaces the NTFS OEM ID with the
  same field, so order matters.
- `file_browser.rs` `start_open()`: added `FilesystemType::Encrypted` short-circuit
  (same pattern as Phase 117 HFS+ fix) with actionable message:
  `"BitLocker encrypted volume detected. Decrypt the volume first (e.g. manage-bde
  or Disk Management) then re-open. File carving (Tab 5) may recover unencrypted
  fragments from slack space."`
- **1 new test:** `detect_bitlocker_volume` — verifies `detect_filesystem()` returns
  `FilesystemType::Encrypted` for a buffer with `-FVE-FS-` OEM ID at bytes `[3..11]`.

---

### ENH-10 · Post-extraction SHA-256 sidecar
**File:** `crates/ferrite-tui/src/screens/carving/extract.rs`

- Added `use sha2::{Digest, Sha256}` import (sha2 already in ferrite-tui Cargo.toml).
- Added `pub(super) fn write_sha256_sidecar(path: &str)` helper:
  - Reads the extracted file, hashes with SHA-256.
  - Writes `<path>.sha256` in GNU `sha256sum`-compatible format:
    `<hex>  <filename>\n`
  - Errors are silently ignored — sidecar generation is best-effort.
- Called in **single extraction path** (`extract_selected`) after `try_recovered_rename`
  resolves the final filename and before `CarveMsg::Extracted` is sent.
- Called in **bulk extraction path** (worker thread in `extract_all_selected`) after the
  `try_recovered_rename` step and before `WorkerMsg::Completed` is sent — guarded by
  `if result.is_ok()` so sidecars are only written for successful extractions.
- No changes to `HitStatus`, `CarveMsg`, or checkpoint JSONL format — sidecar
  writing is a pure side-effect with no message threading required.

---

### ENH-13 · Zero-size device guard in carving
**Files:**
- `crates/ferrite-carver/src/scanner.rs`
- `crates/ferrite-tui/src/screens/carving/input.rs`

- `scanner.rs` `scan_impl()`: separated the `device_size == 0` branch from
  `signatures.is_empty()` and added `tracing::warn!` when device_size is zero,
  so the condition is always logged when the carver is invoked directly.
- `input.rs` `start_scan()`: added pre-flight check immediately after
  `device.size()` is read. If `device_size == 0`, sets `CarveStatus::Error` with
  the message `"Device reports size 0 — select a valid drive or image file before
  scanning."` and returns without spawning any threads. This surfaces the condition
  in the TUI error slot rather than silently doing nothing.

---

## Enhancement Backlog Updates

Items marked `done` in `docs/enhancement-backlog.md`:
- ENH-07 ✅
- ENH-10 ✅
- ENH-13 ✅

Remaining open items: ENH-08, ENH-09, ENH-11, ENH-12, ENH-14 through ENH-18.
