# Phase 87 — Damaged Drive Support

## Goal
Improve usability when working with critically damaged drives (SMART CRITICAL,
fatal I/O errors at sector 0).  Two complementary strategies:

1. **Image-first workflow** — let the user open a `.img` file as the source
   device so they can image first (using the Imaging tab's 5-pass engine) then
   carve from the image without stressing the dying drive further.
2. **Error-tolerant extraction** — when a bad sector is hit during file
   extraction, zero-fill that chunk and continue rather than aborting the
   carved file entirely.

## Changes

### `crates/ferrite-carver/src/carver_io.rs`
- Added `read_bytes_zeroed(device, offset, len) -> Vec<u8>`: calls
  `read_bytes_clamped`; on I/O error logs a `warn!` and returns a zero-filled
  vector of the requested length instead of propagating the error.
- `stream_bytes`, `stream_until_footer`, `stream_until_last_footer` now use
  `read_bytes_zeroed` instead of `read_bytes_clamped?`.  Bad sectors produce
  zero-filled gaps in the output file; the extraction no longer aborts.
- Note: scanner (`scan_impl`) already had its own per-chunk skip on error and
  is unaffected — scanning behavior is unchanged.
- Added 4 unit tests covering zero-fill on error and empty-past-EOF.

### `crates/ferrite-tui/src/screens/drive_select.rs`
- Added `image_input: Option<String>` and `image_error: Option<String>` to
  `DriveSelectState`.
- `f` key opens the image-open overlay (input mode).
- While the overlay is active, key events are routed to the input:
  - Char → appended to path
  - Backspace → remove last char
  - Enter → open via `FileBlockDevice::open()`, propagate as
    `Arc<dyn BlockDevice>` through the same device-selection path as physical
    drives; on error, set `image_error` and keep overlay open.
  - Esc → close overlay, clear error.
- `is_filtering()` now returns `true` when the overlay is active (prevents `q`
  quitting while a path is being typed).
- `render()` draws a floating centered popup (70 % wide, 5 rows) when the
  overlay is active, using `ratatui::widgets::Clear` to erase the background.
- Added `centered_popup()` helper.
- Title bar updated: added `f: open image` hint.
- Added 6 unit tests covering overlay open/close, char input, error handling.

## Test counts
- Before: 854 tests
- After:  860 tests (+6 new)

## Recommended workflow for critically damaged drives
1. **Imaging tab** → create a `.img` sidecar (5-pass ddrescue engine skips
   bad sectors, fills with zeros, resumes across reboots).
2. **Drives tab → `f`** → enter path to the `.img` file → Enter.
3. All tabs (Carving, Files, Artifacts, Text Scan, Hex, …) now operate on the
   image file — no further stress on the dying drive.
4. If carving directly from a live damaged drive: bad-sector gaps in carved
   files are now zero-filled instead of producing truncated/empty output.
