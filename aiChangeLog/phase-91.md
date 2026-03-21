# Phase 91 — Session: per-session checkpoint + extraction status persistence + signature selection

## Problems Fixed

### 1. Shared checkpoint file polluted across sessions
All sessions using the same output directory appended to the same
`ferrite-hits.jsonl` file.  On resume, ALL historical hits from every prior
session were loaded — producing a mix of stale entries from different drives
and masking the current session's state.

### 2. Extraction status lost on restart
The checkpoint only recorded hits at scan time with status `Unextracted`.
Post-extraction outcomes (Ok, Truncated, Duplicate, Skipped) lived only in
memory.  On resume, every file appeared as unextracted regardless of what
was actually done in the previous run.

### 3. Signature selection not saved
Enabling or disabling file-type signatures (e.g. disabling all Video sigs)
was reset to defaults on every application restart.  The user had to
re-configure the signature panel every time they resumed a session.

## Solutions

### Per-session checkpoint filename (`input.rs`)
New scans generate a unique checkpoint path: `<output_dir>\ferrite-hits-<unix_ts>.jsonl`.
Each session gets its own file; the old shared `ferrite-hits.jsonl` is no
longer created or polluted by new sessions.  Resumed sessions continue
reading and writing their original checkpoint file (path stored in session JSON).

### Extraction status written back to checkpoint (`checkpoint.rs`, `events.rs`, `mod.rs`)
- New `checkpoint::append_batch(path, &[(hit, status)])` — writes a batch
  of status-update entries in a single `write()` call.
- New `checkpoint_extract_pending: Vec<usize>` field on `CarvingState` — collects
  hit indices as `Extracted`/`Duplicate`/`Skipped`/`SkippedCorrupt` messages arrive.
- At `ExtractionDone`, the pending indices are flushed to the checkpoint file via
  `append_batch`, then cleared.
- `checkpoint::load` now deduplicates by `byte_offset`, keeping the **last**
  occurrence.  This means post-extraction status updates win over the initial
  `Unextracted` record, so resume correctly reflects what was already extracted.

### Signature selection persisted (`carving_session.rs`, `session_ops.rs`)
- New `disabled_sigs: Vec<String>` field on `CarvingSession` (serde default = empty,
  so old sessions load cleanly with all sigs enabled).
- `build_session`: collects names of all disabled signatures.  Only disabled names
  are stored — new signatures added after a session was saved remain enabled by default.
- `restore_from_session`: after loading groups, sets `enabled = false` for any
  signature whose name appears in `disabled_sigs`.

### `SessionMsg::Resume` boxed (`session_manager.rs`)
`CarvingSession` grew with `disabled_sigs: Vec<String>`, pushing the
`SessionMsg::Resume` variant past Clippy's large-variant threshold.
`session: CarvingSession` → `session: Box<CarvingSession>`.

## Files Changed

- `crates/ferrite-tui/src/carving_session.rs` — `disabled_sigs` field
- `crates/ferrite-tui/src/screens/carving/checkpoint.rs` — `append_batch`; dedup-on-load
- `crates/ferrite-tui/src/screens/carving/events.rs` — collect pending indices; flush at `ExtractionDone`
- `crates/ferrite-tui/src/screens/carving/mod.rs` — `checkpoint_extract_pending` field + init + clear
- `crates/ferrite-tui/src/screens/carving/session_ops.rs` — `build_session` saves `disabled_sigs`; `restore_from_session` applies them; `load_checkpoint` doc updated
- `crates/ferrite-tui/src/screens/carving/input.rs` — unique timestamp-based checkpoint filename
- `crates/ferrite-tui/src/screens/session_manager.rs` — `Box<CarvingSession>` in `SessionMsg::Resume`

## Test Results
- 880 tests — all passing
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --all -- --check` — clean
