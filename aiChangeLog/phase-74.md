# Phase 74: Refactor size_hint.rs into Module Directory

## Summary
Split `size_hint.rs` (938 lines, over 600-line hard limit) into a module directory
with one file per size-hint variant. Logic-preserving refactor only.

## Changes

### Deleted
- `crates/ferrite-carver/src/size_hint.rs` (938 lines)

### Created: `crates/ferrite-carver/src/size_hint/` module directory

| File | Contents | Lines |
|------|----------|-------|
| `mod.rs` | `read_size_hint()` dispatcher + module declarations | ~90 |
| `helpers.rs` | `read_u16`, `read_u32`, `read_u64` LE/BE helpers | ~40 |
| `linear.rs` | `linear_hint()`, `linear_scaled_hint()` | ~70 |
| `ole2.rs` | `ole2_hint()` | ~25 |
| `sqlite.rs` | `sqlite_hint()` | ~25 |
| `seven_zip.rs` | `seven_zip_hint()` | ~15 |
| `ogg.rs` | `ogg_stream_hint()` | ~65 |
| `isobmff.rs` | `isobmff_hint()` | ~75 |
| `tiff.rs` | `tiff_size_hint()`, `raf_size_hint()` | ~240 |
| `mpeg_ts.rs` | `mpeg_ts_size_hint()` | ~60 |
| `tests.rs` | All existing tests (ISOBMFF, TIFF, RAF, MPEG-TS) | ~253 |

## Verification
- `cargo test --workspace` — 747 tests pass
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
