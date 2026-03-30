# Phase 112 — NTFS Full-Path Resolution + Windows Path Sanitisation

## Summary
Fixed filesystem-indexed carving so that the original folder hierarchy is
recreated under the output directory.  Previously every file landed flat
(e.g. `vacation.jpg` instead of `Photos/vacation.jpg`) because
`enumerate_files()` hardcoded `path: format!("/{name}")` and never used
the MFT `parent_ref` chain.  A Windows-invalid-char sanitisation step was
also added to prevent `create_dir_all` silently failing on paths that
contain `:`, `*`, `?`, etc.

## Changes

### Modified files

**`crates/ferrite-filesystem/src/ntfs.rs`**
- Replaced the 1-line `enumerate_files()` stub with a two-phase
  implementation:
  - **Phase 1** — single MFT pass: directories go into
    `HashMap<mft_record_num, (dir_name, parent_ref)>`; file entries are
    accumulated with an empty `path`.
  - **Phase 2** — for each file, walk the `parent_ref` chain up to
    `ROOT_MFT_RECORD` (5), collecting directory names; depth-limited to 32
    to guard against cycles.  Reversed parts are joined as
    `/<dir>/…/<file>`.
- Added 2 new unit tests:
  - `enumerate_files_resolves_nested_path` — verifies `vacation.jpg` under
    `Photos` gets path `/Photos/vacation.jpg` and `notes.txt` in root gets
    `/notes.txt`.
  - `enumerate_files_flat_path_for_root_file` — verifies existing
    `hello.txt` still resolves to `/hello.txt`.
- Added helper `build_nested_image()` (records 5–8: root, Photos dir,
  vacation.jpg, notes.txt) used by the new tests.

**`crates/ferrite-tui/src/screens/carving/extract.rs`**
- In `filename_for_hit()`: added `.map(|s| s.replace([':', '*', '?', '<',
  '>', '|', '"'], "_"))` after the `.filter()` step so each path component
  is sanitised before being joined into the output path.  Prevents silent
  `create_dir_all` failures on Windows when filenames recovered from NTFS
  contain reserved characters.

## Tests
- 2 new unit tests in `ferrite-filesystem`
- All 1098+ workspace tests still pass
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo fmt --check` clean
