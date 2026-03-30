# Phase 116 - LNK / Prefetch Carved-File Naming

## Goal
After carving extracts a `.lnk` (Windows Shell Link) or `.pf` (Windows Prefetch)
file, parse the embedded target/executable name and rename the carved file from
an opaque offset-based name (`ferrite_lnk_12345678.lnk`) to a human-readable
one (`notepad.exe[r].lnk`).  The `[r]` suffix marks it as a heuristic rename
derived from the file content, not a confirmed filesystem entry.

## Files Changed

### `crates/ferrite-tui/src/screens/carving/extract.rs`

**New pure functions (testable):**

- `parse_lnk_target_name(data: &[u8]) -> Option<String>`
  - Validates the 76-byte LNK header magic (`0x4C000000`)
  - Reads `LinkFlags` at offset 0x14; returns `None` if `HasLinkInfo` (bit 1)
    is not set
  - Skips the optional `LinkTargetIDList` (2-byte size prefix at 0x4C)
  - Reads `LinkInfo`: navigates `LocalBasePathOffset` and `CommonPathSuffixOffset`
  - Concatenates `LocalBasePath + CommonPathSuffix`; returns the last path component

- `parse_pf_exe_name(data: &[u8]) -> Option<String>`
  - Checks `SCCA` magic at bytes 4..8
  - Reads `ExecutableName` at offset 0x10 (60 bytes, UTF-16LE, null-terminated)
  - Returns the name in lower-case with filesystem-safe characters only

**New integration functions:**

- `recovered_name_from_file(path, ext)` — reads first 4 KiB, dispatches to parser
- `try_recovered_rename(path, ext) -> String` — parses name, renames file (does
  nothing if parse fails, new name already exists, or rename errors)
- `read_ansi_nul(data)` — reads a NULL-terminated ANSI string slice

**Wired into extraction paths:**

Both `extract_selected` (single manual extraction) and `start_extraction_batch`
(auto/batch) now call `try_recovered_rename` after a successful extraction of
`lnk` or `pf` files, before reporting the result to the TUI channel.

## Tests (3 new)

| Test | Description |
|------|-------------|
| `lnk_name_extracted_from_link_info` | Minimal LNK blob with `LocalBasePath = C:\Windows\System32\notepad.exe`; asserts `parse_lnk_target_name` returns `"notepad.exe"` |
| `pf_name_extracted_from_header` | Vista-format PF blob with `ExecutableName = "NOTEPAD.EXE"`; asserts `parse_pf_exe_name` returns `"notepad.exe"` |
| `fallback_kept_when_parse_fails` | Garbage data; asserts both parsers return `None` |

## Format Notes

**LNK (MS-SHLLINK):**
- Header magic: bytes 0..4 = `[0x4C, 0x00, 0x00, 0x00]`
- `LinkFlags` at 0x14: bit 0 = HasLinkTargetIDList, bit 1 = HasLinkInfo
- IDList at 0x4C: 2-byte size prefix; skip `2 + size` bytes to reach LinkInfo
- `LinkInfo.LocalBasePathOffset` at header+16; `CommonPathSuffixOffset` at header+24
- Both are NULL-terminated ANSI strings; full path = concatenation

**Prefetch (libscca / SCCA format):**
- Magic: bytes 4..8 = `b"SCCA"` (0x53 0x43 0x43 0x41)
- `ExecutableName`: 60 bytes at 0x10, UTF-16LE, up to 29 chars + null terminator
- Format identical across XP (0x11), Vista (0x17), Win8.1 (0x1A), Win10 (0x1E)
