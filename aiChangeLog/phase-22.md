# Phase 22 — Hex Viewer Overhaul

## Summary
The hex viewer was a minimal sector browser: LBA-only navigation, a 512-byte
hard cap that silently dropped data on 4096-byte sector devices, no way to
reach a carve hit's byte offset, no context about what you were looking at, and
no fast navigation.  Phase 22 rebuilds it into a useful forensic tool.

## Changes

### `crates/ferrite-tui/src/screens/hex_viewer.rs` — complete overhaul

#### Input modes
Replaced the single `editing: bool` / `lba_input: String` pair with an
`EditMode { None, Lba, Offset }` enum and a shared `input: String` buffer.
- `g` → LBA mode (decimal, as before)
- `b` → byte-offset mode: accepts decimal or `0x`-prefixed hex (e.g. `0x1a0000`)

Added `pub fn is_editing() -> bool` (replaces the old `pub editing` field).

#### Byte-offset jump + deep-link highlight
`pub fn jump_to_byte_offset(offset: u64)` converts the offset to an LBA
(`offset / sector_size`) and records the byte position within the sector
(`offset % sector_size`) in `highlight_byte: Option<usize>`.  During rendering,
that byte is shown with a yellow background in both the hex and ASCII columns,
and a "► landed at byte +0x… (absolute 0x…)" hint line is displayed below the
header.

#### Full-sector display
Removed the `data.len().min(512)` cap.  The display now renders every byte of
the sector (typically 512 B or 4096 B) in the classic 16-bytes-per-row layout.

#### Navigation
| Key | Action |
|-----|--------|
| ↑ / ↓ | ±1 sector |
| PgUp / PgDn | ±16 sectors |
| Home | sector 0 |
| End | last sector |
| g | jump to LBA |
| b | jump to byte offset |

All navigation clamps to `[0, last_lba]` and clears `highlight_byte`.

#### Per-byte colour coding
`byte_style()` colours each hex and ASCII cell:
- `0x00` — dark grey (null / unwritten)
- `0xFF` — red (erased flash / unformatted)
- Printable ASCII — green
- Other binary — white

The highlighted landing byte overrides with yellow background + black text.

#### Sector annotation — `detect_sector_type()`
Inspects the first bytes of each sector and appends a magenta label to the
header line, e.g. `[NTFS Volume Boot Record]` or `[JPEG Image]`.

Recognised structures:
MBR, Protective MBR (GPT disk), GPT Header, NTFS VBR, FAT32/FAT16/FAT12 boot
sector, ext2/3/4 superblock, SQLite database, OLE2 compound document
(DOC/XLS/PPT/PST), RIFF container (WAV/AVI), Windows PE/DOS executable, ZIP /
Office Open XML, RAR, 7-Zip, JPEG, PNG, PDF, GIF, OGG, FLAC, MP3 (ID3),
Matroska/MKV, Windows Event Log (EVTX), Outlook PST/OST.

MBR detection is restricted to LBA 0 to avoid false positives on data sectors
with the same boot signature.

#### New tests (11)
`g_key_enters_edit_mode`, `b_key_enters_offset_edit_mode`,
`jump_to_byte_offset_sets_lba_and_highlight`,
`page_up_down_moves_sixteen_sectors`, `home_end_navigation`,
`detect_sector_type_mbr`, `detect_sector_type_gpt_protective_mbr`,
`detect_sector_type_ntfs`, `detect_sector_type_jpeg`,
`detect_sector_type_png`, `detect_sector_type_zip`,
`detect_sector_type_none_for_zeros`

---

### `crates/ferrite-tui/src/screens/carving.rs`
Added `pub fn selected_hit_offset() -> Option<u64>`: returns the byte offset of
the currently selected hit when focus is on the Hits panel; `None` otherwise.

### `crates/ferrite-tui/src/app.rs`
- Screen 5 (carving) key handler: `h` key calls `carving.selected_hit_offset()`;
  if a hit is selected, calls `hex_viewer.jump_to_byte_offset(offset)` and
  switches to screen 6 — one keystroke from carve hit to hex view.
- `is_editing` check: `self.hex_viewer.editing` → `self.hex_viewer.is_editing()`
- Help line for screen 5: added `h: view in hex`
- Help line for screen 6: updated to document all new keys

## Files Modified
- `crates/ferrite-tui/src/screens/hex_viewer.rs`
- `crates/ferrite-tui/src/screens/carving.rs`
- `crates/ferrite-tui/src/app.rs`
- `aiChangeLog/phase-22.md` (this file)

## Test Results
- `cargo test --workspace` — 224 tests pass, 0 failures (11 new hex viewer tests,
  1 existing test renamed from `g_key_enters_edit_mode` to match new API)
- `cargo clippy --workspace -- -D warnings` — clean
