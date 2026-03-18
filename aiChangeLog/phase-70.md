# Phase 70 — Pre-Validator Hardening (EML, Shebang)

## Problem

Two more signatures produced floods of false-positive hits on a 4 TB
drive scan:

- **EML** ("From ", 5-byte ASCII) — XMP/RDF metadata embedded in photos
  contains `From rdf:parseType="Resource">` which matched the mbox
  "From " header.  Every hit extracted 50 MiB of binary image data as
  a supposed email file.

- **Shebang** ("#!", 2 bytes) — `#!/` followed by binary garbage passes
  the old validator (which only checked byte 2 == `/`).  The 3-byte
  effective check `#!` + `/` appears frequently in binary data.

## Solution

### EML (`validate_eml`) — '@' sign requirement

| Check | Old | New |
|-------|-----|-----|
| Byte 5 | printable ASCII | printable ASCII (unchanged) |
| Bytes 5–85 | — | all printable + must contain `@` |

Real mbox "From " lines always contain a sender email address with `@`.
XMP/RDF content like `From rdf:parseType=...` has no `@` and is rejected.

| Setting | Old | New |
|---------|-----|-----|
| `max_size` | 50 MiB | **10 MiB** |
| `min_hit_gap` | 0 | **4 MiB** |

### Shebang (`validate_shebang`) — path prefix validation

| Check | Old | New |
|-------|-----|-----|
| Byte 2 | == `/` | == `/` (unchanged) |
| Byte 3 | — | must be `b` (bin) or `u` (usr) |
| Bytes 4–6 | — | must be printable ASCII (0x20–0x7E) |

All standard Unix interpreter paths start with `/bin/` or `/usr/`.  This
rejects `#!/` followed by binary garbage (the real false positive was
`#!/\x1e\xb0\x3e\x67...`).

| Setting | Old | New |
|---------|-----|-----|
| `min_size` | 3 | **16** |
| `max_size` | 10 MiB | **1 MiB** |
| `min_hit_gap` | 0 | **1 MiB** |

## Files Changed

- `crates/ferrite-carver/src/pre_validate.rs` — rewritten `validate_eml`,
  `validate_shebang`; 11 new/updated unit tests
- `config/signatures.toml` — EML max_size 50→10 MiB, min_hit_gap 4 MiB;
  Shebang min_size 3→16, max_size 10→1 MiB, min_hit_gap 1 MiB

## Tests

- 378 ferrite-carver tests pass (8 new).
- `cargo clippy -p ferrite-carver --all-targets -- -D warnings` clean.
