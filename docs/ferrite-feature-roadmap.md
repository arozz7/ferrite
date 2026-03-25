# Ferrite — Comprehensive Feature Roadmap
**Reviewed as of Phase 101 (115 signatures) — Status updated 2026-03-24**
*Senior Data Recovery & Digital Forensics Perspective*

---

## Executive Summary

Ferrite has completed all planned phases through Phase 101. As of this update:

- **115 signatures** across 10 format categories (up from 43 at initial audit)
- **All workflow features delivered:** SHA-256 hash, thermal guard, write-blocker
  verification, Quick Deleted-File Recovery, custom user signatures, forensic artifact
  scanning, heuristic text block scanner, non-zero-offset scan infrastructure
- **10 TUI tabs** — Drives / Health / Imaging / Partitions / Files / Carving / Hex /
  Quick Recover / Artifacts / Text Scan
- **987 unit tests**, clippy-clean, `cargo fmt --check` passing

**All planned phases complete.** Phase 58 (exFAT + APFS MVP) was delivered as:
- **Phase 58a** — `ExFatParser`: full read-only exFAT `FilesystemParser` (668 tests)
- **Phase 58b** — `ApfsParser`: APFS container → omap → volume → FS B-tree walk (677 tests)

The phases below are documented for historical reference and are ordered by **risk and
recovery impact**, not by complexity.

---

## Current State Audit

### Signatures Implemented (99 total — Phase 63)

| Category | Formats | Count |
|---|---|---|
| Images | JPEG×2, PNG, GIF, BMP, TIFF×2, WebP, PSD, ICO | 10 |
| RAW Photos | ARW, CR2, NEF, RW2, RAF, HEIC×2, ORF, PEF, CR3, SR2, DCR, CRW, MRW, X3F | 15 |
| Video | MP4, MOV, M4V, 3GP, AVI, MKV, WebM, WMV, FLV, MPG, RM, SWF×3, TS, M2TS, WTV | 17 |
| Audio | MP3, WAV, FLAC, OGG, M4A, MIDI, AIFF, WavPack, APE, AU | 10 |
| Archives | ZIP, RAR, 7-Zip, GZip, XZ, BZip2, ISO, TAR | 8 |
| Documents | PDF, XML, HTML, RTF, VCF, ICS, EML, EPUB, ODT, CDR, TTF, WOFF, CHM, Blender, InDesign, PHP, Shebang | 17 |
| Office & Email | ZIP-Office (OOXML), OLE2 (legacy), PST, MSG | 4 |
| System / Exec | SQLite, EVTX, EXE, ELF, VMDK, REGF, VHD, VHDX, QCOW2, Mach-O, KDBX, KDB, E01, PCAP×2, DMP, plist, LUKS, DICOM | 18 |

### Engineering Features Status

| Feature | Status |
|---|---|
| Five-pass imaging engine (ddrescue-compatible) | ✅ Done |
| GNU ddrescue mapfile (session resume) | ✅ Done |
| MBR / GPT partition parsing & recovery | ✅ Done |
| NTFS metadata (MFT + deleted file listing) | ✅ Done |
| FAT32 metadata | ✅ Done |
| ext4 metadata (direct + indirect + **extents tree**) | ✅ Done — Phase 44 |
| S.M.A.R.T. diagnostics (smartctl wrapper) | ✅ Done |
| Grouped collapsible TUI signature panel | ✅ Done |
| Filesystem-assisted recovery (folder structure) | ✅ Done |
| Session state (scan_window_start / resume %) | ✅ Done (Phase 42) |
| Per-read timeout (overlapped I/O + `CancelIo`) | ✅ Done — Phase 43 |
| File browser → `e` key extract file to disk | ✅ Done — Phase 45 |
| exFAT detection (`EXFAT   ` at sector offset 3) | ✅ Done — Phase 50 |
| HFS+ / HFSX detection (magic 0x482B / 0x4858) | ✅ Done — Phase 50 |
| Read-rate monitoring (rolling `read_rate_bps`) | ✅ Done — Phase 47 partial |
| Raw sector hex viewer (TUI screen) | ✅ Done — Phase 48 |
| LBA range selection (`start_lba` / `end_lba`) | ✅ Done — Phase 49 |
| S.M.A.R.T. bad LBA → mapfile pre-population | ✅ Done — Phase 51 partial |
| Recovery report export (`generate_report`) | ✅ Done — Phase 54 |
| Drive temperature guard during imaging | ✅ Done — Phase 51b (`ThermalGuard`, RAII, configurable thresholds) |
| SHA256 image integrity hash | ✅ Done — Phase 47b (`.sha256` sidecar, amber TUI warning on resume) |
| Low read-rate alert (threshold + TUI warning) | ✅ Done — Phase 47c (amber bar + `[⚠ LOW RATE]` when < 5 MB/s) |
| Write-blocker verification | ✅ Done — Phase 53 (`write_blocker::check()`, pre-flight in `set_device()`) |
| Quick Deleted-File Recovery mode | ✅ Done — Phase 45b (Tab 7; RecoveryChance scoring; High/Med/Low) |
| Carve hit integrity validation | ✅ Done — Phase 55 (`CarveQuality` enum; `post_validate::validate_extracted()`) |
| Duplicate hit suppression (content hash) | ✅ Done — Phase 55 (4 KiB fingerprint `HashSet`; `[DUP]` tag) |
| Custom user-defined signatures (TUI) | ✅ Done — Phase 56 (`user_sigs.rs`; `u` key overlay; "Custom" group) |
| Forensic artifact scanning (email, URLs, CC#) | ✅ Done — Phase 57 (`ferrite-artifact`; Tab 8; 6 scanners; CSV export) |
| Heuristic text block scanner | ✅ Done — Phase 64 (`ferrite-textcarver`; Tab 9; 9 TextKind variants) |
| Non-zero-offset scan (ISO, DICOM, TAR) | ✅ Done — Phase 62 (`header_offset: u64` on `Signature`; 94 → 97 sigs) |
| PhotoRec Tier A batch (CR3/SR2/EPUB/ODT/MSG/WavPack/CDR/SWF×3/DCR) | ✅ Done — Phase 59 (62 → 73 sigs) |
| PhotoRec Tier B batch 1 (CRW/MRW/KDBX/KDB/E01/PCAP×2/DMP/plist/TS/M2TS/LUKS/X3F) | ✅ Done — Phase 60 (73 → 86 sigs) |
| PhotoRec Tier B batch 2 (APE/AU/TTF/WOFF/CHM/Blender/InDesign/WTV) | ✅ Done — Phase 61 (86 → 94 sigs) |
| Code file signatures (PHP, shebang) | ✅ Done — Phase 63 (97 → 99 sigs) |

---

## Signature Gap Analysis
*Derived from systematic comparison against PhotoRec's full format list at
`cgsecurity.org/wiki/File_Formats_Recovered_By_PhotoRec` (retrieved 2026-03-17).*

### Already Planned (Phases 46 + 52)

WebP, AAC/M4A, GZip, EML, ELF, REGF, PSD/PSB, VHD, VHDX, QCOW2,
MIDI, AIFF, XZ, BZip2, RealMedia, ICO, Olympus ORF, Pentax PEF, Mach-O — **19 formats**

### Newly Identified Gaps (from PhotoRec comparison)

#### Tier A — Quick Wins: new format, existing infrastructure already handles it

These add a new signature + pre-validator but require zero scanner infrastructure changes.

| Format | Extension | Magic Header (hex) | Existing Mechanism | Recovery Value |
|---|---|---|---|---|
| **Canon CR3** | `cr3` | `?? ?? ?? ?? 66 74 79 70 63 72 78 20` (ftyp `crx `) | ISOBMFF box walker (`size_hint_kind = "mp4"`) | **High** — modern Canon cameras (R5, R6, 90D) |
| **Sony SR2** | `sr2` | `49 49 2A 00 08 00 00 00` | TIFF IFD walker (`size_hint_kind = "tiff"`); validate "SR2" or tag 0x7200 | **High** — Sony Alpha 7 series RAW |
| **Kodak DCR** | `dcr` | `49 49 2A 00` (TIFF LE) | TIFF IFD walker; validate Kodak Make tag | **Medium** — legacy Kodak cameras |
| **EPUB e-Book** | `epub` | `50 4B 03 04` (ZIP) | ZIP inner validator; peek first entry for `application/epub+zip` | **High** — e-books, digital publications |
| **OpenDocument (ODT/ODS/ODP)** | `odt` | `50 4B 03 04` (ZIP) | ZIP inner validator; peek for `application/vnd.oasis.opendocument.*` | **High** — LibreOffice/OpenOffice documents |
| **Outlook MSG** | `msg` | `D0 CF 11 E0 A1 B1 1A E1` (OLE2) | OLE2 validator; add branch checking `__substg1.0` stream name | **High** — individual Outlook email messages |
| **WavPack** | `wv` | `77 76 70 6B` ("wvpk") | New standalone — 4-byte magic, validate block_samples @4 > 0 | **Medium** — lossless audio |
| **CorelDRAW** | `cdr` | `52 49 46 46 ?? ?? ?? ?? 43 44 52 56` (RIFF `CDRV`) | RIFF size-hint (same as WAV/AVI); validate subtype "CDRV"/"CDRX"/"CDR6" | **Medium** — design files |
| **Shockwave Flash** | `swf` | `46 57 53` (FWS) / `43 57 53` (CWS) / `5A 57 53` (ZWS) | New standalone — 3 magic variants; validate version byte @3 in [1,45] | **Medium** — legacy web content |

#### Tier B — New Infrastructure Required: standalone validator, new magic

| Format | Extension | Magic Header (hex) | Pre-validate Logic | Recovery Value |
|---|---|---|---|---|
| **Canon CRW** | `crw` | `49 49 1A 00 00 00 48 45 41 50 43 43 44 52` | CIFF container; validate "HEAPCCDR" at offset 6 | **High** — older Canon cameras (pre-2004) |
| **Minolta MRW** | `mrw` | `00 4D 52 4D` ("\0MRM") | Validate block size u32 BE @4; "PRD\0" or "TTW\0" at offset 8 | **High** — Minolta/Konica-Minolta RAW |
| **KeePass 2.x** | `kdbx` | `03 D9 A2 9A 67 FB 4B B5` | 8-byte magic; validate version u16 LE @8 | **High** — password manager vaults |
| **KeePass 1.x** | `kdb` | `03 D9 A2 9A 65 FB 4B B5` | 8-byte magic (differs at byte 7 vs KDBX) | **High** — password manager vaults |
| **EnCase Evidence** | `e01` | `45 56 46 09 0D 0A FF 00` ("EVF") | 8-byte magic + segment header type @8 == 0x01 | **High** — forensic imaging format |
| **Packet Capture** | `pcap` | `D4 C3 B2 A1` (LE) / `A1 B2 C3 D4` (BE) | Two-variant magic; validate magic_number + version_major @4 == 2 | **High** — network forensics |
| **Windows Dump** | `dmp` | `4D 44 4D 50 93 A7` (minidump "MDMP") / `50 41 47 45 44 55 36 34` (full dump) | Two-variant magic | **High** — crash analysis, forensics |
| **Apple Property List** | `plist` | `62 70 6C 69 73 74 30 30` ("bplist00") | 8-byte magic; validate trailer offset table (last 26 bytes) | **High** — macOS app data, iOS backups |
| **EMLX (Apple Mail)** | `emlx` | No binary magic — starts with ASCII decimal byte count | Validate first line is all digits + newline; check for `X-Apple-UUID:` header | **High** — Apple Mail messages |
| **MPEG-TS** | `ts` | `47` repeating every 188 bytes | Validate 0x47 at offsets 0, 188, 376 (3 consecutive sync bytes) | **High** — broadcast, dashcam footage |
| **M2TS (Blu-ray)** | `m2ts` | `?? ?? ?? ?? 47` — 4-byte timestamp + 0x47 sync, stride 192 bytes | Validate 0x47 at offsets 4, 196, 388 (192-byte stride) | **High** — Blu-ray, Sony camcorders |
| **KeePass / LUKS** | `luks` | `4C 55 4B 53 BA BE` ("LUKS\xBA\xBE") | Validate version u16 BE @6 in {1,2} | **Medium** — encrypted Linux disk containers |
| **Monkey's Audio** | `ape` | `4D 41 43 20` ("MAC ") | Validate file version u16 LE @6 in [3930, 4100] | **Medium** — lossless audio |
| **Sun AU Audio** | `au` | `2E 73 6E 64` (".snd") | Validate data_offset u32 BE @4 ≥ 24; encoding u32 BE @12 in [1–8, 23–27] | **Medium** — legacy Unix audio |
| **TrueType Font** | `ttf` | `00 01 00 00 00` | Validate numTables u16 BE @4 in [4, 50]; searchRange/entrySelector/rangeShift sane | **Medium** — font recovery |
| **Web Font** | `woff` | `77 4F 46 46` ("wOFF") | Validate flavor u32 BE @4 ∈ {0x00010000, 0x4F54544F}; length u32 BE @8 | **Medium** — web font recovery |
| **CHM Help** | `chm` | `49 54 53 46 03 00 00 00 60 00 00 00` ("ITSF") | 12-byte magic is fully deterministic | **Medium** — Windows help files, tutorials |
| **Blender Project** | `blend` | `42 4C 45 4E 44 45 52` ("BLENDER") + `-`/`_` + `v`/`V` | Validate pointer-size byte @7 ∈ {`-`,`_`}; endian byte @8 | **Medium** — 3D project files |
| **Adobe InDesign** | `indd` | `06 06 ED F5 D8 1D 46 E5 BD 31 EF E7 FE 74 B7 1D` | 16-byte GUID is globally unique | **Medium** — print/layout design files |
| **Parchive** | `par2` | `50 41 52 32 00 50 4B 54` ("PAR2\0PKT") | 8-byte magic + packet_length u64 LE @8 > 0 | **Medium** — recovery/repair sets |
| **Sigma X3F** | `x3f` | `46 4F 56 62` ("FOVb") | Validate version u32 LE @4 in known set {0x00020000..0x00030000} | **Medium** — Sigma/Foveon camera RAW |
| **Windows TV** | `wtv` | `B7 D8 00 20 37 49 DA 11 A6 4E 00 07 E9 5E AD 8D` | 16-byte GUID (same pattern as WMV/ASF detection) | **Medium** — Windows Media Center recordings |
| **DPX (Film)** | `dpx` | `53 44 50 58` ("SDPX") / `58 50 44 53` ("XPDS") | Two-variant endian magic | **Medium** — film post-production |
| **GIMP XCF** | `xcf` | `67 69 6D 70 20 78 63 66 20 76` ("gimp xcf v") | Validate version string @10 is "file" or 3 ASCII digits | **Low** — GIMP image projects |
| **JPEG 2000** | `jp2` | `00 00 00 0C 6A 50 20 20 0D 0A 87 0A` | 12-byte signature box; validate JP2 header box follows | **Low** — medical imaging, archival |
| **BitTorrent** | `torrent` | `64 38 3A 61 6E 6E 6F 75 6E 63 65` (bencoded "d8:announce") | Validate bencoded dict structure | **Low** — metadata only |

#### Tier C — Non-Zero Offset Magic (requires scanner infrastructure change)

These formats have their identifying signature at a non-zero disk offset and cannot be
detected by the current scanner which anchors on byte 0 of each block.

| Format | Extension | Magic Location | Notes |
|---|---|---|---|
| **ISO 9660** | `iso` | `43 44 30 30 31` ("CD001") at **byte 32769** | Primary Volume Descriptor sector 16 (sector 0 = system area). Requires offset-scan feature or a workaround scanning for the pattern within the first 40 KiB of a hit. |
| **DICOM Medical** | `dcm` | `44 49 43 4D` ("DICM") at **byte 128** | 128-byte preamble precedes the magic. Very high forensic value in medical contexts. Same infrastructure fix as ISO would cover this. |
| **TAR Archive** | `tar` | `75 73 74 61 72` ("ustar") at **byte 257** | POSIX header magic. PhotoRec handles this via a non-zero-offset scan. Requires the same infrastructure. |

**Infrastructure fix needed (Phase 60):** Add `header_offset: u64` to `Signature` struct
(default 0). In `scan_search.rs`, when `header_offset > 0`, treat the match as a
candidate at `(found_pos - header_offset)` and validate the full magic starting at
`found_pos`. This unlocks ISO, DICOM, and TAR in a single change.

---

## Implementation Phases

Phases continue from Phase 42. Each phase must pass:
```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

---

### Phase 43 — Per-Read Timeout (Critical Blocker)

**Risk:** Without this, a single bad sector can hang the imaging thread for 7–60 seconds
on a consumer HDD. On a drive with hundreds of bad sectors, this causes hours of extra
mechanical stress and can trigger total head failure.

**Changes:**
- `ferrite-blockdev/src/windows.rs` — reopen source device handle with
  `FILE_FLAG_OVERLAPPED`; replace synchronous `ReadFile` + static OVERLAPPED offset
  pattern with true async overlapped I/O: `ReadFile` → `GetOverlappedResultEx(timeout_ms)`
  → `CancelIoEx` on timeout
- `ferrite-blockdev/src/linux.rs` — use `pread` inside a thread with a join timeout, or
  `io_uring` with a timeout SQE on Linux 5.4+
- `ferrite-blockdev/src/error.rs` — add `BlockDeviceError::Timeout { lba: u64, timeout_ms: u32 }`
- `ferrite-core/src/config.rs` — add `read_timeout_ms: u32` (default 15 000) to
  `ImagingConfig`
- `ferrite-imaging/src/engine.rs` — handle `Timeout` variant: mark sector `BadSector`,
  log with `tracing::warn!`, continue rather than abort
- `ferrite-tui` — expose `read_timeout_ms` in imaging config screen

**Tests:**
- Mock `BlockDevice` that sleeps > timeout_ms → assert `Timeout` is returned
- Imaging engine advances past a timeout sector and continues

---

### Phase 44 — ext4 Extents Tree Support (Critical Blocker)

**Risk:** Since Linux kernel 2.6.23 (2007), every file written to ext4 uses the extents
tree by default (inode flag `0x80000`). Without extent support, Ferrite silently returns
corrupt data for virtually every file on a modern Linux volume.

**Changes:**
- `ferrite-filesystem/src/ext4.rs`:
  - Detect `EXT4_EXTENTS_FL` flag (`0x00080000`) in inode `i_flags`
  - Implement extent header (`ee_magic = 0xF30A`) parsing
  - Walk extent tree: handle leaf nodes (extent entries with `ee_start_hi/lo` +
    `ee_len`) and index nodes (index entries pointing to child tree blocks)
  - Collect all physical block ranges and concatenate for `read_file()`
  - Support the "inline data" flag (`0x10000000`) for tiny files stored in inode space

**Tests:**
- Unit test with a raw ext4 image containing a file using extents (generate with
  `mke2fs` + `debugfs` or ship a minimal raw fixture)
- Verify `read_file()` returns correct byte count and SHA256 matches original

---

### Phase 45 — File Browser Extraction (Critical Blocker)

**Risk:** `FilesystemParser::read_file()` is fully implemented in all three parsers but
is unreachable from the UI. A user who browses to a recoverable deleted file has no way
to extract it.

**Changes:**
- `ferrite-tui/src/screens/file_browser/input.rs` — add `e` key handler
- Spawn a background task: `parser.read_file(entry, &mut File::create(dest_path))`
- Display per-entry extraction status in the entry list:
  - `[extracting…]` → spinner while in progress
  - `✓ 2.1 MiB → ferrite_extract_resume.docx` on success
  - `✗ read error at block 0x1A230` on failure
- `dest_path` = configurable output directory (default: `./ferrite_output/fs_extract/`)
- Preserve original filename from `FileEntry::name`; sanitise forbidden characters

**Tests:**
- TUI input test: `e` key emits correct extraction message
- Integration test: FileBlockDevice + NtfsParser + extraction roundtrip produces
  bit-identical output

---

### Phase 45b — Quick Deleted-File Recovery Mode

**Motivation:** The most common real-world scenario is a healthy drive where a user
accidentally deleted something. Full imaging is wasteful and stressful on the hardware.
This mode bypasses imaging entirely — it reads filesystem metadata directly from the
live drive, scores recoverability per file, and extracts selected files in seconds.

**Design principle:** No imaging. No carving. Read the MFT / FAT directory / ext4 inodes
directly from the mounted (read-only) source device, find deleted records, assess
whether the data is still intact, and extract on demand.

#### Recovery Probability Model

| Filesystem | Confidence Basis |
|---|---|
| **NTFS** | Check if each cluster run in the deleted MFT record's `$DATA` attribute is still marked free in the `$Bitmap`. All free = `High`. Some reallocated = `Medium`. All reallocated = `Low`. |
| **FAT32** | Directory entry intact (0xE5 prefix, start cluster readable). FAT chain zeroed so only contiguous runs are safe. Flag `High` if start cluster is in free space; `Low` if FAT shows reuse. |
| **ext4** | Inode `dtime` non-zero; check block bitmap for each block pointer or extent block. Same logic as NTFS. |

Recovery probability drives a colour-coded indicator in the TUI:
- `🟢 High` — data almost certainly intact; clusters still unallocated
- `🟡 Medium` — partially overwritten; partial recovery likely
- `🔴 Low` — clusters reallocated; recovery unlikely but attempt available

#### New TUI Screen: Quick Recover

Accessible from the main menu via a new `Q` shortcut (or as the first option after
filesystem detection, before the full imaging workflow).

**Flow:**
```
Select Device → [Q] Quick Recover (no imaging)
  → Filesystem detected: NTFS
  → Scanning MFT for deleted records… (1–10 seconds)
  → Deleted File List (filterable, sortable)
      Name         | Size   | Deleted     | Type | Chance
      report.docx  | 1.2 MB | 2026-03-15  | DOCX | 🟢 High
      photo.jpg    | 3.8 MB | 2026-03-10  | JPG  | 🟡 Medium
      archive.zip  | 45 MB  | 2026-02-28  | ZIP  | 🔴 Low
  → Space = select/deselect, Enter = preview metadata, R = recover selected
  → Output: ./ferrite_output/quick_recover/
```

**Changes:**
- `ferrite-filesystem/src/ntfs.rs` — extend `deleted_files()` to also return cluster
  bitmap check result per entry; add `RecoveryChance` enum (`High`, `Medium`, `Low`) to
  `FileEntry`
- `ferrite-filesystem/src/fat32.rs` — same: check if start cluster is still free
- `ferrite-filesystem/src/ext4.rs` — same: check block/extent bitmap
- `ferrite-filesystem/src/lib.rs` — add `RecoveryChance` to `FileEntry` struct
- `ferrite-tui/src/screens/quick_recover/mod.rs` — new screen module
- `ferrite-tui/src/screens/quick_recover/render.rs` — table layout with colour-coded
  chance indicator, size, deletion timestamp (from MFT `$STANDARD_INFORMATION` or ext4
  `dtime`), file type icon
- `ferrite-tui/src/screens/quick_recover/input.rs` — multi-select (`Space`), `R` to
  recover, `/` to filter by name/type, `s` to sort by chance/date/size
- `ferrite-tui/src/screens/quick_recover/extract.rs` — batch extraction using
  `FilesystemParser::read_file()` in a background task per selected file; per-file
  progress and status update back to UI
- `ferrite-tui/src/app.rs` — add `Screen::QuickRecover` variant; `Q` key on device
  menu routes here

**Key design decisions:**
- **No write to source.** Output is always to a user-specified destination directory,
  never back to the source device.
- **Graceful partial recovery.** If `RecoveryChance` is `Medium` or `Low`, attempt
  extraction anyway; truncate cleanly at the first unreadable cluster; annotate the
  output filename `_partial_<bytes>.jpg` so the user knows.
- **Skips allocated files entirely.** Only entries with the "deleted" flag are shown.
  Allocated, live files are invisible in this mode — the user uses Explorer for those.
- **Works on disk images too.** Since `FilesystemParser` operates on a `BlockDevice`
  trait, this mode works identically on a `.img` file opened as `FileBlockDevice`.

**Tests:**
- Unit: `deleted_files()` on a known NTFS fixture returns only deleted entries with
  correct `RecoveryChance` values
- Unit: FAT32 0xE5 entries are discovered; live entries are excluded
- Integration: extract a deleted NTFS file from a `FileBlockDevice` fixture; SHA256
  matches original
- TUI: selecting multiple files and pressing `R` emits the correct batch extract message

---

### Phase 46 — Tier 1 Signature Batch (WebP, AAC/M4A, GZip, EML, ELF, REGF, PSD, VHD, VHDX, QCOW2)

Add 10 high-value signatures. Signature count goes from 43 → 53.

**Signatures to add:**

| Sig | Extension | Header | Pre-validate Logic |
|---|---|---|---|
| WebP | `webp` | `52 49 46 46 ?? ?? ?? ?? 57 45 42 50` | Confirm "WEBP" at +8; reuse RIFF size hint |
| AAC / M4A | `m4a` | `?? ?? ?? ?? 66 74 79 70 4D 34 41 20` | ISOBMFF brand `M4A `; reuse mp4 box walker |
| GZip | `gz` | `1F 8B` | Byte @2 = compression method (must be 8); byte @3 = flags (bit 5 must be 0) |
| EML | `eml` | `46 72 6F 6D 20` ("From ") | Next chars match `[A-Za-z0-9._@\-]+\s` (email address pattern) |
| ELF | `elf` | `7F 45 4C 46` | EI_CLASS @4 in {1=32-bit, 2=64-bit}; EI_DATA @5 in {1=LE, 2=BE}; e_type @16 in {0-4} |
| REGF | `dat` | `72 65 67 66` ("regf") | Checksum u32 LE @508 validates; major version @20 in {1} |
| PSD / PSB | `psd` | `38 42 50 53` ("8BPS") | Version u16 BE @4 in {1,2}; channels u16 BE @6 in [1,56] |
| VHD | `vhd` | `63 6F 6E 65 63 74 69 78` ("conectix") | Disk type u32 BE @60 in {2=fixed,3=dynamic,4=diff}; checksum validates |
| VHDX | `vhdx` | `76 68 64 78 66 69 6C 65` ("vhdxfile") | Signature matches; log GUID at offset 192 is valid |
| QCOW2 | `qcow2` | `51 46 49 FB` | Version u32 BE @4 in {2,3}; cluster_bits u32 BE @20 in [9,21] |

**Changes:**
- `config/signatures.toml` — add 10 `[[signature]]` entries
- `crates/ferrite-carver/src/pre_validate.rs` — add validators: `Webp`, `M4a`, `Gz`,
  `Eml`, `Elf`, `Regf`, `Psd`, `Vhd`, `Vhdx`, `Qcow2`
- `crates/ferrite-carver/src/lib.rs` — update assertion to `53`
- `crates/ferrite-tui/src/screens/carving/helpers.rs` — add new sigs to appropriate
  groups; add `Forensic` group for REGF / LNK / Prefetch entries

**Tests:**
- 10 new unit tests in `ferrite-carver` (one per pre-validator with valid + invalid fixtures)

---

### Phase 47 — Image Integrity Hash + Read-Rate Monitoring

**Motivation:** A professional imaging job requires a SHA-256 hash for forensic chain of
custody. Read-rate monitoring catches a drive entering failure spiral.

**Changes (SHA-256 streaming hash):**
- `ferrite-imaging/Cargo.toml` — add `sha2 = "0.10"` dependency
- `ferrite-imaging/src/progress.rs` — add `image_sha256: Option<String>` to
  `ProgressUpdate`
- `ferrite-imaging/src/engine.rs` — maintain a `sha2::Sha256` hasher alongside the
  `BufWriter`; feed every successfully written buffer; finalise on completion
- `ferrite-tui/src/screens/imaging/render.rs` — display SHA-256 digest line when status
  is `Complete`

**Changes (read-rate monitoring):**
- `ferrite-imaging/src/progress.rs` — add `read_rate_bps: u64` and
  `low_rate_alert: bool` fields
- `ferrite-imaging/src/engine.rs` — maintain a ring buffer of the last 10 seconds of
  throughput; compute rolling average; emit `low_rate_alert = true` when sustained rate
  < configurable threshold (default: 1 MB/s for 30 s)
- `ferrite-tui/src/screens/imaging/render.rs` — show read rate gauge; flash amber when
  `low_rate_alert` is set

---

### Phase 48 — Raw Sector Hex Viewer

**Motivation:** When filesystems are undetectable or encrypted, the hex viewer is the
recovery engineer's last tool. It is also essential for manual MBR/GPT inspection and
carve-hit context.

**New screen:** `ferrite-tui/src/screens/hex_viewer/`

**Features:**
- Navigate by LBA (sector) or absolute byte offset
- Display: `offset | hex bytes (16 per row) | ASCII representation`
- Keyboard: `↑/↓` = scroll row, `PgUp/PgDn` = scroll page, `g` = go-to LBA dialog,
  `s` = go-to sector, `/` = find hex pattern (naive forward scan)
- Read 4 sectors (2 KiB) per page at minimum; configurable page size
- Highlight offset under cursor in both hex and ASCII columns
- Accessible from the main device menu and from the carving hit context (jump to hit LBA)

**Changes:**
- `ferrite-tui/src/screens/hex_viewer/mod.rs` — `HexViewerState`
- `ferrite-tui/src/screens/hex_viewer/render.rs` — layout
- `ferrite-tui/src/screens/hex_viewer/input.rs` — key handling + go-to dialog
- `ferrite-tui/src/app.rs` — add `Screen::HexViewer` variant; route `h` from device
  menu

**Tests:**
- Render test: given 512 bytes of known data, output matches expected hex dump format
- Go-to dialog: entering LBA N loads correct sector from `BlockDevice`

---

### Phase 49 — LBA Range Selection + Partial Imaging

**Motivation:** Recovery engineers frequently need to image a single partition, skip a
mechanically unstable region at the start of a drive, or image the end of a disk first.

**Changes:**
- `ferrite-core/src/config.rs` — add `start_lba: Option<u64>` and `end_lba: Option<u64>`
  to `ImagingConfig`; add `reverse: bool` (image from `end_lba` downward)
- `ferrite-imaging/src/engine.rs` — initialise the mapfile with only the requested LBA
  range as `NonTried`; everything outside is pre-marked `Finished` (skip entirely)
- `ferrite-tui/src/screens/imaging/config.rs` — add LBA range input fields to the
  imaging config screen; pre-populate from detected partition table
- Document interaction: if a partition is selected in the partition browser, offer to
  pre-fill start/end LBA automatically

---

### Phase 50 — exFAT + HFS+ + APFS Detection

**Motivation:** exFAT is the dominant filesystem on SD cards and USB drives >32 GB.
HFS+ covers every Mac produced before 2017. APFS covers Macs from 2017 onward. All
three are currently silently misidentified as `FilesystemType::Unknown`.

**Scope: Detection only — not full parsing (parsing is future work).**

**Changes:**
- `ferrite-filesystem/src/lib.rs`:
  - exFAT: check for `"EXFAT   "` (8 bytes) at sector offset 3
  - HFS+: check for `0x482B` (`H+`) at byte offset 1024 from partition start
  - APFS: check for `0x4253584E` (`NXSB`) at byte 0 of the APFS container
  - Return new `FilesystemType` variants with a clear message:
    `"exFAT detected — full parsing not yet supported"`
- `ferrite-tui/src/screens/file_browser/` — display the "not yet supported" message
  gracefully rather than an error; suggest using carving mode instead
- `ferrite-core/src/types.rs` — add `ExFat`, `HfsPlus`, `Apfs` variants to
  `FilesystemType`

---

### Phase 51 — S.M.A.R.T. Integration: Bad LBA Pre-population + Thermal Guard

**Motivation:** Two separate gaps addressed together since both require wiring
`ferrite-smart` into `ferrite-imaging`:

**A. Bad LBA pre-population:**
- `ferrite-smart` — parse `smartctl -l error --json` output; extract LBA fields from
  error log entries; export as `Vec<u64>` from `SmartReport`
- `ferrite-imaging/src/engine.rs` — accept `bad_lbas: &[u64]` at session init; mark
  corresponding mapfile blocks as `BadSector` before Pass 1 begins
- `ferrite-tui` — show "Pre-populated N known bad LBAs from S.M.A.R.T." in imaging
  status

**B. Thermal guard:**
- `ferrite-imaging/src/engine.rs` — accept a `temperature_fn: Option<Box<dyn Fn() -> Option<u8> + Send>>`
  callback; call it every 60 seconds; if temperature ≥ configurable threshold (default
  55°C for HDD, 70°C for SSD), pause with `tracing::warn!`; resume when temperature
  drops below threshold − 5°C hysteresis
- `ferrite-tui/src/screens/imaging/render.rs` — show live drive temperature on the
  imaging screen (updated every 60 s via background S.M.A.R.T. poll)

---

### Phase 52 — Tier 2 Signature Batch (MIDI, AIFF, XZ, BZip2, RealMedia, ICO, ORF, PEF, Mach-O)

Add 9 more signatures. Count goes from 53 → 62.

| Sig | Extension | Header | Notes |
|---|---|---|---|
| MIDI | `mid` | `4D 54 68 64` | Chunk length BE @4 must be 6; format u16 @8 in {0,1,2} |
| AIFF | `aif` | `46 4F 52 4D ?? ?? ?? ?? 41 49 46 46` | RIFF/IFF big-endian; subtype "AIFF" or "AIFC" |
| XZ | `xz` | `FD 37 7A 58 5A 00` | Stream flags @6–7; CRC32 @8 validates |
| BZip2 | `bz2` | `42 5A 68 3? ` | Byte @3 is `'1'`–`'9'`; byte @4 `31 41 59 26 53 59` = pi (stream magic) |
| RealMedia | `rm` | `2E 52 4D 46` | Object size BE @4 ≥ 18; version @8 in {0} |
| ICO | `ico` | `00 00 01 00` | Count u16 LE @4 in [1,500]; type u16 LE @2 in {1=ICO, 2=CUR} |
| Olympus ORF | `orf` | `49 49 52 4F 08 00 00 00` | Validate `OLYMP` string in first 512 bytes |
| Pentax PEF | `pef` | `49 49 2A 00` | Pre-validate "PENTAX" in first 512 bytes (TIFF LE base) |
| Mach-O 64 | `macho` | `CF FA ED FE` | CPU type u32 LE @4; file type u32 LE @12 in [1–8] |

---

### Phase 53 — Write-Blocker Verification + Forensic Workflow Mode

**Motivation:** Before every imaging session, a forensic-grade tool must prove the source
device is read-only. This protects against accidental writes caused by misconfigured
software write blockers or hardware write-blocker failures.

**Changes:**
- `ferrite-blockdev/src/windows.rs` — add `verify_read_only()`: attempt a `WriteFile`
  of 0 bytes at offset 0 on the source handle; assert it fails with
  `ERROR_ACCESS_DENIED` or `ERROR_WRITE_PROTECT`; return `Ok(true)` only on verified
  read-only
- `ferrite-blockdev/src/linux.rs` — attempt `pwrite` of 0 bytes; expect `EACCES` or
  `EROFS`
- `ferrite-imaging/src/engine.rs` — call `verify_read_only()` before starting any pass;
  abort with a clear error if the check fails
- `ferrite-tui` — display write-blocker verification result as a pre-flight checklist
  step on the imaging config screen: `✓ Write protection verified` / `⚠ Write protection
  NOT confirmed — proceed with caution`

---

### Phase 54 — Recovery Report Export

**Motivation:** Every professional recovery engagement ends with a deliverable for the
client or organisation. Ferrite already collects all the data needed.

**Report sections:**
1. **Device** — model, serial, firmware, capacity, interface
2. **S.M.A.R.T. verdict** — overall health, key attributes (reallocated sectors, pending
   sectors, uncorrectable, temperature at time of imaging)
3. **Imaging summary** — start time, end time, total bytes imaged, bad sector count,
   percentage recovered, SHA-256 hash of image file
4. **Partition table** — type (MBR/GPT), partitions found, filesystem per partition
5. **Filesystem analysis** — for each partition: file count, deleted file count, total
   space, free space
6. **Carving results** — signatures enabled, hits found per type, files extracted, total
   extracted bytes

**Changes:**
- New crate `ferrite-report` (optional, or module within `ferrite-tui`)
- Output formats: HTML (styled, printable) and plain text
- `ferrite-tui` — add `R` key on the main menu to export report; show save-path dialog
- CLI: `ferrite report --session ferrite-session.json --output report.html`

---

### Phase 55 — Carve Hit Integrity Validation + Duplicate Suppression

**Motivation:** The current carver extracts all hits with no quality assessment.
Fragmented files produce truncated, invalid output silently. Thumbnail/cache hits produce
duplicate content.

**A. Post-extraction validation:**
- After extraction, re-run the relevant `pre_validate` function on the extracted bytes
- For formats with known structural requirements (PDF, ZIP, PNG, JPEG), attempt a deeper
  structural parse: PDF `startxref` + xref table presence; ZIP central directory;
  PNG `IEND` chunk
- Tag each hit: `Complete`, `Truncated`, `Corrupt`, `Unknown`
- Surface tags in the TUI hit list with colour coding (green/yellow/red)

**B. Duplicate suppression:**
- Compute SHA-1 of the first 4 KiB of each extracted file (fast fingerprint)
- Maintain a `HashSet<[u8; 20]>` across the scan; skip extraction for hits whose
  fingerprint has already been seen
- Show "N duplicates suppressed" counter in the scan summary

---

### Phase 56 — Custom User Signatures (TUI)

**Motivation:** Users need to recover proprietary formats, database file types, or
internal application formats not in the built-in database. PhotoRec and Scalpel both
support this; it is a competitive requirement.

**Design:**
- User signatures stored in `ferrite-user-signatures.toml` (same schema as
  `config/signatures.toml`; loaded at runtime and merged with built-in sigs)
- `ferrite-tui` — add a "Custom Signatures" sub-screen accessible from the carving screen
  - `a` = add new signature (name, extension, header hex, max_size)
  - `e` = edit selected
  - `d` = delete selected (with confirmation)
  - `i` = import from file path
- Validate hex header syntax on entry; reject obviously invalid entries
- User sigs appear in their own "Custom" group in the grouped sig panel

---

### Phase 57 — Forensic Artifact Scanner (Bulk Extractor Mode)

**Motivation:** Beyond file recovery, digital forensics requires scanning unallocated
space for structured data artefacts: email addresses, URLs, credit card numbers, and
social media identifiers. Bulk Extractor pioneered this approach.

**Architecture:**
- New crate `ferrite-artifact` (or submodule of `ferrite-carver`)
- Trait `ArtifactScanner: Send + Sync { fn scan(&self, block: &[u8]) -> Vec<ArtifactHit> }`
- Built-in scanners (regex-based):

| Scanner | Pattern | Output |
|---|---|---|
| Email | RFC 5321 local-part + `@` + domain | Deduplicated list of email addresses |
| URL | `https?://[^\s"'<>]+` | Unique URLs found |
| Credit Card | Luhn-validated 13–19-digit runs | Masked CC numbers (last 4 only in output) |
| IBAN | ISO 13616 pattern | Financial account numbers |
| Windows path | `[A-Za-z]:\\[\w\\. ]+` | File paths (proves file existence) |
| Social Security (US) | `\d{3}-\d{2}-\d{4}` | PII artefacts |

- Results exported to `ferrite_artifacts.csv` and displayed in a separate TUI screen
- **Privacy note:** artefact scanner is opt-in and disabled by default; consent dialog
  required before first scan

---

### Phase 59 — PhotoRec Gap Batch: Tier A Quick Wins (9 new → +9 signatures)

Add signatures that require no scanner infrastructure changes — reuse existing
ISOBMFF box walker, TIFF IFD walker, ZIP inner validator, OLE2 validator, or RIFF
size-hint.

| Sig | Ext | Header | Mechanism |
|---|---|---|---|
| Canon CR3 | `cr3` | `?? ?? ?? ?? 66 74 79 70 63 72 78 20` | ISOBMFF box walker; `pre_validate = "cr3"` (brand `crx `) |
| Sony SR2 | `sr2` | `49 49 2A 00 08 00 00 00` | TIFF IFD walker; validate "SR2" marker |
| EPUB | `epub` | `50 4B 03 04` (ZIP) | ZIP inner validator; first entry `mimetype` = `application/epub+zip` |
| OpenDocument | `odt` | `50 4B 03 04` (ZIP) | ZIP inner validator; first entry `mimetype` = ODF MIME type |
| Outlook MSG | `msg` | `D0 CF 11 E0 A1 B1 1A E1` (OLE2) | OLE2 validator; check `__substg1.0` stream name in directory |
| WavPack | `wv` | `77 76 70 6B` ("wvpk") | New standalone; validate block_samples u32 LE @4 > 0 |
| CorelDRAW | `cdr` | `52 49 46 46 ?? ?? ?? ?? 43 44 52 56` | RIFF size-hint; validate subtype "CDRV"/"CDRX" |
| Shockwave Flash | `swf` | `46 57 53` / `43 57 53` / `5A 57 53` | New standalone; 3 variants; version byte @3 in [1,45] |
| Kodak DCR | `dcr` | `49 49 2A 00` (TIFF LE) | TIFF IFD walker; validate Kodak Make tag |

**Changes:**
- `config/signatures.toml` — 9 new `[[signature]]` entries
- `crates/ferrite-carver/src/pre_validate.rs` — add: `Cr3`, `Sr2`, `Epub`, `Odt`,
  `Msg`, `Wv`, `Cdr`, `Swf`, `Dcr`
- `crates/ferrite-carver/src/lib.rs` — update assertion count

---

### Phase 60 — PhotoRec Gap Batch: Tier B New-Infrastructure (13 new → +13 signatures)

Add high-value formats that need new pre-validators with no walker reuse.

| Sig | Ext | Magic | Notes |
|---|---|---|---|
| Canon CRW | `crw` | `49 49 1A 00 00 00 48 45 41 50 43 43 44 52` | Validate "HEAPCCDR" string |
| Minolta MRW | `mrw` | `00 4D 52 4D` | Validate "PRD\0"/"TTW\0" block type |
| KeePass 2.x | `kdbx` | `03 D9 A2 9A 67 FB 4B B5` | 8-byte magic; version check |
| KeePass 1.x | `kdb` | `03 D9 A2 9A 65 FB 4B B5` | Differs at byte 7 from kdbx |
| EnCase E01 | `e01` | `45 56 46 09 0D 0A FF 00` | 8-byte magic; segment type @8 == 0x01 |
| PCAP | `pcap` | `D4 C3 B2 A1` (LE) / `A1 B2 C3 D4` (BE) | Two-variant magic; version check |
| Windows Minidump | `dmp` | `4D 44 4D 50 93 A7` ("MDMP") | Validate stream_count u32 LE @8 > 0 |
| Apple plist | `plist` | `62 70 6C 69 73 74 30 30` | Validate trailer (last 26 bytes) |
| MPEG-TS | `ts` | `47` (sync, stride 188) | Validate 3 consecutive sync bytes at 188-byte intervals |
| M2TS (Blu-ray) | `m2ts` | `47` at offset 4 (stride 192) | Validate 3 consecutive syncs at 192-byte intervals |
| LUKS Encrypted | `luks` | `4C 55 4B 53 BA BE` ("LUKS\xBA\xBE") | Validate version u16 BE @6 in {1,2} |
| Apple EMLX | `emlx` | ASCII decimal + newline | Validate numeric byte count line + `X-Apple-UUID:` header |
| Sigma X3F | `x3f` | `46 4F 56 62` ("FOVb") | Validate version u32 LE @4 in known set |

---

### Phase 61 — PhotoRec Gap Batch: Tier B Continued (8 new → +8 signatures)

| Sig | Ext | Magic | Notes |
|---|---|---|---|
| Monkey's Audio | `ape` | `4D 41 43 20` ("MAC ") | Version u16 LE @6 in [3930, 4100] |
| Sun AU Audio | `au` | `2E 73 6E 64` (".snd") | data_offset @4 ≥ 24; encoding @12 in known set |
| TrueType Font | `ttf` | `00 01 00 00 00` | numTables u16 BE @4 in [4,50] |
| WOFF Font | `woff` | `77 4F 46 46` ("wOFF") | Validate flavor + length fields |
| CHM Help | `chm` | `49 54 53 46 03 00 00 00 60 00 00 00` | 12-byte magic fully deterministic |
| Blender | `blend` | `42 4C 45 4E 44 45 52` + pointer/endian bytes | Validate pointer-size + endian byte |
| Adobe InDesign | `indd` | `06 06 ED F5 D8 1D 46 E5 BD 31 EF E7 FE 74 B7 1D` | 16-byte GUID, globally unique |
| Windows WTV | `wtv` | `B7 D8 00 20 37 49 DA 11 A6 4E 00 07 E9 5E AD 8D` | 16-byte GUID, same pattern as WMV/ASF |

---

### Phase 62 — Non-Zero Offset Scan (ISO, DICOM, TAR)

**Motivation:** Three high-value formats (ISO 9660, DICOM medical images, TAR archives)
have their identifying magic bytes at a non-zero offset within the file. The current
scanner cannot detect these.

**Infrastructure change:**
- `ferrite-core/src/signature.rs` — add `header_offset: u64` field to `Signature`
  (default 0; non-zero means "the magic appears at this offset within the file, not
  at byte 0")
- `ferrite-carver/src/scan_search.rs` — in `find_all()`, when a signature has
  `header_offset > 0`, shift the candidate start back by that amount; validate the
  full expected magic at position `found_pos` while emitting `CarveHit` at
  `found_pos - header_offset`
- `config/signatures.toml` — add `header_offset` to ISO, DICOM, TAR entries

| Sig | Ext | Magic | Offset | Notes |
|---|---|---|---|---|
| ISO 9660 | `iso` | `43 44 30 30 31` ("CD001") | 32769 | Primary Volume Descriptor at sector 16 + 1 |
| DICOM | `dcm` | `44 49 43 4D` ("DICM") | 128 | 128-byte preamble precedes the tag |
| TAR | `tar` | `75 73 74 61 72 00` ("ustar\0") | 257 | POSIX ustar magic in header block |

---

### Phase 63 — Code File Signatures (+2 signatures: PHP, Shebang scripts)

**Motivation:** Source code files with unambiguous openers can be recovered reliably
by carving without filesystem metadata. PHP files always start with `<?php`. Shell,
Python, Ruby, Perl, and Node.js scripts almost always start with a shebang (`#!/`).
These two signatures cover a large proportion of real-world code file recovery needs.

| Sig | Ext | Magic | Notes |
|---|---|---|---|
| PHP script | `php` | `3C 3F 70 68 70` (`<?php`) | Pre-validate: byte @5 is space/newline/tab; after shebang line content is ≥ 70% printable |
| Shebang script | `sh`/`py`/`rb`/`pl`/`js` | `23 21 2F` (`#!/`) | Pre-validate: path is ASCII printable; shebang line ≤ 128 bytes; classify by interpreter name |

**Shebang extension classification** (via pre-validator):
- `python`/`python3` → `.py`
- `ruby` → `.rb`
- `perl` → `.pl`
- `node`/`nodejs` → `.js`
- all others (bash/sh/zsh/dash) → `.sh`

**Changes:**
- `config/signatures.toml` — 2 new entries (`php`, shebang)
- `crates/ferrite-carver/src/pre_validate.rs` — add `Php` and `Shebang` variants + validators
- `crates/ferrite-carver/src/lib.rs` — update assertion (73 → 75)
- `crates/ferrite-tui/src/screens/carving/helpers.rs` — route `php`/`sh`/`py`/`rb`/`pl`/`js` → Documents group

**Note:** `.bat`/`.cmd` files (starting with `@echo`) are intentionally excluded — the
opener is too fragile and occurs frequently in binary data. BAT files are best recovered
via filesystem metadata (Tab 4/7).

---

### Phase 64 — Heuristic Text Block Scanner

**Motivation:** Plain text files, Markdown, and most source code have no binary magic
bytes and cannot be recovered by the signature-based carver. When filesystem metadata
is unavailable, the only option is a heuristic scan: identify contiguous regions of
valid UTF-8 / ASCII text in the raw device stream, classify them by content, and emit
each as a candidate file. Results are inherently variable quality — some blocks will be
partial files or merged fragments — but recovering *something* is better than recovering
nothing.

This phase adds a new crate `ferrite-textcarver` and a new TUI tab "Text Scan" (Tab 9).

---

#### Architecture

```
ferrite-textcarver/
  src/
    lib.rs          — pub re-exports
    scanner.rs      — TextBlock, TextKind, TextScanConfig, TextScanMsg, TextScanProgress
    engine.rs       — run_scan(), sliding window scanner, gap-tolerant UTF-8 accumulation
    classifier.rs   — classify() → (TextKind, confidence: u8)
    export.rs       — write_files(output_dir, blocks) → (written, errors)
  Cargo.toml        — deps: ferrite-blockdev, tracing, thiserror

ferrite-tui/src/screens/text_scan/
  mod.rs            — TextScanState, ScanStatus, tick(), start_scan(), cancel_scan(), export_files()
  input.rs          — handle_key(): consent, output dir editing, navigation, s/c/e/o/0-6 filter
  render.rs         — full render: output bar, progress gauge, block list, status bar, consent overlay
```

---

#### Core Data Model (`scanner.rs`)

```rust
/// Classification of a recovered text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextKind {
    Php,        // <?php opener
    Script,     // #!/ shebang (sh/py/rb/pl/js — extension set by classifier)
    Json,       // { or [ with "key": value structure
    Yaml,       // --- frontmatter or key: value density
    Markup,     // <html / <!DOCTYPE / <?xml
    Sql,        // SELECT/INSERT/CREATE/UPDATE keyword density
    CSource,    // #include / typedef / struct keyword density
    Markdown,   // # headings + **bold** / [link]() patterns
    Generic,    // printable text, no strong classification signal
}

impl TextKind {
    pub fn extension(self) -> &'static str { … }  // php/sh/json/yaml/html/sql/c/md/txt
    pub fn label(self) -> &'static str { … }       // display name for TUI filter
}

pub struct TextBlock {
    pub byte_offset:  u64,
    pub length:       u64,
    pub kind:         TextKind,
    pub confidence:   u8,    // 0–100; how confident the classifier is
    pub quality:      u8,    // 0–100; printable_bytes / total_bytes × 100
    pub preview:      String, // first ≤ 80 chars, newlines replaced with ↵
}

pub struct TextScanConfig {
    pub min_block_bytes:     u64,   // default: 256
    pub max_block_bytes:     u64,   // default: 1_048_576 (1 MiB)
    pub gap_tolerance_bytes: usize, // default: 8 — non-printable bytes before block ends
    pub min_printable_pct:   u8,    // default: 80 — % of bytes that must be "text-like"
    pub chunk_bytes:         u64,   // default: 1_048_576 (1 MiB read per I/O call)
    pub overlap_bytes:       usize, // default: 4096 — tail of chunk prepended to next
}

pub struct TextScanProgress {
    pub bytes_done:  u64,
    pub bytes_total: u64,
    pub blocks_found: usize,
}

pub enum TextScanMsg {
    BlockBatch(Vec<TextBlock>),
    Progress(TextScanProgress),
    Done { total_blocks: usize },
    Error(String),
}
```

---

#### Scanner Engine (`engine.rs`)

The engine uses a **gap-tolerant sliding window** over the raw device stream:

1. Read aligned 1 MiB chunks from the device. Prepend the 4 KiB overlap tail from the
   previous chunk to catch blocks that span chunk boundaries.

2. Walk byte-by-byte through the chunk maintaining state:
   - `in_block: bool` — whether we are accumulating a text block
   - `block_start: u64` — absolute device offset where current block began
   - `gap_run: usize` — consecutive non-text bytes seen while in a block
   - `printable_count: u64` / `total_count: u64` — for quality scoring

3. **"Text-like" definition** (used for both start detection and gap counting):
   - ASCII printable: 0x09 (tab), 0x0A (LF), 0x0D (CR), 0x20–0x7E
   - Valid UTF-8 continuation: 0x80–0xBF following a valid lead byte
   - UTF-8 lead bytes: 0xC2–0xF4 (2–4 byte sequences)
   - Everything else: non-text

4. **Block start**: Three consecutive text-like bytes → `in_block = true`, record offset.

5. **Block continuation**: Each text-like byte resets `gap_run = 0`. Each non-text byte
   increments `gap_run`.

6. **Block end triggers**:
   - `gap_run > gap_tolerance_bytes` → emit block (if length ≥ `min_block_bytes`)
   - `length == max_block_bytes` → emit block and immediately start new block at next byte
   - End of device → emit any in-progress block

7. **Quality gate**: After extraction, compute `printable_count / total_count`. If below
   `min_printable_pct` threshold, discard the block rather than emitting it.

8. **Deduplication**: Maintain a `HashSet<u64>` of xxHash-64 digests of each block's
   content. Skip blocks whose hash has already been seen (handles sectors read twice
   due to overlaps or defect-retry logic).

9. Emit `TextScanMsg::BlockBatch` periodically (every 50 blocks or 5 s, whichever
   first) and `TextScanMsg::Progress` every 1 MiB.

---

#### Classifier (`classifier.rs`)

The classifier examines the first 256 bytes of a block (for openers) and the first
1 KiB (for keyword density). Returns `(TextKind, confidence: u8)`.

**Priority order (first match wins):**

| Check | Kind | Confidence |
|---|---|---|
| Starts with `<?php` | `Php` | 99 |
| Starts with `#!/` + valid path | `Script` | 97 |
| Starts with `<?xml` | `Markup` | 99 |
| Starts with `<!DOCTYPE` or `<html` (case-insensitive) | `Markup` | 95 |
| Starts with `{` or `[` + `"key":` pattern ≥ 2× | `Json` | 85 |
| Starts with `---\n` or `---\r\n` | `Yaml` | 90 |
| SQL keyword density ≥ 3 distinct (`SELECT`/`INSERT`/`CREATE`/`UPDATE`/`DELETE`/`FROM`/`WHERE`) | `Sql` | 70 |
| C keyword density ≥ 3 (`#include`/`#define`/`typedef`/`struct`/`void`/`int`) | `CSource` | 65 |
| Markdown pattern density ≥ 2 (`# `/`## `/`**`/`[`+`](`/`- `+`[ ]`) | `Markdown` | 65 |
| None of the above | `Generic` | 50 |

**Script sub-classification** (sets `TextBlock::extension` via `TextKind::Script`):
- shebang line contains `python` or `python3` → `py`
- shebang line contains `ruby` → `rb`
- shebang line contains `perl` → `pl`
- shebang line contains `node` → `js`
- otherwise → `sh`

---

#### Exporter (`export.rs`)

```rust
pub fn write_files(output_dir: &str, blocks: &[TextBlock]) -> (usize, Vec<String>)
```

- Creates `output_dir` if it does not exist
- Names each file: `text_<hex_offset>.<ext>` — e.g. `text_00A4BC00.md`
- Writes raw bytes (the content is already valid UTF-8 by definition of the scanner)
- Returns `(written_count, error_messages)`

---

#### TUI State (`mod.rs`)

```rust
pub struct TextScanState {
    device:           Option<Arc<dyn BlockDevice>>,
    status:           ScanStatus,
    blocks:           Vec<TextBlock>,
    block_sel:        usize,
    filtered:         Vec<usize>,          // indices into blocks
    progress:         Option<TextScanProgress>,
    scan_start:       Option<Instant>,
    cancel:           Arc<AtomicBool>,
    rx:               Option<Receiver<TextScanMsg>>,
    consent_given:    bool,
    show_consent:     bool,
    filter_kind:      Option<TextKind>,    // None = show all
    output_dir:       String,
    editing_dir:      bool,
    export_status:    Option<String>,
    blocks_page_size: usize,
    error_msg:        String,
}
```

`tick()` drains `rx` using the `self.rx.take()` pattern (same as ArtifactsState).

---

#### TUI Render (`render.rs`)

**Title bar:** ` Text Scan [status] — filter: <kind>  s:scan  c:cancel  e:export  o:output  0-8:filter `

**Layout (4 rows inside outer border):**
```
┌─ Text Scan [scanning…] ──────────────────────────────────────────────────────┐
│ Output dir: ./ferrite_text/  (o to edit)                                     │  ← row 0
│ ████████████░░░░░░  47 blocks  256/512 MiB  38s                              │  ← row 1 (progress)
│ 00A4BC00  2.1 KiB  py   95%  #!/usr/bin/python3↵import os↵import sys↵…     │  ← row 2 (block list)
│ 00A52000  512 B    txt  82%  This is a sample text document with no…        │
│  …                                                                            │
│ 0:all  1:php  2:script  3:json  4:yaml  5:markup  6:sql  7:csrc  8:md      │  ← row 3
└──────────────────────────────────────────────────────────────────────────────┘
```

**Block list columns:** `<hex_offset>  <size>  <kind>  <quality%>  <preview>`
- `kind` colored by TextKind (same color-per-kind approach as ArtifactsState)
- `quality%` colored: ≥90 = green, ≥70 = yellow, <70 = red

**Consent dialog:** Similar to Artifacts. Warns that results are variable quality, some
blocks may be partial files or merged fragments, and no data is written until `e` is pressed.

---

#### Integration (`app.rs`)

- `SCREEN_NAMES`: extend to 10 entries, add `" Text Scan "`
- `App` struct: add `text_scan: TextScanState`
- `App::tick()`: call `self.text_scan.tick()`
- `App::handle_key()`: route screen 9 → `self.text_scan.handle_key()`
- `App::render()`: route screen 9 → `self.text_scan.render()`
- `set_device()` propagation: call `self.text_scan.set_device(Arc::clone(&dev))`
- `is_editing()` check: add screen 9 to the `q`-quit guard
- Help line: add screen 9 entry

---

#### Known Limitations (documented in UI consent dialog)

1. **No filename recovery** — files are named by device offset only
2. **Merged fragments** — two adjacent text files with no binary gap will be
   emitted as one block; this is unavoidable without filesystem metadata
3. **False positives** — binary files that happen to contain long ASCII strings
   (e.g. PE executables, SQLite databases) will produce spurious hits; the quality
   score and min_printable_pct gate reduce but do not eliminate these
4. **No encoding detection** — only UTF-8 and ASCII are recognised; UTF-16 (common in
   Windows) requires separate handling (future phase)
5. **Tab 4 / Tab 7 always preferred** — if filesystem metadata is available, those
   screens recover text files with original names and zero false positives

---

**Changes:**
- New crate `crates/ferrite-textcarver/` (4 source files + Cargo.toml)
- `Cargo.toml` (workspace) — add `ferrite-textcarver` member + workspace dep
- `crates/ferrite-tui/Cargo.toml` — add `ferrite-textcarver` dep
- `crates/ferrite-tui/src/screens/mod.rs` — add `pub mod text_scan`
- `crates/ferrite-tui/src/screens/text_scan/` — 3 files (mod, input, render)
- `crates/ferrite-tui/src/app.rs` — wire Tab 9 into all routing paths

---

### Phase 58 — APFS + exFAT Full Parser (Stretch Goal)

**Motivation:** exFAT and APFS are the dominant filesystems on removable media and Apple
hardware respectively. Detection (Phase 50) gets Ferrite to "not broken"; full parsing
gets it to "recovers actual files."

**exFAT parser:**
- Boot sector at sector 0: VBR signature, cluster heap offset, root directory cluster
- FAT chain walk: 32-bit entries; cluster → LBA mapping
- Directory entries: `FileEntry` (85), `StreamExtension` (192), `FileName` (193)
- Deleted file detection: entry type byte high-bit cleared for deleted entries
- Implement `FilesystemParser` trait for `ExFatParser`

**APFS parser (minimum viable):**
- Container superblock (`nx_superblock_t`) at block 0
- Volume object map → volume superblock (`apfs_superblock_t`)
- B-Tree node walk for the file system object tree
- Inode + dentry objects → `FileEntry`
- Cloned file (reflink) awareness: extents can overlap in APFS
- Read-only; no snapshot support required for MVP

---

## Summary Priority Matrix

| # | Phase | Priority | Effort | Status |
|---|---|---|---|---|
| 43 | Per-read timeout | 🔴 Critical | High | ✅ **Done** |
| 44 | ext4 extents | 🔴 Critical | High | ✅ **Done** |
| 45 | File browser extraction | 🔴 Critical | Low | ✅ **Done** |
| 48 | Raw hex viewer | 🟠 High | Medium | ✅ **Done** |
| 49 | LBA range selection | 🟠 High | Medium | ✅ **Done** |
| 50 | exFAT / HFS+ detection | 🟡 Medium | Low | ✅ **Done** |
| 51a | SMART bad LBA → mapfile pre-pop | 🟡 Medium | Medium | ✅ **Done** |
| 54 | Recovery report export | 🟡 Medium | High | ✅ **Done** |
| 47a | Read-rate tracking (`read_rate_bps`) | 🟠 High | Low | ✅ **Done** |
| — | — | — | — | — |
| 45b | Quick Deleted-File Recovery mode | 🔴 Critical | Medium | ✅ **Done** |
| 46 | Tier 1 signatures (10 new → 53 total) | 🟠 High | Low | ✅ **Done** |
| 47b | SHA256 image integrity hash | 🟠 High | Low | ✅ **Done** |
| 47c | Low read-rate alert + TUI warning | 🟠 High | Low | ✅ **Done** |
| 51b | SMART thermal guard during imaging | 🟡 Medium | Medium | ✅ **Done** |
| 52 | Tier 2 signatures (9 new → 62 total) | 🟡 Medium | Low | ✅ **Done** |
| 53 | Write-blocker verification | 🟡 Medium | Low | ✅ **Done** |
| 55 | Carve hit validation + dedup | 🟡 Medium | Medium | ✅ **Done** |
| 56 | Custom user signatures (TUI) | 🟢 Low | Medium | ✅ **Done** |
| 57 | Forensic artifact scanner | 🟢 Low | High | ✅ **Done** |
| 59 | PhotoRec Tier A quick wins (9 new sigs) | 🟠 High | Low | ✅ **Done** |
| 60 | PhotoRec Tier B batch 1 (13 new sigs) | 🟡 Medium | Low | ✅ **Done** |
| 61 | PhotoRec Tier B batch 2 (8 new sigs) | 🟡 Medium | Low | ✅ **Done** |
| 62 | Non-zero offset scan (ISO, DICOM, TAR) | 🟡 Medium | Medium | ✅ **Done** |
| 63 | Code file signatures (PHP, shebang) | 🟡 Medium | Low | ✅ **Done** |
| 64 | Heuristic text block scanner | 🟡 Medium | High | ✅ **Done** |
| 58 | APFS + exFAT full parser | 🟢 Low | Very High | ✅ **Done** |

---

---

## Phase 100 — PhotoRec Gap Batch 1: High-Impact Quick Wins ✅ Done (2026-03-24)

**107 signatures total after this phase.**

Systematic cross-reference of Ferrite's signature database against the full PhotoRec
source tree (`ajnelson/photorec-testdisk` on GitHub). Two commits:

### Commit 1 — JPEG Raw/DQT (FF D8 FF DB)
Root-cause analysis of a live 20 GB image file revealed that 972 large photos (>100 KB)
start with `FF D8 FF DB` (DQT directly after SOI, no APP header). These were invisible
to the existing JFIF/Exif-only signatures. Pre-validator checks DQT segment length in
[67, 518]. **100 → 100 signatures.**

### Commit 2 — 7-signature batch
| # | Format | Header | Notes |
|---|--------|--------|-------|
| 101 | JPEG/COM | `FF D8 FF FE` | JPEG starting with Comment marker |
| 102 | Java Class | `CA FE BA BE` | Major version [45,80] validator |
| 103 | Microsoft Cabinet | `4D 53 43 46` | reserved1==0, size>0 validator; 512 MiB cap |
| 104 | OpenType Font OTF | `4F 54 54 4F` | `OTTO`; numTables [1,50] |
| 105 | WOFF2 | `77 4F 46 32` | `wOF2`; flavor ∈ {TrueType, CFF}; numTables [1,50] |
| 106 | Android DEX | `64 65 78 0A` | version = 3 ASCII digits + null |
| 107 | Adobe PSB | `38 42 50 53 00 02` | Photoshop Large Doc; reuses PSD validator; 2 GiB cap |

**Total: 100 → 107 signatures. 561 tests, all passing.**

---

## Phase 101 — PhotoRec Gap Batch 2: Common Consumer Formats ✅ Done (2026-03-24)

**115 signatures total after this phase.**

Added 8 signatures covering common consumer formats. All are quick-wins —
new signature + lightweight pre-validator, no scanner infrastructure changes.

| # | Format | Header | Pre-validate | Max size |
|---|--------|--------|--------------|----------|
| 108 | Raw AAC MPEG-4 ADTS | `FF F1` | `Aac`: layer==00; sfi≤12 | 50 MiB |
| 109 | Raw AAC MPEG-2 ADTS | `FF F9` | `Aac` (shared) | 50 MiB |
| 110 | DjVu Document | `AT&TFORM` (8 B) | `Djvu`: form type ∈ {DJVU,DJVM,DJVI,THUM} | 200 MiB |
| 111 | OpenEXR HDR Image | `76 2F 31 01` | — (4-byte unique) | 500 MiB |
| 112 | GIMP XCF Image | `gimp xcf v` (10 B) | `Xcf`: version `file\0` or 3 digits+`\0` | 500 MiB |
| 113 | JPEG 2000 | 12-byte sig box | — (globally unique) | 500 MiB |
| 114 | PCX Image | `0A` | `Pcx`: strict 7-field check | 50 MiB |
| 115 | BPG Image | `42 50 47 FB` | — (4-byte unique) | 50 MiB |

**Total: 107 → 115 signatures. 587 tests in ferrite-carver (+26), all passing.**

---

## Phase 102 — PhotoRec Gap Batch 3: Developer & Science Formats (Planned)

**Target: ~120 → ~130 signatures.**

Formats common on developer workstations, science/research machines, mobile devices.

| Format | Ext | Header (hex) | Pre-validate | Max size | Priority |
|--------|-----|-------------|--------------|----------|----------|
| Java Archive | `jar` | `50 4B 03 04` | ZIP with `META-INF/MANIFEST.MF` first entry | 500 MiB | High |
| Python bytecode | `pyc` | `55 0D 0D 0A` / `33 0D 0D 0A` / others | magic = Python version word; timestamp follows | 100 MiB | Medium |
| LZH/LHA archive | `lzh` | at offset 2: `-lh?-` | 5-char method ID; header checksum valid | 200 MiB | Medium |
| Microsoft CAB patch | `msp` | `D0 CF 11 E0` + `MSP` stream | OLE2 with `MSP` or `Patch` stream name | 500 MiB | Medium |
| HDF5 | `h5` | `89 48 44 46 0D 0A 1A 0A` | 8-byte magic + superblock version @8 ∈ {0,1,2,3} | 2 GiB | Low |
| FITS | `fits` | `53 49 4D 50 4C 45 20 20 3D` | `SIMPLE  =`; value must be `T` (true) @29 | 2 GiB | Low |
| Parquet | `parquet` | `50 41 52 31` at offset 0 AND footer | 4-byte `PAR1` magic at both start and end | 2 GiB | Low |
| DPX (film) | `dpx` | `53 44 50 58` / `58 50 44 53` | two endian variants; magic-only check sufficient | 2 GiB | Low |

**Implementation notes:**
- JAR: distinguish from generic ZIP by checking the ZIP central directory for
  `META-INF/MANIFEST.MF` — requires reading up to `max_size` bytes to find EOCD;
  use the existing EPUB/ODT inner-ZIP pattern
- Python `.pyc`: magic word varies by Python version (2.x: `0x0D0D`, 3.x: `0x0D0A`);
  build a table of known magic words
- LZH: the method field `-lh0-` through `-lhd-` at offset 2 is the key discriminator;
  offsets 0 and 1 are the header checksum (any value)

---

## Phase 103 — PhotoRec Gap Batch 4: Forensic & System Formats (Planned)

**Target: ~130 → ~140 signatures.**

Formats with high forensic / data-recovery value that require more careful validation
or larger max_size caps.

| Format | Ext | Header (hex) | Pre-validate | Max size | Priority |
|--------|-----|-------------|--------------|----------|----------|
| VirtualBox VDI | `vdi` | `7F 10 DA BE` at offset 0x40 | header_offset = 64; image type u32 LE @72 ∈ {1,2} | 2 TiB | High |
| AFF forensic image | `aff` | `41 46 46` | `AFF` magic; version u32 BE @3 ∈ {1} | 2 TiB | High |
| Windows LNK (Shell Link) | `lnk` | `4C 00 00 00 01 14 02 00` | 8-byte CLSID prefix; FileAttributes u32 LE @24 plausible | 1 MiB | High |
| Windows Prefetch | `pf` | `11 00 00 00` / `17 00 00 00` / `1A 00 00 00` | version byte ∈ {17, 23, 26, 30} | 10 MiB | Medium |
| Windows Event Log (EVT) | `evt` | `30 00 00 00 4C 66 4C 65` | 8-byte magic; OldestRecord u32 LE @16 > 0 | 100 MiB | Medium |
| PEM certificate | `pem` | `2D 2D 2D 2D 2D 42 45 47 49 4E` | `-----BEGIN`; next non-space byte valid label char | 1 MiB | Medium |
| Bitcoin wallet | `wallet` | `62 31 05 00 09 00` | Berkeley DB wallet; magic + version | 100 MiB | Low |
| Ethereum keystore | `json` | `7B 22 63 72 79 70 74 6F` | `{"crypto`; JSON structure for ETH keystore | 1 MiB | Low |

**Implementation notes:**
- VDI: `header_offset = 64` (uses non-zero-offset scanner infrastructure from Phase 62).
  Magic `7F 10 DA BE` is the VirtualBox disk image identifier at byte 64.
- LNK: Windows Shell Link files are ubiquitous on any Windows drive; the 8-byte prefix
  (`4C 00 00 00` = size 76 + `01 14 02 00 00 00 00 00` = Shell Link CLSID) is distinctive
- PEM: extremely common in `~/.ssh/`, server configs, certificate stores

---

## Phase 104 — Signature Quality: Size Hints for New Formats (Planned)

**No new signatures — improves extraction accuracy for existing entries.**

Several formats added in Phases 100–103 produce oversized extractions because they
lack size hints and fall back to `max_size`. This phase adds size-hint walkers for
the highest-impact cases.

| Format | Size Hint Logic |
|--------|----------------|
| **CAB** | Cabinet file size is at bytes 8–11 (u32 LE) — read directly, same pattern as OLE2 |
| **Java Class** | Class file size is not embedded; keep max_size cap (50 MiB is already tight) |
| **DEX** | File size at bytes 32–35 (u32 LE) — embed as `SizeHint::Dex` |
| **WOFF2** | Total file length at bytes 8–11 (u32 BE) — embed as `SizeHint::Woff2` |
| **OTF** | No embedded size; use TTF size-hint walker (numTables × table sizes) |
| **DjVu** | FORM chunk size at bytes 4–7 (u32 BE) + 8 — same pattern as RIFF/AIFF |
| **OpenEXR** | No simple header size field; rely on footer search or max_size |

**Changes:**
- `crates/ferrite-carver/src/size_hint.rs` — add `SizeHint::Dex`, `SizeHint::Woff2`,
  `SizeHint::Djvu` (reuse RIFF size pattern), `SizeHint::Cab`
- `config/signatures.toml` — add `size_hint_kind` to CAB, DEX, WOFF2, DjVu entries
- Tests: each size hint reads correct value from a synthetic byte slice

---

## File Type Coverage After All Phases

After phases 100–104, signature count reaches **~140**:

| Category | Formats | Count |
|---|---|---|
| Images (raster) | JPEG×4, PNG, GIF, BMP, TIFF×2, WebP, ICO, PSD, PSB, DjVu, EXR, JP2, PCX, BPG, XCF | **18** |
| RAW Photos | ARW, CR2, CR3, CRW, DCR, NEF, RW2, RAF, MRW, SR2, ORF, PEF, HEIC×2, X3F | **15** |
| Video | MP4, MOV, M4V, 3GP, MKV, WebM, AVI, WMV, FLV, MPEG-PS, RM, SWF×3, TS, M2TS, WTV, DPX | **17** |
| Audio | MP3, WAV, FLAC, OGG, M4A, MIDI, AIFF, WavPack, APE, AU, AAC×2 | **12** |
| Archives | ZIP, RAR, 7-Zip, GZip, BZip2, XZ, ISO, TAR, CAB, LZH, JAR | **11** |
| Documents | PDF, XML, HTML, RTF, VCF, ICS, EML, EPUB, ODT, CDR, TTF, OTF, WOFF, WOFF2, CHM, Blender, InDesign, PHP, Shebang | **19** |
| Office & Email | ZIP-Office (OOXML), OLE2, PST, MSG | **4** |
| System / Exec | SQLite, EVTX, EXE, ELF, VMDK, REGF, VHD, VHDX, QCOW2, Mach-O, KDBX, KDB, E01, PCAP×2, DMP, plist, LUKS, DICOM | **18** |
| Developer | Java Class, Android DEX, Python `.pyc`, HDF5, FITS, Parquet | **6** |
| Forensic | VDI, AFF, LNK, Windows Prefetch, EVT, PEM | **6** |
| Fonts | TTF, OTF, WOFF, WOFF2 | **4** |

**Total: ~140 signatures across ~120 format families**

This places Ferrite at roughly **35% of PhotoRec's format family count**, with
significantly deeper per-format validation — pre-validators for every signature,
TIFF/ISOBMFF/OGG/SQLite/PNG/GIF/PDF size-hint walkers, and forensic-grade
false-positive rejection that PhotoRec does not match.
