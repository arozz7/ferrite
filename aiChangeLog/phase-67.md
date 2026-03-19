# Phase 67 — MpegTs Size Hint + M2TS/TS/GZip Extraction Improvements

## Problems

1. **Video blend** — extracted M2TS/TS files contained data from two or more
   original video files.  Extraction always wrote exactly `max_size` (2 GiB)
   bytes regardless of where the actual stream ended.

2. **M2TS/TS hit flood** — `min_hit_gap = 4 MiB` generated one hit per 4 MiB
   of video data on the disk.  A 30 GB Blu-ray archive produced ~7 500 hits,
   each initiating a 2 GiB extraction — tens of terabytes of I/O.

3. **GZip catastrophic false positives** — `1F 8B` is a 2-byte magic that
   appears coincidentally inside compressed video/audio data.  With a 2 GiB
   cap, every false positive wrote 2 GiB of garbage to disk.

## Solution

### 1. `SizeHint::MpegTs { ts_offset: u8, stride: u16 }` (new variant)

Added to `ferrite-carver::signature::SizeHint`.  The resolver
(`mpeg_ts_size_hint` in `size_hint.rs`) walks the stream packet by packet:

- Reads in blocks of 1 024 packets (≈ 192 KiB for M2TS, ≈ 188 KiB for TS).
- At each expected packet boundary, checks that byte `ts_offset` equals `0x47`.
- Stops and returns the size of the valid run when **10 consecutive** packets
  fail the sync-byte check.
- Caps the scan at `max_size` (new parameter on `read_size_hint`).

| Format | `ts_offset` | `stride` |
|--------|-------------|----------|
| TS     | 0           | 188      |
| M2TS   | 4           | 192      |

### 2. `read_size_hint` gains a `max_size: u64` parameter

Prevents the MpegTs walker from reading further than the signature's `max_size`
window.  All existing hint variants ignore this parameter; the single call site
in `scanner.rs` passes `sig.max_size`.

### 3. `signatures.toml` changes

| Signature | Change |
|---|---|
| MPEG Transport Stream (TS) | `size_hint_kind = "mpeg_ts"`, `size_hint_ts_offset = 0`, `size_hint_stride = 188`; `min_hit_gap` 4 MiB → **512 MiB** |
| Blu-ray M2TS Video | `size_hint_kind = "mpeg_ts"`, `size_hint_ts_offset = 4`, `size_hint_stride = 192`; `min_hit_gap` 4 MiB → **512 MiB** |
| GZip Compressed | `max_size` 2 GiB → **512 MiB** |

## Effects

- **Video blend eliminated** — each extraction terminates at the real end of
  the transport stream, not at the 2 GiB cap.
- **M2TS/TS hit count reduced 128×** — 512 MiB gap vs 4 MiB; a 30 GB Blu-ray
  archive now produces ~60 hits instead of 7 500.
- **GZip false-positive damage limited** — 512 MiB cap instead of 2 GiB.

## Files Changed

- `crates/ferrite-carver/src/signature.rs` — `SizeHint::MpegTs` variant; `RawSig` fields `size_hint_ts_offset`/`size_hint_stride`; TOML mapping; `kind_name()`
- `crates/ferrite-carver/src/size_hint.rs` — `max_size` parameter on `read_size_hint`; `mpeg_ts_size_hint()` function; 5 new unit tests; updated existing test calls
- `crates/ferrite-carver/src/scanner.rs` — updated `read_size_hint` call to pass `sig.max_size`
- `config/signatures.toml` — TS/M2TS size hints + min_hit_gap; GZip max_size

## Tests

- All 342+ ferrite-carver tests pass (5 new: `mpeg_ts_exact_stream_size`,
  `mpeg_m2ts_exact_stream_size`, `mpeg_ts_no_valid_packets_returns_none`,
  `mpeg_ts_max_size_caps_scan`, `mpeg_ts_stops_on_invalid_run`).
- `cargo clippy --workspace -- -D warnings` clean.
