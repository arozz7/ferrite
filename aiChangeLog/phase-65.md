# Phase 65 — MPG / TS / M2TS Hit Deduplication (`min_hit_gap`)

**Date:** 2026-03-17
**Branch:** master
**Tests:** 680 total (was 677; +3 gap-suppressor unit tests) — all passing
**Clippy:** Clean (-D warnings)

---

## Problem

MPEG Program Stream (`mpg`) and MPEG Transport Stream (`ts` / `m2ts`) embed their
magic bytes at **every** pack/packet boundary throughout the file:

| Format | Magic | Recurrence |
|---|---|---|
| MPEG-PS | `00 00 01 BA` | Every pack (~2 KiB on DVD) |
| MPEG-TS | `0x47` | Every 188-byte packet |
| Blu-ray M2TS | `?? ?? ?? ?? 47` | Every 192-byte packet |

The pre-validators correctly accept every occurrence as a valid header, causing
the hit list to flood with hundreds or thousands of intra-file duplicates (e.g.
887 MPG hits from a single large file, all truncated at 4 GiB `max_size`).

---

## Solution: `min_hit_gap` field on `Signature`

### New field — `crates/ferrite-carver/src/signature.rs`

```rust
/// Minimum byte distance between consecutive hits of this signature (0 = disabled).
#[serde(default)]
pub min_hit_gap: u64,
```

Zero by default — fully backwards-compatible. All existing signatures are
unaffected. TOML parser extended with a matching `min_hit_gap` field on `RawSig`.

### Gap tracking — `crates/ferrite-carver/src/scanner.rs`

`scan_impl` now maintains `last_hit_by_sig: HashMap<String, u64>` across chunks.
After sorting each chunk's hits the retain pass applies:

```
accept = match last_hit_by_sig.get(sig_name) {
    None    => true,          // first hit — always accept
    Some(l) => offset >= l + min_hit_gap,
}
```

Kept hits update the tracker; rejected hits are silently dropped. State persists
across chunk boundaries so the 16 MiB gap works even when a single file spans
many 4 MiB scan chunks.

### Configuration — `config/signatures.toml`

| Signature | `min_hit_gap` | Rationale |
|---|---|---|
| MPEG-2 / MPEG-1 Program Stream | 16 MiB | Pack headers every ~2 KiB; real files >> 16 MiB |
| MPEG Transport Stream (TS) | 4 MiB | Sync bytes every 188 bytes |
| Blu-ray M2TS | 4 MiB | Sync bytes every 192 bytes |

---

## Session Resync — `D` key

For sessions started **before** this fix (with existing flood hits), press **`D`**
in the Hits panel to retroactively apply the gap filter to the current hit list.

- `dedup_hits_by_gap()` method on `CarvingState`: sorts and retains using the same
  logic as the scanner, then clamps `hit_sel` to valid range
- `D` key bound in `input.rs` (hits panel focus)
- Key shown in hits panel title bar: `D: dedup`

---

## Files Changed

| File | Change |
|---|---|
| `crates/ferrite-carver/src/signature.rs` | `min_hit_gap: u64` field + `RawSig` parser field |
| `crates/ferrite-carver/src/scanner.rs` | `last_hit_by_sig` HashMap across chunks; 3 new tests |
| `config/signatures.toml` | `min_hit_gap` on MPG (16 MiB), TS (4 MiB), M2TS (4 MiB) |
| `crates/ferrite-carver/src/scan_search.rs` | `min_hit_gap: 0` in test Signature literals |
| `crates/ferrite-carver/src/carver_io.rs` | `min_hit_gap: 0` in Signature literals |
| `crates/ferrite-carver/src/size_hint.rs` | `min_hit_gap: 0` in Signature literal |
| `crates/ferrite-carver/tests/size_hints.rs` | `min_hit_gap: 0` in all test Signature literals |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `dedup_hits_by_gap()` method; `min_hit_gap: 0` in test literal |
| `crates/ferrite-tui/src/screens/carving/input.rs` | `D` key → `dedup_hits_by_gap()` |
| `crates/ferrite-tui/src/screens/carving/render.rs` | `D: dedup` in hits panel help bar |
| `crates/ferrite-tui/src/screens/carving/user_sigs.rs` | `min_hit_gap: 0` in `to_signature()` |

---

## Tests Added (`scanner::tests` — 3 tests)

| Test | Asserts |
|---|---|
| `min_hit_gap_suppresses_nearby_hits` | Hits at 0 and 100 with gap=512: only 0 and 2000 kept |
| `min_hit_gap_zero_does_not_suppress` | gap=0 passes all three hits |
| `min_hit_gap_tracks_across_chunks` | Hit at 600 (chunk 1) suppressed by hit at 0 (chunk 0) with gap=1024 |
