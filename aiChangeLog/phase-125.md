# Phase 125 — Drive Profile View (Tab 4 — File Browser)

## Summary

Adds a **Drive Profile** sub-view to the File Browser tab (Tab 4), accessible via the
`p` key once a filesystem has been opened.  The profile gives an instant breakdown of
every file on the volume by category (images, audio, video, documents, etc.), split by
active vs deleted, with byte totals and a proportional Unicode bar chart.  A heuristic
label infers the likely purpose of the drive (e.g. "Camera / Photo Storage", "Windows
System Drive") from the category distribution.

## New Module — `ferrite-filesystem::profile`

**File:** `crates/ferrite-filesystem/src/profile.rs`

| Symbol | Kind | Description |
|--------|------|-------------|
| `FileCategory` | enum | 9 variants: Images, RawPhoto, Video, Audio, Archive, Document, System, Database, Other |
| `ext_to_category(ext)` | fn | Maps ~80 lowercase extensions to a `FileCategory` |
| `CategoryStats` | struct | `active_count`, `deleted_count`, `active_bytes`, `deleted_bytes`; `total_count()` + `total_bytes()` |
| `DriveProfile` | struct | `HashMap<FileCategory, CategoryStats>` + totals + `fs_type` |
| `build_profile(files, fs_type)` | fn | Pure aggregation over `&[FileEntry]`; skips dirs; lowercases extensions |
| `infer_drive_type(profile)` | fn | 8 ratio-based heuristic labels |

All symbols re-exported from `ferrite-filesystem::` top level.

**13 unit tests** — extension mapping, aggregation counts, active/deleted split, case
insensitivity, directory skipping, empty input, inference cases.

## TUI Changes — `file_browser.rs`

### New state fields on `FileBrowserState`

| Field | Type | Purpose |
|-------|------|---------|
| `show_profile` | `bool` | Toggle for the profile sub-view |
| `profile` | `Option<DriveProfile>` | Completed profile; `None` until build finishes |
| `profile_rx` | `Option<Receiver<Result<DriveProfile, String>>>` | Background build channel |
| `profile_building` | `bool` | Spinner guard while thread is running |
| `profile_error` | `Option<String>` | Last build error, if any |

All fields reset in `set_device()`.

### `p` key behaviour

- Only active after a filesystem has been opened (`parser.is_some()`).
- First press: sets `show_profile = true`, spawns background thread
  (`parser.enumerate_files()` → `build_profile()`).
- Subsequent presses: instant toggle (profile is cached for the session).
- After a build error, pressing `p` again clears the error and retries.

### Error feedback (three-layer)

1. **Red border** on the entire File Browser panel.
2. **Status bar** (bottom of browser view): `Profile error: <reason>  [p] to retry` — visible without opening the profile view.
3. **Profile sub-view**: Red error text with retry instruction.

### `render_profile()` layout

```
 NTFS — 12,483 active  |  1,247 deleted  |  74.6 GB total
 Drive type: Personal Workstation / Office PC

 Category     Active   Deleted    Total     Size     %   Distribution
 Documents     4,521       342    4,863   12.4 GB  38%  ██████████░░░░░░
 Images        3,102       891    3,993    8.2 GB  32%  ████████░░░░░░░░
 System        2,344        87    2,431    1.8 GB  19%  ███████░░░░░░░░░
 Audio         1,829       102    1,931    3.1 GB  15%  █████░░░░░░░░░░░
 Video           234        12      246   45.1 GB   2%  ░░░░░░░░░░░░░░░░
 Archives        312        91      403    4.2 GB   3%  ░░░░░░░░░░░░░░░░
 Other           141        22      163    0.8 GB   1%  ░░░░░░░░░░░░░░░░

 [p] back to browser
```

Rows sorted by total file count descending.  Bar is 16 chars wide (`█` filled, `░` empty).
"Scanning filesystem…" shown in yellow while the background thread runs.

### Key binding added

`p` — Toggle Drive Profile view (shown in the browser title bar hint).

## Files Changed

| File | Change |
|------|--------|
| `crates/ferrite-filesystem/src/profile.rs` | **New** — all aggregation + inference logic |
| `crates/ferrite-filesystem/src/lib.rs` | `pub mod profile;` + re-exports |
| `crates/ferrite-tui/src/screens/file_browser.rs` | Profile state, `p` handler, background build, `render_profile()`, error feedback |
| `docs/user-manual.md` | Section 10 updated — Drive Profile sub-section added |
| `aiChangeLog/phase-125.md` | This file |

## Test Results

- `cargo test --workspace`: **1,149 passed, 0 failed** (13 new in `ferrite-filesystem::profile`)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all`: formatted
