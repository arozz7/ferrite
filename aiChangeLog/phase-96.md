# Phase 96 — WMV / TAR / MPG Size Hints

## Problem
Analysis of a 20 GB test carve (29.3 GiB output) revealed three file types
extracting far beyond their actual size due to missing size hints:

| Type | Files | Over-extracted total | Root cause |
|------|-------|----------------------|------------|
| WMV  | 6     | 16.52 GiB            | No hint — falls back to 4 GiB max_size |
| TAR  | 2     | 1.44 GiB             | No hint — falls back to 2 GiB max_size |
| MPG  | 1     | 1.37 GiB             | No hint — falls back to 4 GiB max_size |

## Fixes

### SizeHint::Asf — WMV / ASF header walker
**New file:** `crates/ferrite-carver/src/size_hint/asf.rs`

- Reads the 30-byte ASF Header Object at `file_offset`; verifies GUID
  `30 26 B2 75 8E 66 CF 11 A6 D9 00 AA 00 62 CE 6C`
- Walks sub-objects (GUID + u64 LE size, 24 bytes each) up to `num_headers`
  sub-objects or 256 iterations
- On finding the File Properties Object (GUID `A1 DC AB 8C 47 A9 CF 11 …`),
  reads the exact `File Size` (u64 LE at FPO offset 40)
- Returns `None` when file_size == 0 (ASF "unknown") — falls back to max_size

**4 unit tests:** valid 512 MiB file, zero file size, wrong GUID, nonzero offset.

---

### SizeHint::Tar — POSIX ustar block walker
**New file:** `crates/ferrite-carver/src/size_hint/tar.rs`

- Walks 512-byte TAR blocks from the true archive start (`file_offset`,
  already adjusted by `header_offset = 257`)
- Parses each header's size field (12 bytes ASCII octal at header offset 124)
  via `parse_octal()` — handles null-terminated and space-padded fields
- Advances past header block + `ceil(size / 512)` data blocks per entry
- Stops at two consecutive zero blocks (end-of-archive marker)
- Safety cap: max 200 000 entries; returns partial size on read failure

**5 unit tests:** single file, multiple files, zero-size file, valid octal,
invalid octal digit.

---

### SizeHint::MpegPs — MPEG-2 PS pack-header walker
**New file:** `crates/ferrite-carver/src/size_hint/mpeg_ps.rs`

- Scans forward in 512 KiB chunks looking for pack-start codes (`00 00 01 BA`)
  and the Program End code (`00 00 01 B9`)
- Returns exact size (PSEND offset + 4) when the Program End code is found
- Stops when no pack header appears within a 2 MiB gap (sync lost); returns
  `last_pack_offset + 2048` (one typical DVD pack) as an estimate
- Hard cap: 4 GiB (same as `max_size`)

**4 unit tests:** stream with PSEND (exact size), stream without PSEND
(estimate), empty stream (None), PSEND-only stream.

---

## Modified files
| File | Change |
|------|--------|
| `crates/ferrite-carver/src/signature.rs` | Added `SizeHint::Asf`, `SizeHint::Tar`, `SizeHint::MpegPs` variants; `kind_name()` arms; TOML parser arms |
| `crates/ferrite-carver/src/size_hint/mod.rs` | Added `mod asf`, `mod tar`, `mod mpeg_ps`; dispatch arms |
| `config/signatures.toml` | Added `size_hint_kind = "asf"` to WMV; `size_hint_kind = "mpeg_ps"` to MPG; `size_hint_kind = "tar"` to TAR |
