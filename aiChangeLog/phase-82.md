# Phase 82 — PDF linearized size hint + TS/M2TS min_size increase

## Summary
Fixes oversized PDF extraction for linearized PDFs and eliminates tiny
false-positive M2TS/TS files from real-world 4TB carving.

## Changes

### 1. PDF linearized size hint — `SizeHint::Pdf`
- **`size_hint/pdf.rs`** — NEW.  Reads the first ~256 bytes looking for
  `/Linearized` and `/L <n>`.  Returns the declared file length for
  linearized PDFs; non-linearized PDFs return `None` (footer search).
- **`signature.rs`** — added `Pdf` variant + TOML parser arm.
- **`size_hint/mod.rs`** — `mod pdf;` + dispatch arm.
- **`config/signatures.toml`** — PDF signature now uses `size_hint_kind = "pdf"`.
  Linearized PDFs that declare `/L 15206` will extract exactly 15,206 bytes
  instead of sweeping 25 MB to the next `%%EOF`.

### 2. TS/M2TS min_size increase — `config/signatures.toml`
- **TS:** `min_size` raised from 377 (3 packets) to 18,800 (100 packets).
- **M2TS:** `min_size` raised from 389 (3 packets) to 19,200 (100 packets).
- Eliminates false positives where random data coincidentally has `0x47`
  at the right stride offsets for a handful of packets.

### 3. Tests
- 3 new PDF size_hint tests: linearized with `/L`, non-linearized, and
  linearized without `/L` key.
- **763 tests passing**, clippy clean, fmt clean.

## Files Modified
- `config/signatures.toml` — PDF size_hint_kind, TS/M2TS min_size
- `crates/ferrite-carver/src/signature.rs` — SizeHint::Pdf variant
- `crates/ferrite-carver/src/size_hint/mod.rs` — Pdf dispatch
- `crates/ferrite-carver/src/size_hint/pdf.rs` — **NEW** linearized PDF reader
- `crates/ferrite-carver/src/size_hint/tests.rs` — 3 new PDF tests
