# Phase 95 — Quick Recover Scroll Fix

## Problem
Arrow keys (↑/↓) and Page Up/Down did not scroll the file list in the Quick
Recover tab (Tab 7). `move_selection()` updated `self.selected` but never
touched `self.scroll`, so the viewport was always pinned to the top. Page Up
and Page Down were not handled at all.

## Fix

### Modified file
`crates/ferrite-tui/src/screens/quick_recover.rs`

| Change | Detail |
|--------|--------|
| Added `visible_rows: usize` field | Stores the number of data rows that fit in the rendered area; updated each frame from `chunks[list_idx].height - 1` (subtracts header row) |
| `move_selection(delta)` | Now uses `delta.unsigned_abs()` as step size (enables page-sized jumps); calls `clamp_scroll()` after every move |
| New `clamp_scroll(list_len)` | Scrolls viewport up when cursor goes above it; scrolls down when cursor goes below `scroll + visible_rows`; clamps `selected` to valid range |
| Added key handlers | `PageUp`, `PageDown`, `Home`, `End` |
| Resize resilience | `clamp_scroll()` also called during `render_content()` so terminal resize can't leave the viewport in a broken state |

## Keys added
| Key | Action |
|-----|--------|
| `PageDown` | Move down one full page |
| `PageUp`   | Move up one full page |
| `Home`     | Jump to first entry |
| `End`      | Jump to last entry |
