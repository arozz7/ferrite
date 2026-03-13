# Phase 23 — Hit Preview Panel

## Overview

Implemented a real-time Hit Preview Panel for the Carving screen. Pressing `v`
while focus is on the Hits panel toggles an inline preview pane that shows
file-format metadata and, for images on true-color terminals, a halfblock pixel
rendering of the first 64 KiB of the selected carve hit.

---

## Files Changed

### Deleted
- `crates/ferrite-tui/src/screens/carving.rs` (1 536 lines) — replaced by the
  directory module below.

### Created
| File | Lines | Purpose |
|---|---|---|
| `crates/ferrite-tui/src/screens/carving/mod.rs` | ~900 | Logic, state, types, key handling, helpers, tests |
| `crates/ferrite-tui/src/screens/carving/render.rs` | ~340 | All `impl CarvingState` render methods |
| `crates/ferrite-tui/src/screens/carving/preview.rs` | ~680 | New preview module |
| `aiChangeLog/phase-23.md` | — | This file |

### Modified
| File | Change |
|---|---|
| `Cargo.toml` | Added `image = { version = "0.25", default-features = false, features = ["jpeg", "png", "bmp", "gif"] }` to `[workspace.dependencies]` |
| `crates/ferrite-tui/Cargo.toml` | Added `image = { workspace = true }` to `[dependencies]` |
| `crates/ferrite-tui/src/app.rs` | Updated screen-5 help line to include `v: preview` |

---

## Module Split — carving.rs → carving/

Rust's module system automatically finds `carving/mod.rs` once `carving.rs` is
deleted, so `pub mod carving;` in `screens/mod.rs` required no change.

`mod render;` and `mod preview;` are private submodules declared inside
`carving/mod.rs`. Rust 2018 privacy rules allow child modules to access parent
private items, so the explicit `use super::…` imports in `render.rs` work
without any visibility changes.

---

## New State Fields (CarvingState)

```rust
pub(crate) show_preview: bool,
pub(crate) current_preview: Option<preview::HitPreview>,
preview_hit_idx: Option<usize>,      // cache invalidation key
pub(crate) color_cap: ColorCap,
```

`color_cap` is detected once at `CarvingState::new()` by inspecting
`$COLORTERM`, `$WT_SESSION`, `$TERM_PROGRAM`, and `$TERM`.

---

## New Key Binding

| Key | Condition | Effect |
|---|---|---|
| `v` | focus = Hits | Toggle preview panel on/off |
| `↑` / `↓` | focus = Hits, preview on | Navigate + auto-refresh preview |

---

## preview.rs — Parsers Implemented

| Extension | Metadata extracted |
|---|---|
| `jpg` / `jpeg` | Width × Height (from SOF), EXIF date |
| `png` | Width × Height (IHDR), bit depth, colour type |
| `bmp` | Width × Height, bits-per-pixel |
| `gif` | Width × Height, version (GIF87a / GIF89a) |
| `mp3` | ID3v2 version, tag size, Title, Artist, Album |
| `flac` | Sample rate, channels, bit depth, duration |
| `zip` | First 8 filenames from local file headers |
| `pdf` | /Title, /Author, /Creator from PDF dictionary |
| `db` | Page size, page count, estimated size, encoding |
| `exe` / `dll` | Architecture, subsystem, section count |
| *(other)* | Format label only (graceful fallback) |

Image formats (JPEG, PNG, BMP, GIF) additionally attempt full decoding with
the `image` crate for halfblock pixel rendering when the terminal supports
true-colour or 256-colour.

---

## Implementation Notes

- Byte parsing uses `u16/u32/u64::from_be_bytes` / `from_le_bytes` from stdlib
  only — no `byteorder` crate.
- `image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?.decode()`
  used per image 0.25 API (top-level `ImageReader`, not `io::Reader`).
- Preview reads are capped at 64 KiB, rounded up to sector alignment.
- `refresh_preview()` is cached by `preview_hit_idx` so repeated Up/Down on the
  same row does not re-read the device.
- All parsers return `None` or a zero-metadata `HitPreview` on parse failure —
  no panics.

---

## Quality Gates

```
cargo test  --workspace       → 219 passed, 0 failed
cargo clippy --workspace -- -D warnings  → 0 errors, 0 warnings
cargo fmt   --check           → 0 diffs in Phase-23 files
```
