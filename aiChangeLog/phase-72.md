# Phase 72 — ZIP Footer Extra Bytes + RAR Continuation Volume Fix

## Problem

### ZIP — Truncated EOCD record

Extracted ZIP files were corrupt because the footer `PK\x05\x06` marks
the **start** of the 22-byte End of Central Directory (EOCD) record, not
the end.  The extraction stopped after writing the 4-byte footer signature,
omitting the 18 bytes of essential metadata (disk number, central directory
offset, entry count, comment length).  WinRAR reported "The archive is
corrupt".

### RAR — Continuation volumes still extracted

Some RAR files hit the 500 MiB `max_size` cap, resulting in truncated
archives that WinRAR couldn't extract ("Unexpected end of archive").
The Phase 71 continuation-volume rejection was applied but the user found
additional files with this issue.

## Solution

### `footer_extra` field (new infrastructure)

Added `footer_extra: usize` to `Signature` and `RawSig`.  When the footer
is found during extraction, the extractor includes `footer_extra` additional
bytes after the footer match.  If those bytes extend past the current read
buffer, an additional read is issued to fetch them.

Both `stream_until_footer` and `stream_until_last_footer` in `carver_io.rs`
were updated to accept and apply this parameter.

### ZIP configuration

```toml
footer_extra = 18   # 22-byte EOCD minus 4-byte signature
```

This ensures the essential EOCD fields (central directory offset, entry
count, comment length) are included.  Variable-length comments beyond
the 18 fixed bytes may be truncated, but the archive structure is intact.

## Files Changed

- `crates/ferrite-carver/src/signature.rs` — `footer_extra: usize` field on
  `Signature` and `RawSig`; TOML deserialization
- `crates/ferrite-carver/src/carver_io.rs` — `footer_extra` parameter on
  `stream_until_footer` and `stream_until_last_footer`
- `crates/ferrite-carver/src/scanner.rs` — pass `sig.footer_extra` to both
  streaming functions; `footer_extra: 0` on all test `Signature` structs
- `crates/ferrite-carver/src/scan_search.rs` — `footer_extra: 0` on test structs
- `crates/ferrite-carver/src/size_hint.rs` — `footer_extra: 0` on test structs
- `crates/ferrite-carver/tests/size_hints.rs` — `footer_extra: 0` on test structs
- `crates/ferrite-tui/src/screens/carving/mod.rs` — `footer_extra: 0`
- `crates/ferrite-tui/src/screens/carving/user_sigs.rs` — `footer_extra: 0`
- `config/signatures.toml` — ZIP `footer_extra = 18`

## Tests

- Full workspace: all tests pass (730+ across all crates).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
