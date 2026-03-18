# Phase 41 — Grouped Collapsible Signature List (TUI)

## Problem

The signature panel showed all 43 file types as a flat list, making it hard to
quickly toggle an entire category (e.g., "all video formats") or focus on only
the types you care about.

## Solution

Replaced the flat `sig_list` with a two-level collapsible tree: 8 category
groups, each containing its member signatures. Groups start collapsed (8 rows
visible by default); the user expands any group with Enter.

### 8 Groups

| Group | Count | Extensions |
|-------|-------|------------|
| Images | 7 | jpg×2, png, gif, bmp, tif×2 |
| RAW Photos | 7 | arw, cr2, nef, rw2, raf, heic×2 |
| Video | 10 | mp4, mov, m4v, 3gp, avi, mkv, webm, wmv, flv, mpg |
| Audio | 4 | mp3, flac, wav, ogg |
| Documents | 6 | pdf, xml, html, rtf, vcf, ics |
| Office & Email | 3 | zip, ole, pst |
| Archives | 2 | rar, 7z |
| System | 4 | db, vmdk, evtx, exe |

### New Key Bindings (Signatures panel)

| Key | On group header | On individual sig |
|-----|----------------|-------------------|
| `Space` | Toggle all sigs in group on/off | Toggle that sig on/off |
| `Enter` | Expand / collapse the group | (noop) |

### Group Header Display

```
▶ Video (10/10)       ← collapsed, all enabled (white bold)
▼ Images (6/7)        ← expanded, partial (yellow bold)
  [✓] JPEG JFIF
  [ ] BMP
▶ Archives (0/2)      ← collapsed, all disabled (dark gray bold)
```

### State Changes

- Removed: `sig_list: Vec<SigEntry>` from `CarvingState`
- Added: `groups: Vec<SigGroup>` — the tree data
- Added: `cursor_rows: Vec<CursorRow>` — flat navigation index, rebuilt on expand/collapse
- `SigGroup { label, expanded, entries }` — new public struct
- `CursorRow::Group(gi)` / `CursorRow::Sig(gi, si)` — navigation enum
- `rebuild_cursor_rows()` — recomputes `cursor_rows` from current `groups` expand state; clamps `sig_sel` to stay in bounds

## Files Changed

- `crates/ferrite-tui/src/screens/carving/helpers.rs` — replaced `load_builtin_signatures()` with `load_builtin_sig_groups()` + `sig_group_label()` mapping
- `crates/ferrite-tui/src/screens/carving/mod.rs` — `SigGroup`, `CursorRow` types; updated `CarvingState`; `rebuild_cursor_rows()`; updated + expanded tests
- `crates/ferrite-tui/src/screens/carving/input.rs` — `move_selection` uses `cursor_rows.len()`; `toggle_signature` dispatches on Group vs Sig row; new `toggle_group_expand()`; `Enter` key bound; `start_scan` collects across groups
- `crates/ferrite-tui/src/screens/carving/render.rs` — `render_sig_panel` renders group headers with arrows and enabled counts, indented sig rows
- `aiChangeLog/phase-41.md` — this file

## Test Results

- 63 unit tests in ferrite-tui (was 59, +4 new)
- All workspace tests pass, clippy clean
