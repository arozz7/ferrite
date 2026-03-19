# Phase 85 — PNG Chunk-Header File Walk + Auto-Follow UX

## Goal
Fix PNG corruption detection for fragmented drives where the garbage chunk
sits in the "dead zone" between the fixed 8 KiB head buffer and 64 KiB tail
buffer, and introduce a live auto-follow mode for the carving hit list.

---

## PNG Validation — Architectural Fix

### Problem
Phase 84 introduced a post-IDAT chunk-type check using the tail buffer, which
caught corruption where the IDAT end fell within the last 64 KiB of the file.
Two real-world PNG files from the 4 TB test run revealed a second pattern:

| File | Size | IDAT end offset | Tail start | Result (before) |
|------|------|-----------------|------------|-----------------|
| `ferrite_png_2564461650457.png` | 470 KB | 470,777 | 405,393 | **in tail → Phase 84 caught it** |
| `ferrite_png_2564460998851.png` | 1.1 MB | 651,572 | 1,056,999 | **dead zone → Phase 84 missed it** |

Both files have the same corruption pattern:
- Header ancillary chunks (IHDR / pHYs / iCCP / cHRM) are intact with valid CRCs
- Single IDAT body was overwritten by a different sector — CRC fails
- The chunk immediately following IDAT has a non-ASCII type (e.g. `00 14 00 00`
  or `21 60 1E EE`) — a clear sign of fragmentation
- IEND is present at the very end

For file 2, the garbage chunk is 405 KB into the file — out of reach of both
the 8 KB head and the 64 KB tail.

### Solution — `validate_png_file(path: &Path) -> CarveQuality`

New function in `ferrite-carver/src/post_validate/mod.rs`.

Instead of sampling the file at fixed offsets, it opens the carved file and
walks the chunk structure sequentially using `Seek`:

```
open file
verify 8-byte PNG signature
loop:
  read 8 bytes → data_len (4 B) + chunk_type (4 B)
  if chunk_type is not ASCII alphabetic → Corrupt
  if chunk_type == IEND → Complete (or Corrupt if data_len ≠ 0)
  if data_len ≤ 64 KiB:
    read body, verify CRC-32 (catches corrupt small chunks like IHDR)
  else:
    seek forward data_len + 4 (skip IDAT body + CRC, no read)
  repeat
```

**I/O cost:** ~8 bytes (signature) + N × (8 bytes read + 1 seek) per chunk.
For a typical PNG with 6 chunks: ~56 bytes read + 6 seeks.  For comparison,
the previous approach read 8 KiB + 64 KiB = 72 KiB unconditionally.

**Dead zones:** none.  A garbage chunk type anywhere in the file is caught on
the first read after seeking past the preceding chunk body.

### Call-site changes — `ferrite-tui/src/screens/carving/extract.rs`

Both extraction paths (single `e`-key extract and bulk auto-extract worker)
now branch on extension:

```rust
let quality = if hit.signature.extension == "png" && !truncated {
    post_validate::validate_png_file(Path::new(&filename))
} else {
    // existing head + tail path for all other formats
};
```

Truncated PNGs skip the file walk (the file is incomplete by definition) and
return `CarveQuality::Truncated` through the existing `validate_extracted` path.

### Test coverage

Four new file-based tests in `post_validate/tests.rs` (use `tempfile` crate):

| Test | Scenario |
|------|----------|
| `validate_png_file_complete_minimal` | Minimal valid IHDR + IDAT + IEND |
| `validate_png_file_corrupt_garbage_type_after_large_idat` | Large IDAT (100 KB, > MAX_CRC_BODY) followed by non-ASCII garbage chunk type — the key real-world case |
| `validate_png_file_corrupt_bad_crc_on_small_chunk` | IHDR with deliberately wrong CRC |
| `validate_png_file_corrupt_missing_iend` | File truncated inside IDAT body |

`tempfile` added as a dev-dependency to `ferrite-carver/Cargo.toml`.

---

## Carving Hit List — Auto-Follow UX

### Problem
The hit list renders newest-first (Phase 84).  With `hit_sel = 0` (internal
index of the oldest hit), `visual_sel = hits.len() - 1`, which ratatui scrolls
to show.  As new hits arrive `hits.len()` grows and the selected row drifts
downward — the user is scrolled away from the newest activity without any way
to re-engage live tracking.

### Solution — `auto_follow: bool` field

Standard "sticky scroll" / `tail -f` pattern:

| Event | `auto_follow` | Effect |
|-------|---------------|--------|
| Scan starts | `true` | Live tracking enabled |
| `HitBatch` arrives | — | If `true`: `hit_sel = hits.len() - 1` (visual top = newest) |
| ↑ / ↓ / PgUp / PgDn / End | `false` | User took manual control |
| `Home` | `true` | Re-engage live tracking, jump to newest |

A `[↓ LIVE]` badge appears in the hits panel title while active and the scan
is running, so the user always knows the current mode.  The badge disappears
when paused, done, or the user navigates away.

### Files changed

| File | Change |
|------|--------|
| `mod.rs` | Added `auto_follow: bool` field, initialised `false` |
| `input.rs` | `start_scan` sets `true`; ↑ ↓ PgUp PgDn End set `false`; Home sets `true` |
| `events.rs` | `HitBatch` handler: if `auto_follow`, pin `hit_sel = hits.len() - 1` |
| `render.rs` | `[↓ LIVE]` badge in panel title; updated hint text to show `Home: live` |

---

## Files Modified

```
crates/ferrite-carver/Cargo.toml                  — tempfile dev-dep
crates/ferrite-carver/src/post_validate/mod.rs    — validate_png_file + post-IDAT tail check
crates/ferrite-carver/src/post_validate/tests.rs  — 6 new tests (4 file-walk + 2 tail-check)
crates/ferrite-tui/src/screens/carving/events.rs  — auto_follow pin in HitBatch
crates/ferrite-tui/src/screens/carving/extract.rs — route PNG to validate_png_file
crates/ferrite-tui/src/screens/carving/input.rs   — auto_follow set/clear on nav keys
crates/ferrite-tui/src/screens/carving/mod.rs     — auto_follow field + init
crates/ferrite-tui/src/screens/carving/render.rs  — [↓ LIVE] badge in title
```

## Test Results

- **799 unit tests** — all passing
- **0 clippy warnings**
- Real-world verification: both `ferrite_png_2564460998851.png` and
  `ferrite_png_2564461650457.png` from the 4 TB test run now return `Corrupt`
