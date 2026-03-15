# Phase 27 — Carving UX: LBA Window, Hit Persistence, Session Manager

## Summary

Extended the carving screen with scan-range windowing, JSONL checkpoint persistence,
per-drive session files, a session-manager overlay, and improved hits-list navigation.

---

## Task 6 — PageUp/PageDown/Home/End for hits list

**File:** `crates/ferrite-tui/src/screens/carving/input.rs`

- Added `PageUp`, `PageDown`, `Home`, `End` key handlers (hits panel only).
- Each calls `refresh_preview()` when preview panel is open.
- `hits_page_size` field on `CarvingState` tracks visible rows (set each render).

**File:** `crates/ferrite-tui/src/screens/carving/render.rs`

- `render_hits_panel` changed to `&mut self` so it can update `hits_page_size`.
- Title bar now includes `PgUp/Dn: page  Home/End: jump` hint.

---

## Task 1 — Scan range (LBA window)

**File:** `crates/ferrite-carver/src/signature.rs`

- Added `start_byte: u64` and `end_byte: Option<u64>` to `CarvingConfig`.
- `Default` uses `scan_chunk_size: 4 * 1024 * 1024`.
- Added `serde::{Serialize, Deserialize}` to `Signature` and `SizeHint`.

**File:** `crates/ferrite-carver/src/scanner.rs`

- `ScanProgress` gains `scan_start` and `scan_end` fields.
- `scan_inner` respects `config.start_byte` / `config.end_byte` window.
- `CarveHit` derives `serde::{Serialize, Deserialize}`.

**File:** `crates/ferrite-tui/src/screens/carving/mod.rs`

- Added `ScanRangeField` enum and fields `scan_start_lba_str`, `scan_end_lba_str`, `scan_range_field`.
- `is_editing()` covers scan range field editing.

**File:** `crates/ferrite-tui/src/screens/carving/input.rs`

- `[` / `]` keys (when not running) activate start/end LBA fields.
- Digit/Backspace routed to active field; Esc/Enter confirms.
- `start_scan()` parses LBA strings → `start_byte` / `end_byte` in `CarvingConfig`.

**File:** `crates/ferrite-tui/src/screens/carving/render.rs`

- New `render_scan_range_bar()` shows From/To LBA fields; active field highlighted yellow.
- Layout updated to 3 rows (output dir + scan range + panels).
- Progress gauge and ETA calculation use windowed range (`scan_start`..`scan_end`).

---

## Task 2 — Hit persistence (JSONL checkpoint)

**File:** `crates/ferrite-tui/src/screens/carving/checkpoint.rs` (NEW)

- `CheckpointEntry { hit: CarveHit, status: HitStatus }` — serde round-trippable.
- `append(path, hit, status)` — appends one JSONL line (creates dir/file as needed).
- `load(path)` — reads all valid lines from a checkpoint file.

**File:** `crates/ferrite-tui/src/screens/carving/mod.rs`

- Added `checkpoint_path: Option<String>` and `checkpoint_flushed: usize`.
- `HitStatus` derives `serde::{Serialize, Deserialize}`.

**File:** `crates/ferrite-tui/src/screens/carving/events.rs`

- Flushes new hits to checkpoint every 1000 hits during `Progress` messages.
- Flushes all remaining hits when `Done` arrives.

**File:** `crates/ferrite-tui/src/screens/carving/input.rs`

- `start_scan()` sets `checkpoint_path` derived from `output_dir`.

---

## Task 3 — Drive-tagged session files

**File:** `crates/ferrite-tui/src/carving_session.rs` (NEW)

- `CarvingSession` struct: `drive_serial`, `drive_model`, `drive_size`, `scan_start_lba`,
  `scan_end_lba`, `last_scanned_byte`, `output_dir`, `hits_file`, `hits_count`, `saved_at`.
- `save()` — writes JSON to `sessions/<serial>-<YYYY-MM-DD>.json`.
- `load_all()` — reads all session files from `sessions/`.
- `delete()` — removes the session JSON file.
- `matches_drive(info)` — checks serial equality.
- `age_str()` — human-readable relative age.

**File:** `crates/ferrite-tui/src/lib.rs`

- Added `pub mod carving_session;`.

**File:** `crates/ferrite-tui/src/screens/carving/session_ops.rs` (NEW)

- `load_checkpoint(path)` — restores hits from JSONL into `self.hits`.
- `build_session(info)` — constructs a `CarvingSession` from current state.
- `restore_from_session(session)` — wires `output_dir`, LBA fields, calls `load_checkpoint`.

**File:** `crates/ferrite-tui/src/session.rs`

- Added `carving_scan_start_lba: String` and `carving_scan_end_lba: String` (serde default).

---

## Task 5 — Session manager overlay

**File:** `crates/ferrite-tui/src/screens/session_manager.rs` (NEW)

- `SessionManagerState` — `visible`, `sessions`, `selected`, `connected`, `verify` fields.
- `SessionMsg` — `Resume { session, device }` or `Dismissed`.
- `VerifyState` — `Unknown | Matched(String) | NotFound`.
- `open()` loads all sessions; `handle_key()` handles ↑↓/Enter/d/r/Esc/q.
- `render()` draws a centered popup (80% × 60%) listing sessions with age, hits, drive tag.
- `d` key deletes selected session; `r` refreshes drive list; Enter emits `Resume`.

**File:** `crates/ferrite-tui/src/screens/mod.rs`

- Added `pub mod session_manager;`.

**File:** `crates/ferrite-tui/src/app.rs`

- Added `session_manager: SessionManagerState` to `App`.
- Key events intercepted at top of `handle_key` when session manager is visible.
- `o` on drive-select screen opens session manager.
- Session manager rendered as overlay after main screen render.
- On quit (if hits or checkpoint set) saves a `CarvingSession`.
- Restores `carving_scan_start_lba`/`carving_scan_end_lba` from saved `Session`.

---

## Task 4 — Drive select hint

**File:** `crates/ferrite-tui/src/screens/drive_select.rs`

- Block title updated to include `o: sessions` hint.
- `platform_enumerate`, `platform_get_info`, `platform_open` changed to `pub(crate)`.

---

## Module Refactoring

`crates/ferrite-tui/src/screens/carving/mod.rs` was reduced from ~1 098 lines to 454 lines
by extracting:

| New file      | Extracted content                                      |
|---------------|--------------------------------------------------------|
| `helpers.rs`  | `fmt_bytes`, `hash_hit_prefix`, `dedup_hits`, `load_builtin_signatures` |
| `input.rs`    | `handle_key`, `start_scan`, `toggle_pause`, `cancel_scan`, navigation helpers |
| `events.rs`   | `tick` — channel drain + checkpoint flush              |
| `session_ops.rs` | `load_checkpoint`, `build_session`, `restore_from_session` |

`render.rs` reduced from 603 → 585 lines within the 600-line hard limit.

---

## Tests & Quality

- `cargo test --workspace` — all 222+ tests pass (0 failures).
- `cargo clippy --workspace -- -D warnings` — clean.
- `cargo fmt --check` — not yet run (no formatting changes made).

---

## Files Changed

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/signature.rs` | Added `start_byte`/`end_byte` to `CarvingConfig`; serde derives |
| `crates/ferrite-carver/src/scanner.rs` | `ScanProgress` windowing fields; `CarveHit` serde |
| `crates/ferrite-carver/Cargo.toml` | Added `serde_json` |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | Reduced to 454 lines; new fields |
| `crates/ferrite-tui/src/screens/carving/checkpoint.rs` | NEW |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | NEW |
| `crates/ferrite-tui/src/screens/carving/input.rs` | NEW |
| `crates/ferrite-tui/src/screens/carving/events.rs` | NEW |
| `crates/ferrite-tui/src/screens/carving/session_ops.rs` | NEW |
| `crates/ferrite-tui/src/screens/carving/render.rs` | Scan range bar; windowed progress |
| `crates/ferrite-tui/src/screens/carving/extract.rs` | Updated `CarvingConfig` constructors |
| `crates/ferrite-tui/src/carving_session.rs` | NEW |
| `crates/ferrite-tui/src/screens/session_manager.rs` | NEW |
| `crates/ferrite-tui/src/screens/mod.rs` | Added `session_manager` module |
| `crates/ferrite-tui/src/lib.rs` | Added `carving_session` module |
| `crates/ferrite-tui/src/session.rs` | Added LBA fields |
| `crates/ferrite-tui/src/app.rs` | Session manager wiring; save-on-quit |
| `crates/ferrite-tui/src/screens/drive_select.rs` | `o: sessions` hint; pub(crate) fns |
