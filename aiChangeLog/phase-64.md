# Phase 64 — Heuristic Text Block Scanner

## Summary

New crate `ferrite-textcarver` + TUI Tab 9 ("Text Scan").

Scans raw device data for contiguous text regions, classifies them by content
type (PHP, Script, JSON, YAML, HTML/XML, SQL, C, Markdown, Generic), and
exports each as a named file under a user-chosen output directory.

**Results are variable quality** — no filename recovery, possible merged
fragments, binary false positives.  Documented in consent dialog.

---

## New Crate: `crates/ferrite-textcarver/`

| File | Purpose |
|------|---------|
| `src/scanner.rs` | Data model: `TextKind`, `TextBlock`, `TextScanConfig`, `TextScanProgress`, `TextScanMsg` |
| `src/classifier.rs` | Priority-ordered heuristic classifier → `(TextKind, confidence, extension)` |
| `src/engine.rs` | Gap-tolerant sliding-window scanner; dedup via `DefaultHasher` |
| `src/export.rs` | `write_files(dir, blocks)` — names files `text_<8-hex>.ext` |
| `src/lib.rs` | Public re-exports |

### Classifier priority order
1. `<?php` + whitespace → Php (99%)
2. `#!/` → Script; sub-classified to py/rb/pl/js/sh (97%)
3. `<?xml` → Markup (99%)
4. `<!DOCTYPE`/`<html` → Markup (95%)
5. `{`/`[` + `":` ≥ 2× → Json (85%)
6. `---\n` frontmatter → Yaml (90%)
7. ≥3 distinct SQL keywords → Sql (70%)
8. ≥3 C/C++ keywords → CSource (65%)
9. ≥2 Markdown signals → Markdown (65%)
10. Fallback → Generic (50%)

### Engine algorithm
- Aligned 1 MiB chunks with 4 KiB overlap buffer
- Text-like: ASCII printable + UTF-8 lead/continuation bytes
- Block start: 3 consecutive text-like bytes
- Block end: `gap_run > gap_tolerance` (default 8) or `length == max_block_bytes` (1 MiB)
- Quality gate: printable% ≥ 80% (default)
- Dedup: `HashSet<u64>` of `DefaultHasher` digests

---

## New TUI Screen: `crates/ferrite-tui/src/screens/text_scan/`

| File | Purpose |
|------|---------|
| `mod.rs` | `TextScanState`, `ScanStatus`, tick/start/cancel/export logic + 6 unit tests |
| `input.rs` | Key handling: s/c/e/o, consent dialog, dir editing, ↑/↓/PgUp/PgDn, 0–8 filter |
| `render.rs` | 4-row layout, consent overlay, colored kind/quality columns |

**Status enum:** Idle / Running / Done / Cancelled / Error

**Keys:** `s` scan, `c` cancel, `e` export, `o` output dir, `0`–`8` filter by kind

---

## Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Added `ferrite-textcarver` member + workspace dep |
| `crates/ferrite-tui/Cargo.toml` | Added `ferrite-textcarver` dependency |
| `crates/ferrite-tui/src/screens/mod.rs` | Added `pub mod text_scan` |
| `crates/ferrite-tui/src/app.rs` | `SCREEN_NAMES` 9→10; `App` struct, `new()`, `tick()`, `handle_key()`, `render()`, device propagation (both paths), `is_editing()` guard, help line all updated; `screen_count_matches_names` test 9→10 |

---

## Test Results

- **658 total tests** — all passing (up from 629; 29 new)
- `cargo clippy --workspace -- -D warnings` — clean
