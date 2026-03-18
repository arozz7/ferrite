# Phase 47b — SHA-256 Image Integrity Hash

## Summary
Adds post-completion SHA-256 integrity hashing to the imaging engine. After all
five passes complete, the output image is read sequentially and hashed.  The
digest is written to a `<image>.sha256` sidecar file in `sha256sum`-compatible
format and surfaced in the TUI Statistics panel.

## Changes

### ferrite-imaging
- `Cargo.toml` — added `sha2 = { workspace = true }` dependency.
- `src/hash.rs` (new) — public module with three functions:
  - `hash_file(path)` — streams a file in 64 KiB chunks, returns lowercase hex SHA-256.
  - `sidecar_path(image_path)` — returns `<image>.sha256` path.
  - `hash_and_save(image_path)` — calls `hash_file`, writes sidecar in `sha256sum` format, returns hex.
- `src/lib.rs` — exports `pub mod hash`.
- `src/engine.rs` — added `sha256_matches_source_data` integration test that verifies
  `hash::hash_file` output matches `sha2::Sha256::digest` of the raw output bytes.

### ferrite-tui
- `src/screens/imaging/mod.rs`:
  - Replaced local `compute_sha256` helper with `ferrite_imaging::hash::hash_and_save`
    (eliminates duplicate hashing code; sidecar is now written automatically).
- `src/screens/imaging/render.rs`:
  - `stats` binding changed from `let mut` to `let` (clippy fix).
  - SHA-256 display changed from a plain string append to a styled `Line`:
    - **Green** when imaging was a fresh start (`imaging_resumed = false`).
    - **Amber** with `⚠ resumed — hash covers new data only` warning when
      `imaging_resumed = true` (hash does not cover bytes from prior sessions).
  - Stats/hash/write-blocker lines are now composed via `Text::push_line` for
    proper per-span styling.

## Tests added
- `ferrite_imaging::hash` — 5 unit tests:
  - `hash_file_known_value` — empty file → known SHA-256 empty-string digest.
  - `hash_file_nonempty` — non-empty file → 64-char hex output.
  - `hash_and_save_writes_sidecar` — sidecar file created with correct prefix.
  - `sidecar_path_appends_extension` — `/tmp/disk.img` → `/tmp/disk.img.sha256`.
  - `hash_file_nonexistent_returns_none` — missing file → `None`.
- `ferrite_imaging::engine::sha256_matches_source_data` — full engine run on a
  `MockBlockDevice`, hash cross-checked against direct `sha2::Sha256::digest`.

## Sidecar Format
```
<hex64>  <filename>\n
```
Compatible with `sha256sum --check disk.img.sha256`.
