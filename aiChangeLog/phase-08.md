# Phase 08 — Recovery Quality Improvements

## Summary

Implemented five targeted improvements recommended by the senior data-recovery
engineer review.  All changes are additive (no breaking API changes) and come
with tests.

---

## Changes by Item

### A1 — File Extraction in File Browser

**`crates/ferrite-tui/src/screens/file_browser.rs`**

- Added `extract_status: Option<String>` field to `FileBrowserState`.
- Added `KeyCode::Char('e')` handler in `handle_key` → calls `extract_selected()`.
- `extract_selected()` calls `parser.read_file(entry, &mut std::fs::File)` for
  the currently selected non-directory entry, writing to a file in the CWD.
- A one-row status bar at the bottom of `render_browser` shows the last result
  (green for success, red for error).
- Screen title updated to include `e: extract` hint.

New tests:
- `e_key_noop_without_parser` — pressing 'e' with no parser is a safe no-op
- `e_key_noop_on_directory_entry` — pressing 'e' on a dir entry does nothing

---

### A2 — Additional Carving Signatures

**`config/signatures.toml`** — 8 new entries (total: 18):

| Name | Extension | Header |
|---|---|---|
| SQLite Database | db | `53 51 4C 69 74 65 20 66 6F 72 6D 61 74 20 33 00` |
| WAV Audio (RIFF) | wav | `52 49 46 46` |
| AVI Video (RIFF) | avi | `52 49 46 46` |
| Matroska / MKV | mkv | `1A 45 DF A3` |
| FLAC Audio | flac | `66 4C 61 43` |
| Windows PE Executable | exe | `4D 5A` |
| VMDK Disk Image | vmdk | `4B 44 4D 56` |
| OGG Media | ogg | `4F 67 67 53` |

**`crates/ferrite-carver/src/lib.rs`** — updated `builtin_signatures_parse`
assertion from 10 → 18.

New tests in `crates/ferrite-tui/src/screens/carving.rs`:
- `signatures_include_sqlite`
- `signatures_include_flac`
- `signatures_include_mkv`

---

### A3 — exFAT and HFS+ Filesystem Detection

**`crates/ferrite-filesystem/src/lib.rs`**

- Added `FilesystemType::ExFat` and `FilesystemType::HfsPlus` enum variants.
- `Display` impl updated for both.
- `detect_filesystem()` now checks:
  - exFAT: OEM ID `"EXFAT   "` at bytes 3–10 of the boot sector.
  - HFS+: magic `0x482B` (or `0x4858` for HFSX) at offset 1024 (big-endian).
- `open_filesystem()` returns `UnknownFilesystem` for both (detect-only; no
  parser is implemented yet, but the UI can display the detected type).
- The ext4 and HFS+ checks now share a single `io::read_bytes(device, 1024, 60)`
  call for efficiency.

New tests:
- `detect_exfat_volume`
- `detect_hfsplus_volume`
- `detect_hfsx_volume`
- `open_exfat_returns_unknown_filesystem_error`

---

### A4 — SHA-256 Image Integrity Hash

**`Cargo.toml`** (workspace) — added `sha2 = "0.10"` to workspace dependencies.

**`crates/ferrite-tui/Cargo.toml`** — added `sha2 = { workspace = true }`.

**`crates/ferrite-tui/src/screens/imaging.rs`**

- `ImagingMsg::Done` now carries `Option<String>` (hex SHA-256 digest).
- Added `image_sha256: Option<String>` to `ImagingState`.
- After `ImagingEngine::run()` succeeds (in the background thread), the helper
  `compute_sha256(path)` streams the output file through `sha2::Sha256` and
  sends the digest with the Done message.
- The Statistics panel appends `SHA-256: <hex>` when the hash is present.

New test: `image_sha256_initially_none`

---

### B1 — Per-Read Timeout via Overlapped I/O (Windows)

**`crates/ferrite-blockdev/Cargo.toml`** — added `Win32_System_Threading`
feature to `windows-sys` for `CreateEventW`.

**`crates/ferrite-blockdev/src/error.rs`** — added:
```rust
Timeout { device: String, offset: u64 }
```

**`crates/ferrite-blockdev/src/windows.rs`** — full rewrite of I/O path:

- `WindowsBlockDevice` gains a `timeout_ms: u32` field (default 30 000 ms).
- New `open_with_timeout(path, timeout_ms)` constructor for callers that need
  custom timeouts.
- `open()` opens a temporary synchronous handle for geometry IOCTLs (`query_geometry`,
  `query_storage_property`), then opens the main read handle with
  `FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED`.
- `read_at()` now:
  1. Creates a one-shot manual-reset event (`CreateEventW`) per call → RAII
     `EventGuard` closes it on all exit paths.
  2. Issues `ReadFile` with a stack-allocated `OVERLAPPED` (offset + event).
  3. Calls `GetOverlappedResultEx` with `timeout_ms` — handles both synchronous
     and asynchronous completions uniformly.
  4. On `WAIT_TIMEOUT`: calls `CancelIo`, drains the pending I/O with a 5-second
     drain wait, returns `BlockDeviceError::Timeout`.
- `EnterEventGuard` struct defined at module level as RAII wrapper.

New test: `timeout_error_display` (in `error.rs`)

---

## Test Results

| Crate | Tests |
|---|---|
| ferrite-blockdev | 15 (+1) |
| ferrite-carver | 20 |
| ferrite-core | 2 |
| ferrite-filesystem | 22 (+4) |
| ferrite-imaging | 25 |
| ferrite-partition | 27 |
| ferrite-smart | 18 |
| ferrite-tui | 26 (+6) |
| **Total** | **155** |

All 155 tests pass.  `cargo clippy --workspace -- -D warnings` and
`cargo fmt --check` are both clean.

---

## Known Limitations (still open)

- `Carver::scan()` cancellation: background thread runs to completion.
- File extraction is synchronous (brief UI pause on large files).
- exFAT and HFS+ are detect-only; no directory browser or file extraction.
- `ImagingPhase::Retry { attempt, max }` displayed without counter values.
