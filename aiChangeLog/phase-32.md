# Phase 32 — ZIP False-Hit Suppression & min_size Enforcement

## Problem

File carving was producing large numbers of spurious ZIP files that opened in
WinRAR / 7-Zip showing only a single empty folder entry (e.g. `patch/`) and
the error "Unexpected end of archive".

**Root causes:**

1. The ZIP header magic `PK\x03\x04` is the *Local File Header* signature —
   it appears at the start of **every entry** inside a multi-file ZIP archive,
   not just at the archive's beginning.  A ZIP like `patch.zip` containing a
   `patch/` subdirectory generates spurious hits on each internal LFH,
   including directory entries that contain no file data.

2. `min_size` was stored in `Signature` and parsed from `signatures.toml` but
   was **never actually enforced** during scanning.  The 512-byte threshold
   documented in the ZIP signature was silently ignored.

## Changes

### `crates/ferrite-carver/src/signature.rs`
- Added `pre_validate_zip: bool` field to `Signature` (serde default = false).
- Added `pre_validate: Option<String>` to the internal `RawSig` TOML
  deserialiser.  `pre_validate = "zip"` sets `pre_validate_zip = true`.

### `crates/ferrite-carver/src/scan_search.rs`
- Added `zip_local_header_is_file(data, pos) -> bool` helper that parses the
  30-byte ZIP Local File Header at the hit position and returns `false` (reject)
  when any of the following hold:
  - `version_needed > 63` (no known extractor beyond PKWARE 6.3)
  - `file_name_length == 0` or `> 512`
  - `compression_method` not in PKWARE registered set
    {0, 8, 9, 12, 14, 19, 93, 95, 96, 97, 98, 99}
  - Filename ends with `/` — directory entry, never contains file data
- `find_all` now calls this check when `sig.pre_validate_zip` is set.
- Added 7 unit tests covering: directory entry filtered, file entry kept,
  invalid version filtered, unknown compression filtered, zero fname len
  filtered, end-to-end `find_all` integration for both cases.

### `crates/ferrite-carver/src/scanner.rs`
- `scan_impl` now enforces `min_size` after collecting each chunk's hits:
  ```rust
  chunk_hits.retain(|h| {
      h.signature.min_size == 0
          || h.byte_offset.saturating_add(h.signature.min_size) <= device_size
  });
  ```
- Added two unit tests: `min_size_filters_hit_near_device_end` and
  `min_size_keeps_hit_with_sufficient_space`.

### `config/signatures.toml`
- Added `pre_validate = "zip"` to the ZIP/Office signature entry.
- Updated the comment block to document the pre-validation behaviour.

### All struct literal `Signature { … }` in tests
- Added `pre_validate_zip: false` to every direct struct literal
  (`carver_io.rs`, `scanner.rs`, `scan_search.rs`, `ferrite-tui/mod.rs`,
  `tests/size_hints.rs`).

## Test results

```
cargo test --workspace   → 246 tests, 0 failures
cargo clippy -- -D warnings → clean
```
