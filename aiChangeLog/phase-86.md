# Phase 86 — File-based post-validators (Tier 1 + Tier 2 + EPUB)

## Problem addressed

The file carver was producing 384 nearly-identical `.db` files (all exactly
677,888 bytes) from a single 4TB carving run.  Investigation showed that
SQLite's 16-byte magic (`SQLite format 3\x00`) appeared at every 4096-byte
(NTFS cluster) boundary — old transaction-journal copies of the database
header page scattered by Windows.  The size hint faithfully computed
`page_size × page_count = 677,888` from the valid header, then carved
677,888 bytes of unrelated disk data as the "database body".  Pages 2+ had
overwhelmingly invalid B-tree type bytes (only ~1 in 5 passed), confirming
the bodies were garbage.

The root cause is general: any format with a valid-looking header that the
size hint can compute a size from — SQLite, TIFF, EVTX, RIFF, etc. — can
produce files where the body is random disk data if the header happens to
appear at a non-canonical disk location.

## Solution

Added file-based (seek-based) structural validators for 8 additional formats,
parallel to the existing `validate_png_file` and `validate_pdf_file`.  These
validators open the extracted file directly and walk internal structure with
`Seek`, catching corruption in the "dead zone" that fixed head+tail buffers
cannot reach.

## Files added

- `crates/ferrite-carver/src/post_validate/file_validators.rs`
  — `validate_png_file`, `validate_pdf_file`, `validate_sqlite_file`
  (+ helpers `parse_last_startxref`, `looks_like_xref` moved from `mod.rs`)

- `crates/ferrite-carver/src/post_validate/binary_validators.rs`
  — `validate_evtx_file`, `validate_riff_file`, `validate_exe_file`

- `crates/ferrite-carver/src/post_validate/structural_validators.rs`
  — `validate_flac_file`, `validate_elf_file`, `validate_regf_file`,
    `validate_tiff_file`
  (split from `binary_validators.rs` to comply with 600-line hard limit)

- `crates/ferrite-carver/src/post_validate/tests_binary.rs`
  — 30 unit tests for all 8 new validators

## Files modified

| File | Change |
|------|--------|
| `post_validate/mod.rs` | Removed moved functions; added `mod` declarations + `pub use` re-exports; added `"epub"` to EOCD arm in `validate_extracted` |
| `post_validate/tests.rs` | Added explicit import for moved `pub(crate)` helpers |
| `ferrite-tui/src/screens/carving/extract.rs` | Expanded the file-based validator dispatch at both call sites (single extract + bulk worker) to cover all 10 new extensions |

## Validator logic summary

| Validator | Extension(s) | Check |
|-----------|-------------|-------|
| `validate_sqlite_file` | `db` | Schema page type byte @100 in valid set; pages 2–6 require >50% valid B-tree type bytes |
| `validate_evtx_file` | `evtx` | `ElfFile\x00` magic; first chunk `ElfChnk\x00` @4096; `free_space_offset` in `[512, 65536]` |
| `validate_riff_file` | `wav`, `avi`, `webp`, `aiff` | Form magic `RIFF`/`FORM`; ≥3 of first 10 chunks have ASCII fourCC within file bounds |
| `validate_exe_file` | `exe` | `e_lfanew` in range; `PE\x00\x00` at that offset; majority of section table entries have raw data within file bounds |
| `validate_flac_file` | `flac` | First metadata block = STREAMINFO (type 0, length 34); all block types in `[0, 6]` |
| `validate_elf_file` | `elf` | `e_phentsize` correct for class; majority of program headers have `p_offset + p_filesz ≤ file_size` |
| `validate_regf_file` | `regf` | `regf` magic; `hbin` magic at offset 4096 |
| `validate_tiff_file` | `tif`, `nef`, `arw`, `cr2`, `rw2`, `orf`, `pef`, `sr2`, `dcr` | LE/BE IFD chain walk; entry type codes 1–12; external data pointers within file; majority of entries valid |

**Quick fix:** `"epub"` added to `validate_zip_eocd` arm in `validate_extracted` — EPUB is a ZIP container and the EOCD check was already available but not wired for that extension.

## Test results

- **850 unit tests** — all passing (up from 820)
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — clean
