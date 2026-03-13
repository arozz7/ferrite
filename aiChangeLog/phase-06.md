# Phase 06 — ferrite-carver

## Summary

Implemented the `ferrite-carver` crate: a parallel, signature-based file
carving engine that scans a `BlockDevice` for known file-type magic bytes and
extracts matching candidates to any `Write` sink.

## New crate

`crates/ferrite-carver/` (4 source files, ~570 lines)

| File | Purpose |
|------|---------|
| `src/lib.rs` | Public API, `include_str!` integration test against real `signatures.toml` |
| `src/error.rs` | `CarveError` (thiserror) + `Result<T>` alias |
| `src/signature.rs` | `Signature`, `CarvingConfig`, TOML loader, `parse_hex()` |
| `src/scanner.rs` | `Carver`, `CarveHit`, `scan()`, `extract()`, I/O helpers |

## Public API

```rust
// Load signatures from the bundled TOML
let cfg = CarvingConfig::from_toml_str(include_str!("config/signatures.toml"))?;

// Scan the full device
let carver = Carver::new(Arc::clone(&device), cfg);
let hits = carver.scan()?; // Vec<CarveHit>, sorted by byte_offset

// Extract one hit
let mut out = Vec::new();
let bytes_written = carver.extract(&hits[0], &mut out)?;
```

## Design decisions

- **Overlapping chunk windows:** each chunk reads `scan_chunk_size + (max_header_len - 1)`
  bytes so headers straddling chunk boundaries are never missed.  Hits are
  only reported when `header_start < chunk_size` to prevent double-counting.
- **Parallel signature search:** `rayon::par_iter` over signatures within each
  chunk; `memchr` fast single-byte scan followed by full-header equality check.
- **Streaming extraction:** `stream_bytes` (no footer) and `stream_until_footer`
  (with footer) read in 256 KiB chunks.  A `tail` carry-over buffer of
  `footer.len() - 1` bytes detects footers that span extraction chunk
  boundaries without accumulating the full file in memory.
- **Invariant:** the `tail` buffer holds bytes that have been read but **not yet
  written**; both the flush path and the footer-found path write from
  `combined[0..]` to avoid silently dropping the tail on a match.
- **TOML format:** matches the existing `config/signatures.toml` schema
  (`[[signature]]` array with `name`, `extension`, `header`, `footer`,
  `max_size`).

## Built-in signatures covered

JPEG, PNG, PDF, ZIP/Office, GIF, BMP, MP3, MP4, RAR, 7-Zip (10 total).

## Test counts

| Crate | Tests |
|-------|-------|
| ferrite-carver | 20 new |
| Workspace total | 124 |

## Notable bug fixed during development

Initial `stream_until_footer` wrote `combined[tail.len()..end]` when a footer
was found, silently discarding the unwritten `tail` bytes.  Fixed by writing
`combined[0..end]` in both the footer-found and the flush path, with
`written += end` (not `end - tail.len()`).

## Workspace changes

- `Cargo.toml`: added `crates/ferrite-carver` to `[workspace.members]` and
  `ferrite-carver` to `[workspace.dependencies]`.
- New dependencies resolved: `rayon 1.11`, `memchr 2.8`,
  `crossbeam-{deque,epoch,utils}`, `rayon-core`.
