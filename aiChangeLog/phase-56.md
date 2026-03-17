# Phase 56 — Custom User Signatures

## Summary
Users can now define, edit, delete, and import their own carving signatures via
a TUI overlay panel on the Carving screen (`u` key).  Custom signatures are
persisted to `ferrite-user-signatures.toml` (same TOML schema as
`config/signatures.toml`) and appear as a dedicated **Custom** group in the
signature panel.

## Changes

### `crates/ferrite-carver/src/lib.rs`
- Exported `parse_hex_pattern` so TUI-side code can parse wildcard hex headers
  without re-implementing the logic.

### `crates/ferrite-tui/Cargo.toml`
- Added `toml = { workspace = true }` dependency (already a workspace dep,
  just not listed for this crate — needed if TOML serialisation is extended).

### New: `crates/ferrite-tui/src/screens/carving/user_sigs.rs`
- `UserSigDef` — raw-string representation of a user signature (name,
  extension, header hex, footer hex, max_size).
- `UserSigDef::to_signature() -> Result<Signature, String>` — converts to a
  carver `Signature`; returns a targeted error message on validation failure.
- `validate_header(hex) -> bool` — space-separated `??`-wildcard hex check.
- `validate_footer(hex) -> bool` — space-separated exact-hex check; empty OK.
- `load_user_sigs(path) -> Vec<UserSigDef>` — reads and parses the TOML file;
  returns `[]` on missing/empty/parse-error.
- `save_user_sigs(path, sigs) -> io::Result<()>` — serialises to `[[signature]]`
  TOML without the `toml` crate (hand-written, no extra serde derive needed).
- 11 unit tests (validate, to_signature round-trip, save/load round-trip).

### New: `crates/ferrite-tui/src/screens/carving/user_sig_panel.rs`
- `render_user_panel` — centred popup (70 % × 65 %) listing custom signatures
  with selection highlight, delete-confirm prompt, import-path input, and key-
  hint footer.
- `render_user_form` — smaller centred dialog (60 % × 55 %) with 5 labelled
  text fields, cursor block, error line, and help footer.
- `handle_form_key(form, code, mods) -> FormAction` — pure key handler for the
  form; returns `None | Submit | Cancel`.  No borrow conflicts with `CarvingState`.
- `FormAction` enum.
- 13 unit tests for the form key handler.
- `centered_rect` geometry helper (local, not exported).

### `crates/ferrite-tui/src/screens/carving/helpers.rs`
- Added `use super::user_sigs::UserSigDef`.
- Added `build_user_sig_group(sigs: &[UserSigDef]) -> Option<SigGroup>` — builds
  the "Custom" group from in-memory sigs; `None` when list is empty.
- `GROUP_ORDER` comment updated to note "Custom" is appended dynamically.

### `crates/ferrite-tui/src/screens/carving/mod.rs`
- New types: `FormMode { Add, Edit(usize) }` and `UserSigForm` struct (5 text
  fields + mode + focused field + error).
- New `CarvingState` fields:
  - `user_sig_path: String` (default `"./ferrite-user-signatures.toml"`)
  - `user_sigs: Vec<UserSigDef>` — in-memory source of truth
  - `show_user_panel: bool`
  - `user_panel_sel: usize`
  - `user_sig_form: Option<UserSigForm>`
  - `user_confirm_delete: bool`
  - `user_import_path: String`
  - `editing_import: bool`
- `CarvingState::new()` loads user sigs from disk and appends the Custom group
  if any are present.
- `is_editing()` extended to return `true` while `show_user_panel` is active
  (prevents `q` quitting while the panel is open).
- New methods: `refresh_custom_group`, `submit_user_form`, `do_import`.
- New module declarations: `mod user_sig_panel`, `mod user_sigs`.

### `crates/ferrite-tui/src/screens/carving/input.rs`
- Routing priority at top of `handle_key`:
  1. Import-path text input (`editing_import`)
  2. Form key handler (delegates to `handle_form_key`, dispatches Submit/Cancel)
  3. Panel list keys: `Esc` close, `↑/↓` navigate, `a` add, `e` edit,
     `d` delete (+ `y/n` confirm), `i` import
- `u` key in main match → opens the user-signatures panel.

### `crates/ferrite-tui/src/screens/carving/render.rs`
- After `render_hits_panel`, overlays `render_user_panel` (and `render_user_form`
  if form active) when `show_user_panel` is set.
- Title bar updated: `u: custom sigs` hint added.

## Test Count
- Before: 458
- After:  482  (+24)
- All passing, `cargo clippy --workspace -- -D warnings` clean.

## UX Summary
| Key | Context | Action |
|-----|---------|--------|
| `u` | Carving screen | Open Custom Signatures panel |
| `Esc` | Panel list | Close panel |
| `↑` / `↓` | Panel list | Navigate entries |
| `a` | Panel list | Open Add form |
| `e` | Panel list (non-empty) | Open Edit form pre-filled |
| `d` | Panel list (non-empty) | Prompt delete confirm |
| `y` | Delete confirm | Confirm delete + save |
| any other | Delete confirm | Cancel |
| `i` | Panel list | Open import-path prompt |
| `Enter` | Import prompt | Load + merge from path |
| `Esc` | Import prompt | Cancel |
| `Tab` / `Shift+Tab` | Form | Move between fields |
| `Enter` (last field) | Form | Validate + save |
| `Ctrl+S` | Form (any field) | Validate + save |
| `Esc` | Form | Discard changes |

## File Locations
| File | Role |
|------|------|
| `config/signatures.toml` | Built-in signatures (unchanged) |
| `./ferrite-user-signatures.toml` | User signatures (created on first save) |
| `crates/ferrite-tui/src/screens/carving/user_sigs.rs` | Data + persistence |
| `crates/ferrite-tui/src/screens/carving/user_sig_panel.rs` | Render + form keys |
