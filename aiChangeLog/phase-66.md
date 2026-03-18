# Phase 66 — Cross-Signature Suppression (`suppress_group`) + TTF Gap Fix

## Problem

Two carving false-positive storms observed during real scan:

1. **M2TS / TS duplicate hits** — every M2TS packet (4-byte timestamp + 0x47 sync
   byte) caused a spurious TS hit exactly 4 bytes later, because the `min_hit_gap`
   tracker was keyed per-signature-name, not per-group.  A scan would produce
   paired hits like:
   ```
   Blu-ray M2TS Video @ 0x1b62a7fff79 [TRUNC 2.0 GiB]
   MPEG Transport Stream (TS) @ 0x1b62a7fff7d [TRUNC 2.0 GiB]
   ```

2. **TTF rapid-fire false positives** — the 5-byte TTF header (`00 01 00 00 00`)
   appeared at 12-byte intervals in dense binary regions, producing large numbers
   of low-quality hits capped at 50 MiB each.

## Solution

### `suppress_group: Option<String>` on `Signature`

Added a new optional field to `Signature` in `ferrite-carver::signature`.  When
two or more signatures share the same string value, they advance a **shared** gap
counter — so a hit from one suppresses nearby hits from all others in the group.

- Signatures without a group continue to use their own `name` as the key
  (fully backward-compatible; no behaviour change for existing sigs).

### Scanner update (`ferrite-carver::scanner`)

The gap-tracking map in `Carver::scan()` now keys by
`suppress_group.as_deref().unwrap_or(&sig.name)` instead of always `sig.name`.
The same change was applied to `CarvingState::dedup_hits_by_gap()` in
`ferrite-tui`.

### `signatures.toml` changes

| Signature | Change |
|---|---|
| MPEG Transport Stream (TS) | `suppress_group = "mpeg_transport"` |
| Blu-ray M2TS Video | `suppress_group = "mpeg_transport"` |
| TrueType Font (TTF) | `min_hit_gap = 524_288` (512 KiB) |

## Files Changed

- `crates/ferrite-carver/src/signature.rs` — `suppress_group` field on `Signature` + `RawSig` + TOML mapping
- `crates/ferrite-carver/src/scanner.rs` — group-key gap suppression + new test `suppress_group_cross_sig_dedup`
- `crates/ferrite-carver/src/scan_search.rs` — `suppress_group: None` in test helpers
- `crates/ferrite-carver/src/carver_io.rs` — `suppress_group: None` in test helpers
- `crates/ferrite-carver/src/size_hint.rs` — `suppress_group: None` in test helper
- `crates/ferrite-carver/tests/size_hints.rs` — `suppress_group: None` in all test `Signature` literals
- `crates/ferrite-tui/src/screens/carving/mod.rs` — group-key in `dedup_hits_by_gap` + test literal
- `crates/ferrite-tui/src/screens/carving/user_sigs.rs` — `suppress_group: None` in user sig builder
- `config/signatures.toml` — `suppress_group` on TS + M2TS; `min_hit_gap` on TTF

## Tests

- All 658+ workspace tests pass.
- `cargo clippy --workspace -- -D warnings` clean.
- New unit test: `suppress_group_cross_sig_dedup` verifies that a hit from SigA
  at offset 0 suppresses SigB at offset 4 when both share a group.
