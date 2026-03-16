# Phase 33 — Session Resume Correctness & MP4 Size Fix

## Summary

Two independent bug-fix tracks addressed in this phase:

1. **Session resume didn't resume** — loading a saved session restarted the scan from the beginning instead of from `last_scanned_byte`; `auto_extract` was also lost on resume.
2. **MP4 always extracted as 4 GiB** — no size hint existed for the ISOBMFF container format; `max_size` was always used. A secondary bug caused the `[TRUNC]` label to never appear for footer-less formats.

---

## Track A: Session Resume Bug Fixes

### Root Causes

| # | Symptom | Cause |
|---|---------|-------|
| 1 | Scan restarts from beginning on resume | `restore_from_session()` restored `scan_start_lba_str` to the *original window start*, not `last_scanned_byte`. The scan start byte was computed only from that LBA string. |
| 2 | `auto_extract` resets to `false` on resume | `auto_extract` was not included in `CarvingSession` — pure runtime state. |
| 3 | `set_device()` wiped restored values | In `app.rs`, `restore_from_session()` ran before `set_device()`, so `set_device()` reset `auto_extract` and would have reset `resume_from_byte`. |

### Changes

**`crates/ferrite-tui/src/carving_session.rs`**
- Added `auto_extract: bool` field (`#[serde(default)]` for backward compat with old session files).

**`crates/ferrite-tui/src/screens/carving/mod.rs`**
- Added `resume_from_byte: u64` field to `CarvingState`; initialized to `0`.

**`crates/ferrite-tui/src/screens/carving/session_ops.rs`**
- `restore_from_session()`: now restores `resume_from_byte = session.last_scanned_byte` and `auto_extract`.
- `build_session()`: now saves `auto_extract`.

**`crates/ferrite-tui/src/screens/carving/input.rs`**
- Scan start: if `resume_from_byte > window_start`, it becomes `start_byte` (resume skips already-scanned sectors). `resume_from_byte` is cleared to `0` immediately after so subsequent manual scans start fresh.

**`crates/ferrite-tui/src/app.rs`**
- Swapped `set_device` / `restore_from_session` call order in the `SessionMsg::Resume` handler: `set_device` now runs first (resets everything to defaults), then `restore_from_session` overwrites with persisted values.

---

## Track B: MP4 / ISOBMFF Extraction

### Root Causes

| # | Symptom | Cause |
|---|---------|-------|
| 1 | MP4 always extracted as 4.0 GiB | No size hint — `extraction_size` fell through to `max_size = 4 GiB`. |
| 2 | Footer-less files never showed `[TRUNC]` | `truncated = false` when `footer.is_empty()`, so the cap-hit was silently labelled `[OK]`. |
| 3 | `carver_io.rs` at 600-line hard limit | No room to add the new ISOBMFF walker. |

### Changes

**`crates/ferrite-carver/src/size_hint.rs`** *(new file)*
- Extracted all `read_size_hint` logic from `carver_io.rs` into a dedicated module.
- Added `SizeHint::Isobmff` arm: walks sequential top-level ISO BMFF boxes by reading each 4-byte size (BE u32) + 4-byte printable-ASCII type, summing until sync is lost or a 2 000-box safety cap is reached. Handles largesize (u64) boxes correctly. Returns `None` on failure (caller falls back to `max_size`).
- 4 unit tests: two-box file, invalid type stops walk, size-zero stops walk, no-valid-boxes returns None.

**`crates/ferrite-carver/src/carver_io.rs`**
- Removed `read_size_hint` and all its arms (moved to `size_hint.rs`). File reduced from 600 → ~220 lines.

**`crates/ferrite-carver/src/lib.rs`**
- Registered `mod size_hint`.

**`crates/ferrite-carver/src/scanner.rs`**
- Updated import: `read_size_hint` now from `crate::size_hint` instead of `crate::carver_io`.

**`crates/ferrite-carver/src/signature.rs`**
- Added `SizeHint::Isobmff` enum variant with full doc comment.
- `kind_name()` returns `"mp4"` for the new variant.
- TOML parser: `size_hint_kind = "mp4"` or `"isobmff"` (case-insensitive) maps to `SizeHint::Isobmff`.

**`config/signatures.toml`**
- MP4 signature: added `size_hint_kind = "mp4"`. `max_size = 4 GiB` remains as fallback for corrupt/fragmented files where box-walking fails.

**`crates/ferrite-tui/src/screens/carving/extract.rs`**
- Fixed truncated detection in both single-file and bulk-extraction paths: `truncated = bytes >= hit.signature.max_size` unconditionally (was `false` when `footer.is_empty()`). Footer-less files that hit the cap now correctly show `[TRUNC]` instead of `[OK]`.

---

## Test Results

```
cargo test --workspace   → 250 tests, 0 failures
cargo build --workspace  → clean
```
