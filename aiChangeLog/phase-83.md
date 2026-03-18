# Phase 83 — PNG/GIF size hints + CRC-32 post-validation + HTML body validation

## Problem

1. **False IEND truncation (PNG):** PNG extraction relied on scanning raw bytes
   for the IEND footer pattern. When these 8 bytes appeared by coincidence
   inside compressed IDAT chunk data, the carver stopped early.

2. **Corrupt PNGs marked Complete:** On fragmented/damaged drives, the PNG
   chunk structure and IEND footer may be intact, but intermediate sectors
   contain data from other files. These files rendered as partial images
   (top portion visible, rest black) but were marked `Complete` because the
   IEND footer was present.

3. **Blank HTML files:** Kindle/EPUB e-book scaffold fragments are structurally
   valid HTML (correct `<!DOCTYPE>`, `<body>`, `</html>`) but contain only
   empty `<div>` containers with no readable text — wasting extraction space.

4. **Truncated GIF files:** GIF's footer is just `00 3B` (2 bytes), which
   easily false-matches inside LZW compressed image data. The carver stopped
   at the first `00 3B` occurrence, producing truncated GIFs (e.g. 9 KiB for
   a 697×573 image that should be much larger).

## Solution

### SizeHint::Png — PNG chunk walker
Walks the PNG chunk structure by reading each chunk's 4-byte big-endian length
field and advancing (IHDR → IDAT → … → IEND). Returns the exact file size.

### SizeHint::Gif — GIF block walker
Walks the GIF block structure: header → GCT → extension/image blocks → sub-
blocks → trailer. Follows the length-prefixed sub-block chain rather than
scanning for `00 3B`, so false footer matches inside LZW data are ignored.

### PNG CRC-32 Post-Validation
Extended `validate_extracted()` to accept a `head` buffer (first 8 KiB of the
extracted file). For PNG, the validator walks all chunks that fit within the
head buffer and verifies their CRC-32 checksums using `crc32fast`. If any
chunk has a bad CRC (sector-level corruption), the file is marked `Corrupt`.

### HTML Body Content Validation
Added `validate_html()` that checks:
1. File ends with `</html>` (footer check).
2. The `<body>` must contain at least 32 characters of visible text (non-
   whitespace, outside of HTML tags). Empty scaffold fragments are marked
   `Corrupt` and auto-deleted by skip-corrupt mode.

## Changes

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/signature.rs` | Added `SizeHint::Png`, `SizeHint::Gif` variants + TOML parser arms |
| `crates/ferrite-carver/src/size_hint/png.rs` | **New** — PNG chunk walker + 5 unit tests |
| `crates/ferrite-carver/src/size_hint/gif.rs` | **New** — GIF block walker + 6 unit tests |
| `crates/ferrite-carver/src/size_hint/mod.rs` | Wired `Png` and `Gif` dispatch |
| `crates/ferrite-carver/src/post_validate.rs` | Added `head` param to `validate_extracted()`; PNG CRC-32 verification; HTML body content check |
| `crates/ferrite-carver/Cargo.toml` | Added `crc32fast` dependency |
| `crates/ferrite-tui/src/screens/carving/extract.rs` | Added `read_file_head()` helper; pass head to `validate_extracted()` |
| `config/signatures.toml` | Added `size_hint_kind` for PNG and GIF signatures |

## Tests

### Size hint — PNG (size_hint::png)
- `png_hint_minimal` — IHDR + IDAT + IEND returns exact size
- `png_hint_with_offset` — chunk walk works at non-zero device offset
- `png_hint_false_iend_in_idat` — IEND bytes inside IDAT do NOT cause early stop
- `png_hint_truncated_header` — returns None on truncated file
- `png_hint_corrupt_chunk_type` — returns None on non-ASCII chunk type

### Size hint — GIF (size_hint::gif)
- `gif_hint_minimal` — minimal 1×1 GIF returns exact size
- `gif_hint_with_offset` — works at non-zero device offset
- `gif_hint_false_footer_in_lzw` — `00 3B` inside LZW data does NOT cause early stop
- `gif_hint_with_gce_extension` — walks past Graphic Control Extension
- `gif_hint_with_gct` — handles Global Color Table correctly
- `gif_hint_truncated` — returns None when trailer missing

### Post-validation — PNG
- `png_complete_with_valid_ihdr_crc` — valid IHDR CRC → Complete
- `png_corrupt_with_bad_ihdr_crc` — corrupted IHDR data → Corrupt
- `png_corrupt_with_non_alpha_chunk_type` — non-ASCII chunk type → Corrupt

### Post-validation — HTML
- `html_complete_with_body_content` — body with 32+ text chars → Complete
- `html_corrupt_empty_body` — Kindle fragment with empty divs → Corrupt
- `html_corrupt_missing_closing_tag` — no `</html>` → Corrupt
- `html_complete_uppercase_tags` — uppercase HTML tags accepted
- `html_corrupt_only_whitespace_in_body` — whitespace-only body → Corrupt
