# Phase 69 — Pre-Validator Hardening (XML, ICO) — Rounds 1 & 2

## Problem

The carver produced floods of false-positive hits for two signatures with
weak magic bytes:

- **XML** (`<?xml`, 5-byte ASCII) — XMP metadata blocks embedded in RAW
  photos contain valid `<?xml version="1.0"...` declarations.  Every
  photo's XMP block triggers a hit, extracting 5 MiB of mostly binary
  garbage from the surrounding image data.

- **ICO** (`00 00 01 00`, 4 bytes — 3 of which are zeros) — `00 00 01 00`
  appears inside MP4/video data constantly.  The validator was too weak
  (count-only check), so hits passed validation and extracted 1 MiB each.

## Solution

### XML — `<?xml version` check + reduced extraction window

| Setting | Old | New |
|---------|-----|-----|
| Validator | byte 5 == space | bytes 5–12 == ` version` |
| `max_size` | 5 MiB | **512 KiB** |
| `min_hit_gap` | 0 | **4 MiB** |

The `<?xml version` check eliminates non-XML false positives.  Reducing
`max_size` from 5 MiB to 512 KiB limits disk waste from XMP metadata hits
(which are valid XML but not useful standalone files).  The 4 MiB gap
prevents clustered XMP blocks from flooding the hit list.

### ICO — directory entry bounds checking

| Check | Old | New |
|-------|-----|-----|
| Image count @4 | ∈ [1, 200] | ∈ [1, 200] (unchanged) |
| Reserved byte @9 | — | must be 0 |
| Planes @10–11 | — | must be 0 or 1 |
| BPP @12–13 | — | must be in {0, 1, 4, 8, 16, 24, 32} |
| Planes + BPP | — | cannot both be 0 |
| Data size @14–17 | — | must be in (0, 1 MiB] |
| Data offset @18–21 | — | must be in [6+16×count, 1 MiB] |

The key additions in round 2 were:
- **Planes/BPP both-zero rejection** — catches the MP4 false positive
  (bytes `00 00 01 00 15 00 00 00 0a 00 00 00 00 00 ...`)
- **Data size ≤ 1 MiB** — rejects entries claiming impossible sizes
- **Data offset ≤ 1 MiB** — rejects offsets beyond the ICO max_size

Also `min_hit_gap = 512 KiB` in signatures.toml.

## Files Changed

- `crates/ferrite-carver/src/pre_validate.rs` — tightened `validate_xml`,
  `validate_ico`; 15 new unit tests
- `config/signatures.toml` — XML `max_size` 5 MiB → 512 KiB,
  `min_hit_gap` 4 MiB; ICO `min_hit_gap` 512 KiB

## Tests

- 370 ferrite-carver tests pass (15 new).
- `cargo clippy -p ferrite-carver --all-targets -- -D warnings` clean.
