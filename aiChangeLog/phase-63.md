# Phase 63 — Code File Signatures (PHP + Shebang)  (97 → 99 signatures)

## Summary

Added 2 new signatures for code/script files, bringing the total from 97 to 99.

## New Signatures

| Name | Extension | Magic | Notes |
|------|-----------|-------|-------|
| PHP Script | `.php` | `3C 3F 70 68 70` (`<?php`) | Byte @5 must be whitespace |
| Shell / Script File (Shebang) | `.sh` | `23 21` (`#!`) | Byte @2 must be `/` |

**Excluded by design:** `.bat`/`.cmd` files starting with `@echo` — the opener is too fragile for reliable carving.

**Not implemented:** Dynamic extension classification for shebang scripts (python→.py, ruby→.rb, etc.).  The current architecture uses a fixed `extension` field in `Signature`; all shebang scripts are carved as `.sh`.  Content-based reclassification would require a post-extraction step not yet present.

## Files Changed

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/pre_validate.rs` | Added `Php` and `Shebang` variants to `PreValidate` enum; `kind_name()`, `from_kind()`, `is_valid()` dispatch; `validate_php()` and `validate_shebang()` functions; 9 new unit tests |
| `config/signatures.toml` | 2 new `[[signature]]` entries (PHP, Shebang) |
| `crates/ferrite-carver/src/lib.rs` | Assertion updated: 97 → 99 |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | `sig_group_label`: added `"php" | "sh"` → `"Documents"` |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `groups_cover_all_signatures` assertion: 97 → 99 |

## Validator Logic

**PHP (`validate_php`):**
After the 5-byte `<?php` magic, byte @5 must be one of `' '`, `'\t'`, `'\r'`, `'\n'`.
Rejects obfuscated forms like `<?phpinfo()` which embed the function call directly against the tag.

**Shebang (`validate_shebang`):**
After the 2-byte `#!` magic, byte @2 must be `'/'`.
Rejects false positives from binary data; all valid interpreter paths are absolute (e.g. `/bin/sh`, `/usr/bin/env`).

## Test Results

- **629 total tests** — all passing (up from 620)
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — clean
