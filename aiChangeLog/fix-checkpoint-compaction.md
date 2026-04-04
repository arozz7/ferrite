# fix(carving): Checkpoint compaction — V2 compact format + session resume bug fix

## Problem

On large drives/images the JSONL checkpoint file grew to an unmanageable size
(observed: 915 MB / 2.9 M lines for an 80 GB image scan).  Two root causes:

1. **Full `Signature` struct serialized in every line** (~300 bytes/line).  There
   are only 140 unique signatures but each line embedded the entire struct
   (header bytes, footer, size_hint, etc.).
2. **Append-only writes cause 2× bloat**: every extracted hit has two entries
   (initial `Unextracted` + final status update).  The old `load()` performed
   two full passes over the file, loading all lines into a `Vec` before dedup —
   potentially 2–4 GB of RAM on resume.
3. **Session resume created new JSONL files** instead of reusing the existing
   checkpoint when `last_scanned_byte` was 0 in the saved session (e.g. scan
   completed and `scan_progress` was cleared before save).

---

## Changes

### `crates/ferrite-tui/src/screens/carving/checkpoint.rs` — full rewrite

**V2 compact wire format** (~70 bytes/line):
```json
{"v":2,"offset":351090,"sig":"Raw AAC Audio (MPEG-4 ADTS)","status":"Skipped"}
```
Stores only `byte_offset`, `sig_name`, and `status`.  The full `Signature` is
reconstructed from the live loaded config on resume.

**V1 backward compatibility**: old-format lines (`{"hit":{...full sig...},...}`)
are parsed via an `#[serde(untagged)]` enum.  The live config version of the
signature is preferred over the frozen checkpoint copy (allows size-hint
improvements to apply retroactively).

**Streaming single-pass dedup**: replaces the old two-pass/full-load approach.
Uses a `HashMap<offset → index>` to update the status of already-seen hits
in O(1), so only unique hits occupy memory.  Peak RAM drops from ~2–4 GB to
~100–200 MB for a session with 1.4 M unique hits.

**Compaction on every load**: after dedup the file is atomically rewritten
in V2 format (`write to .tmp` → `remove original` → `rename .tmp`).  This:
- converts V1 legacy files on first resume,
- collapses the 2× append-only bloat, and
- keeps file size proportional to unique hits rather than growing with sessions.

**5 new unit tests**: `roundtrip_v2`, `dedup_keeps_last_status_first_order`,
`compaction_reduces_line_count`, `unknown_sig_dropped`, `v1_legacy_upgraded_on_load`.

### `crates/ferrite-tui/src/screens/carving/session_ops.rs`
- `load_checkpoint()` builds a `HashMap<name, Signature>` from `self.groups`
  and passes it to `checkpoint::load()` for V2 sig reconstruction.

### `crates/ferrite-tui/src/screens/carving/input.rs`
- Fixed session resume creating new JSONL files: condition changed from
  `if was_resumed` to `if was_resumed || self.checkpoint_path.is_some()`.
  A checkpoint loaded from a session file is now always reused, regardless
  of whether `last_scanned_byte` was zero in the saved session.

---

## Expected impact (80 GB image, 2.9 M-line checkpoint)

| Metric | Before | After (first resume) |
|---|---|---|
| File size | 915 MB | ~40–80 MB |
| RAM on load | ~2–4 GB spike | ~100–200 MB |
| Load time | several seconds | sub-second |
| New JSONL on resume | yes (bug) | no (fixed) |
