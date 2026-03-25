# Phase 105 — Backlog Cleanup: PAR2, WOFF1 Size Hint, Roadmap Update

**Date:** 2026-03-25
**Branch:** master
**Status:** Complete

## Summary

Three backlog items that were overlooked during Phases 100–104:

1. **PAR2 (Parchive)** — new signature; was planned in the original Tier B gap
   analysis but never implemented.
2. **WOFF1 size hint** — `length` field (u32 BE @8) identical to WOFF2's; added in
   Phase 104 for WOFF2 but missed for WOFF1.
3. **Roadmap documentation** — executive summary and header still said 129 signatures /
   1006 tests / "reviewed as of Phase 102".

Signature count: **139 → 140**.

## Changes

### New Signature

| # | Name | Header | Pre-validate | Max size |
|---|------|--------|--------------|----------|
| 140 | Parchive PAR2 | `PAR2\0PKT` (8 B) | `Par2`: packet_length u64 LE @8 ≥ 64 | 1 GiB |

**TUI group:** `par2` → **Archives**

### WOFF1 Size Hint

`config/signatures.toml` — added `size_hint_offset = 8`, `size_hint_len = 4`,
`size_hint_endian = "be"` to the WOFF Web Font entry.  The `length` field (u32 BE @8)
is the exact total WOFF1 file size — identical position to WOFF2.

### New Pre-validator

| Variant | Logic |
|---------|-------|
| `Par2` | `packet_length` u64 LE @8 must be ≥ 64 (minimum PAR2 packet header size) |

### Roadmap Documentation

`docs/ferrite-feature-roadmap.md` updated:
- Header: "Phase 102 (129 signatures)" → "Phase 104 (140 signatures)"
- Executive summary: 129 → 140 signatures, 1006 → ~1042 tests, Phase 101 → Phase 104
- "File Type Coverage" Archives row: added AFF + PAR2

## Files Changed

| File | Change |
|------|--------|
| `config/signatures.toml` | +1 `[[signature]]` (PAR2); WOFF1 size hint fields |
| `crates/ferrite-carver/src/pre_validate.rs` | +1 enum variant, +1 validator, +5 unit tests |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | `par2` → Archives |
| `crates/ferrite-carver/src/lib.rs` | assertion 139 → 140 |
| `docs/ferrite-feature-roadmap.md` | Header + executive summary updated |
| `aiChangeLog/phase-105.md` | This file |

## Test Results

- **642 tests** in ferrite-carver (up from 637; +5 new PAR2 validator tests)
- All workspace tests pass; clippy clean with `-D warnings`
