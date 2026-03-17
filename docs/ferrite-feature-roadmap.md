# Ferrite — Comprehensive Feature Roadmap
**Reviewed as of Phase 42 (43 signatures) — Status updated 2026-03-17**
*Senior Data Recovery & Digital Forensics Perspective*

---

## Executive Summary

Ferrite is architecturally mature and further along than the original audit indicated.
After a thorough code review (2026-03-17), all four "Critical Blockers" are resolved and
several major gap items are already implemented. The remaining work falls into two
categories:

1. **Signature coverage** — 43 signatures vs. PhotoRec's ~300 families; 12–18 high-value
   types are completely absent (WebP, EML, ELF, REGF, PSD, etc.).
2. **Remaining workflow features** — SHA-256 image hash, thermal guard during imaging,
   write-blocker verification, Quick Deleted-File Recovery mode, custom signatures,
   forensic artifact scanning.

The phases below are ordered by **risk and recovery impact**, not by complexity.

---

## Current State Audit

### Signatures Implemented (43 total)

| Category | Formats |
|---|---|
| Images | JPEG JFIF, JPEG Exif, PNG, GIF, BMP, TIFF LE, TIFF BE |
| RAW Photos | Sony ARW, Canon CR2, Nikon NEF, Panasonic RW2, Fujifilm RAF, Apple HEIC/HEIX |
| Video | MP4, MOV, M4V, 3GP, MKV, WebM, AVI, WMV, FLV, MPEG-PS |
| Audio | MP3, WAV, FLAC, OGG |
| Archives | ZIP (+ OOXML), RAR, 7-Zip |
| Documents | PDF, RTF, XML, HTML, OLE2 (DOC/XLS/PPT legacy) |
| Email / PIM | Outlook PST/OST, vCard (VCF), iCalendar (ICS) |
| System / DB | SQLite, Windows EVTX, PE Executable (.exe), VMDK |

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
| Drive temperature guard during imaging | ❌ Missing |
| SHA256 image integrity hash | ❌ Missing |
| Low read-rate alert (threshold + TUI warning) | ❌ Missing |
| Write-blocker verification | ❌ Missing |
| Quick Deleted-File Recovery mode | ❌ Missing — Phase 45b |
| Carve hit integrity validation | ❌ Missing |
| Duplicate hit suppression (content hash) | ❌ Missing |
| Custom user-defined signatures (TUI) | ❌ Missing |
| Forensic artifact scanning (email, URLs, CC#) | ❌ Missing |

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
| 45b | Quick Deleted-File Recovery mode | 🔴 Critical | Medium | ⬜ **Next** |
| 46 | Tier 1 signatures (10 new → 53 total) | 🟠 High | Low | ⬜ Todo |
| 47b | SHA256 image integrity hash | 🟠 High | Low | ⬜ Todo |
| 47c | Low read-rate alert + TUI warning | 🟠 High | Low | ⬜ Todo |
| 51b | SMART thermal guard during imaging | 🟡 Medium | Medium | ⬜ Todo |
| 52 | Tier 2 signatures (9 new → 62 total) | 🟡 Medium | Low | ⬜ Todo |
| 53 | Write-blocker verification | 🟡 Medium | Low | ⬜ Todo |
| 55 | Carve hit validation + dedup | 🟡 Medium | Medium | ⬜ Todo |
| 56 | Custom user signatures (TUI) | 🟢 Low | Medium | ⬜ Todo |
| 57 | Forensic artifact scanner | 🟢 Low | High | ⬜ Todo |
| 59 | PhotoRec Tier A quick wins (9 new sigs) | 🟠 High | Low | ⬜ Todo |
| 60 | PhotoRec Tier B batch 1 (13 new sigs) | 🟡 Medium | Low | ⬜ Todo |
| 61 | PhotoRec Tier B batch 2 (8 new sigs) | 🟡 Medium | Low | ⬜ Todo |
| 62 | Non-zero offset scan (ISO, DICOM, TAR) | 🟡 Medium | Medium | ⬜ Todo |
| 58 | APFS + exFAT full parser | 🟢 Low | Very High | ⬜ Stretch |

---

## File Type Coverage After All Phases

After all planned phases (46 + 52 + 59 + 60 + 61 + 62), signature count reaches **~93**:

| Category | Formats | Count |
|---|---|---|
| Images (raster) | JPEG×2, PNG, GIF, BMP, TIFF LE/BE, WebP, ICO, PSD, DPX, XCF, JP2, PCX | **13** |
| RAW Photos | ARW, CR2, CR3, CRW, DCR, NEF, RW2, RAF, MRW, SR2, ORF, PEF, HEIC×2, Sigma X3F | **15** |
| Video | MP4, MOV, M4V, 3GP, MKV, WebM, AVI, WMV, FLV, MPEG-PS, RealMedia, TS, M2TS, WTV | **14** |
| Audio | MP3, WAV, FLAC, OGG, AAC/M4A, MIDI, AIFF, WavPack, APE, AU, XZ (compressed) | **11** |
| Archives | ZIP, RAR, 7-Zip, GZip, BZip2, XZ, TAR, CAB (future), PAR2 | **9** |
| Documents | PDF, RTF, XML, HTML, OLE2, EML, EMLX, EPUB, ODT, InDesign, CorelDRAW, SWF | **12** |
| Email / PIM | PST/OST, VCF, ICS, MSG | **4** |
| System / Exec | EXE (PE), ELF, Mach-O | **3** |
| Forensic | REGF, EVTX, LNK (future), E01, PCAP, LUKS, Windows Dump | **7** |
| Database / Config | SQLite, KeePass 1.x, KeePass 2.x, Apple plist | **4** |
| Virtual Disk | VMDK, VHD, VHDX, QCOW2, ISO (Phase 62) | **5** |
| Fonts | TTF, WOFF | **2** |
| Medical / Science | DICOM (Phase 62), FITS | **2** |
| 3D / Design | Blender, CHM, Parchive | **3** |

**Total: ~104 signatures across ~93 format families**

This places Ferrite at roughly **31% of PhotoRec's format family count**, but with
significantly deeper per-format validation — pre-validators for every signature,
TIFF/ISOBMFF/OGG/SQLite size-hint walkers, and forensic-grade false-positive
rejection that PhotoRec does not match.
