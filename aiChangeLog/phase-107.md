# Phase 107 — Size Hint Walkers (AU / MIDI)

## Summary

Added dedicated size-hint walkers for Sun AU audio and Standard MIDI File formats.
Both formats have deterministic size fields in their headers that allow exact file-size
derivation, eliminating over-extraction against their 500 MiB / 10 MiB `max_size` caps.

## New size hints

### `SizeHint::Au` — Sun/NeXT AU audio

`total_size = data_offset (u32 BE @4) + data_size (u32 BE @8)`

Falls back to `max_size` when `data_size == 0xFFFF_FFFF` (streaming / unknown length).

### `SizeHint::Midi` — Standard MIDI File

Walks the `nTracks` (u16 BE @10) MTrk chunks from offset 14 onward, summing
`8 + track_data_len` per chunk:

`total_size = 14 + Σ (8 + MTrk[i].track_len)`

Returns `None` if any MTrk chunk header is unreadable or has wrong magic.

## Formats updated in `config/signatures.toml`

| Format | `size_hint_kind` | Max-size before | Typical file size |
|--------|-----------------|-----------------|-------------------|
| AU     | `"au"`          | 500 MiB         | 1–50 MiB          |
| MIDI   | `"midi"`        | 10 MiB          | 10 KB–5 MB        |

## Files changed

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/signature.rs` | Added `Au` and `Midi` variants; `kind_name` + TOML parser arms |
| `crates/ferrite-carver/src/size_hint/au.rs` | New — `au_hint()` |
| `crates/ferrite-carver/src/size_hint/midi.rs` | New — `midi_hint()` |
| `crates/ferrite-carver/src/size_hint/mod.rs` | Registered `au` and `midi` modules; dispatch arms added |
| `crates/ferrite-carver/src/size_hint/tests.rs` | 9 new unit tests (4 AU + 5 MIDI) |
| `config/signatures.toml` | Added `size_hint_kind = "au"` and `size_hint_kind = "midi"` |

## Test results

- `ferrite-carver`: **662 tests** (up from 653), all passing
- Full workspace: all passing
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean
