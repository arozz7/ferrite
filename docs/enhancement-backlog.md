# Ferrite — Enhancement & Gap Backlog

**Generated:** 2026-03-30
**Source:** Full forensic engineering audit of Phases 0–112 (140 sigs, 1098+ tests)
**Purpose:** Authoritative prioritised backlog for post-Phase-112 enhancements.
Items are ordered by **forensic value × effort** — highest payoff first.

---

## How to Read This Document

Each item has:
- **ID** — stable reference (e.g. `ENH-01`) for linking from phase logs
- **Category** — Reliability Fix / Feature Gap / Workflow / Polish
- **Effort** — S (hours), M (1–2 days), L (3–5 days), XL (week+)
- **Status** — `open` until a phase commit resolves it, then `done (Phase N)`

---

## Tier 1 — Reliability Fixes
> These are latent bugs or silent failure modes. They do not require new features —
> they prevent existing features from behaving unexpectedly on a real recovery job.

### ENH-01 · Mapfile integrity check on load
| Field | Value |
|-------|-------|
| **Category** | Reliability Fix |
| **Effort** | S |
| **Status** | open |
| **File** | `ferrite-imaging/src/mapfile_io.rs` |

**Problem:** The GNU ddrescue-compatible mapfile has no integrity field. If the mapfile
is partially written during a crash (power loss mid-session), the corrupted file is
silently accepted and resume starts from incorrect block state — potentially re-imaging
already-recovered sectors and skipping others.

**Fix:** Add a CRC32 or Adler-32 checksum in the mapfile header comment block
(`# crc32: XXXXXXXX`). Validate on load; warn user and offer `--force-resume` if invalid.
Existing GNU ddrescue tools ignore unknown comment lines, so the format stays compatible.

---

### ENH-02 · NTFS MFT scan cap — make configurable
| Field | Value |
|-------|-------|
| **Category** | Reliability Fix |
| **Effort** | S |
| **Status** | open |
| **File** | `ferrite-filesystem/src/ntfs.rs:29-30` |

**Problem:** `MAX_SCAN_RECORDS: u64 = 65_536` hard-caps MFT traversal. A 16 TiB NTFS
volume with 4 KiB clusters can hold >1 M records. Large drives silently return an
incomplete file listing — the user sees fewer deleted files than actually exist, with no
warning.

**Fix:** Raise default to `1_048_576` (1 M) and expose as `NtfsConfig::max_scan_records`
fed from `ferrite-core::Config`. Add a TUI status line showing `Scanned N / M records`
so the user knows when a cap is hit.

---

### ENH-03 · Sparse output — validate destination FS capability
| Field | Value |
|-------|-------|
| **Category** | Reliability Fix |
| **Effort** | S |
| **Status** | open |
| **File** | `ferrite-imaging/src/engine.rs:141-150`, `ferrite-imaging/src/sparse.rs` |

**Problem:** When `sparse_output: true`, `enable_sparse()` calls `FSCTL_SET_SPARSE`
(Windows) or uses seek holes (Linux). Both calls are silently ignored when the
destination filesystem does not support sparse files (FAT32, some SMB shares, older
exFAT). The resulting image file is full-sized, but the user believes it is sparse —
which affects expected free-space accounting and can fill the destination unexpectedly.

**Fix:** After calling `enable_sparse()`, perform a small probe write-and-query to
confirm sparseness is active. If not, log a `WARN` via `tracing`, set
`ImagingStatus::SparseUnavailable`, and surface an amber notice in the config panel.

---

### ENH-04 · HFS+ — surface "parser unavailable" in TUI
| Field | Value |
|-------|-------|
| **Category** | Reliability Fix |
| **Effort** | S |
| **Status** | open |
| **File** | `ferrite-filesystem/src/lib.rs:412`, `ferrite-tui/src/screens/files/` |

**Problem:** `FilesystemType::HfsPlus` is detected and displayed in the Partitions and
Files tabs, but `open_filesystem()` returns `Err(UnknownFilesystem)`. The error surfaces
as a generic "failed to open filesystem" message. Users assume their HFS+ volume is too
corrupted to read, when it is simply not yet implemented.

**Fix:** Match `FilesystemType::HfsPlus` explicitly in the Files tab and show:
`HFS+ detected — parser not yet implemented. Use carving (Tab 5) to recover files.`
This sets correct expectations and directs the user to the working recovery path.

---

### ENH-05 · FAT32 cluster index — add defensive bounds check
| Field | Value |
|-------|-------|
| **Category** | Reliability Fix |
| **Effort** | S |
| **Status** | open |
| **File** | `ferrite-filesystem/src/fat32.rs:97` |

**Problem:** `self.data_offset + (cluster as u64 - 2) * self.cluster_size` — if a
corrupted FAT entry ever passes a value `< 2`, the subtraction wraps to a huge u64,
causing a nonsensical LBA read deep into the image. The FAT spec reserves 0–1 but does
not guarantee a corrupt volume will honour that.

**Fix:** Add `if cluster < 2 { return Err(FatError::InvalidCluster(cluster)); }` before
the arithmetic. Low effort, eliminates a class of panic/nonsensical-read on corrupted
volumes.

---

## Tier 2 — High-Value Feature Gaps
> Missing functionality that materially affects real-world recovery completeness.

### ENH-06 · Incremental carving checkpoint / resume
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | M |
| **Status** | done (pre-existing) |
| **Files** | `ferrite-carver/src/engine.rs`, `ferrite-carver/src/scanner.rs` |

**Note:** Already fully implemented. `last_scanned_byte` is saved in `CarvingSession`
and restored as `resume_from_byte` in `session_ops.rs`; `input.rs` advances the scan
start to that offset on resume, with a short-circuit when the position is past
end-of-device. No action needed.

**Original problem description (for reference):** The imaging engine has full GNU ddrescue-compatible checkpoint/resume.
Carving has no equivalent. On a 4 TiB drive a full carve takes 3–8 hours; if the user
closes the TUI mid-scan, the entire pass restarts from sector 0. Every discovered hit is
lost.

**Fix:** Persist a `.ferrite-carve` checkpoint file alongside the image (or device path
hash). Format: `offset_bytes,sig_id,hit_offset` — one line per confirmed hit, plus a
`scan_head` line recording how far the scanner has reached. On resume: replay confirmed
hits into the results list, advance `scan_start` to `scan_head`, skip duplicate offsets.
Checkpoint is flushed every `checkpoint_interval` seconds (configurable, default 30 s).

---

### ENH-07 · Encrypted volume detection
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | M |
| **Status** | done (Phase 118) |
| **Files** | `ferrite-filesystem/src/detect.rs`, `ferrite-tui/src/screens/partitions/` |

**Problem:** BitLocker, FileVault 2, VeraCrypt, and LUKS (already have a sig) all
produce partitions that look non-empty but are not recoverable by any parser. There is
no user warning — the Files tab simply fails to list any entries, which looks like
corruption rather than encryption.

**Fix:**
- BitLocker: detect `-FVE-FS-` OEM ID at offset 3 of boot sector
- FileVault 2: detect Core Storage volume header (`CS` magic at LBA 0)
- VeraCrypt: high-entropy first 512 bytes with no recognisable filesystem header → flag
  as "possible encrypted volume"
- Surface as `FilesystemType::Encrypted(EncryptionHint)` with a TUI message:
  `Encrypted volume detected (BitLocker). Decryption key required before recovery.`

---

### ENH-08 · NTFS Alternate Data Stream enumeration
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | M |
| **Status** | done (Phase 119) |
| **File** | `ferrite-filesystem/src/ntfs.rs` |

**Problem:** The NTFS parser only reads the unnamed `$DATA` attribute from each MFT
record. Named `$DATA` streams (NTFS Alternate Data Streams) are silently skipped. ADS
are used for:
- `Zone.Identifier` (Internet origin metadata — forensically significant)
- Browser/email attachment download sources
- Malware hiding executable payloads
- Thumbnail data

**Fix:** In the MFT attribute loop, collect all `$DATA` attributes (type 0x80). For
each named stream, produce an additional `FileEntry` with name format
`filename.ext:streamname`. Include in `enumerate_files()` output with a distinguishing
`FileKind::AlternateDataStream` variant.

---

### ENH-09 · S.M.A.R.T. pending-sector correlation with imaging bad-block map
| Field | Value |
|-------|-------|

| **Category** | Feature Gap |
| **Effort** | M |
| **Status** | done (Phase 121) |
| **Files** | `ferrite-smart/src/lib.rs`, `ferrite-imaging/src/engine.rs` |

**Problem:** `ferrite-smart` reads S.M.A.R.T. attributes including `Reallocated_Sector_Ct`
and `Current_Pending_Sector` but these counts are never correlated with the imaging
bad-block map. A technician must manually cross-reference.

**Fix:**
1. Expose `SmartData::pending_lbas() -> Vec<u64>` where available (some drives expose
   this via `SMART_READ_LOG` page 0x94 or vendor extensions)
2. Pre-populate the mapfile with those LBAs marked as `?` (unstable) before pass 1
3. In the Health tab, add a "Cross-reference with mapfile" action that shows how many
   SMART-reported bad sectors were also unreadable during imaging — confirming SMART
   data accuracy

---

### ENH-10 · Post-extraction file integrity (SHA-256 + optional NSRL lookup)
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | M |
| **Status** | done (Phase 118) |
| **Files** | `ferrite-carver/src/carver_io.rs`, `ferrite-tui/src/screens/carving/` |

**Problem:** Carved files are written to disk with no hash recorded. There is no way to
later confirm a file was not modified post-extraction, which breaks forensic
chain-of-custody. Additionally, NSRL (National Software Reference Library) hash lookup
would immediately identify known-good system files (skip) and known-bad malware hashes
(flag).

**Fix:**
1. After each file extraction, compute SHA-256 and write a `.sha256` sidecar alongside
   (consistent with the existing imaging hash sidecar pattern in `ferrite-imaging::hash`)
2. Add `verify_extractions: bool` config flag; when enabled, re-read and re-hash after
   write to confirm storage integrity
3. (Optional, Tier 3) Provide `nsrl_db_path` config option; lookup hash against offline
   NSRL RDS database (SQLite export) and annotate result as `[KNOWN-GOOD]` / `[KNOWN-BAD]`

---

### ENH-11 · Forensic artifact confidence scoring
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | M |
| **Status** | done (Phase 119) |
| **File** | `ferrite-artifact/src/lib.rs`, scanner modules |

**Problem:** The artifact scanner reports all regex hits at equal weight. Common
false-positive patterns:
- CC# in binary data (often memory dumps containing non-card integers that pass Luhn)
- Email addresses in code/config (often placeholder `@example.com` addresses)
- URLs in compiled binaries (often internal resource identifiers)

**Fix:**
- **CC#:** Luhn check is already present; add surrounding-context entropy check — if the
  16 digits are embedded in high-entropy binary, confidence = Low
- **Email:** Check domain TLD validity; flag `@example.com`, `@localhost`, etc. as
  Low confidence
- **URL:** Distinguish `http://` in plain text vs. embedded in binary; score accordingly
- Expose `confidence: Confidence { High, Medium, Low }` on `ArtifactHit`
- TUI: colour-code by confidence; default filter to `Medium+`

---

## Tier 3 — Workflow & UX Enhancements
> These do not change what can be recovered, but meaningfully improve how a
> technician interacts with the tool during a real engagement.

### ENH-12 · Partition disk-map visualisation in TUI
| Field | Value |
|-------|-------|
| **Category** | Workflow |
| **Effort** | S |
| **Status** | done (Phase 119) |
| **File** | `ferrite-tui/src/screens/partitions/` |

**Problem:** Reconstructed partitions are shown as a table of LBA ranges. There is no
visual representation of where partitions sit on the disk, making it hard to spot
overlapping ranges, unpartitioned gaps, or partition-table corruption.

**Fix:** Add a horizontal bar diagram below the partition table — each partition rendered
as a coloured block proportional to its LBA span. Gaps shown as hatched/dark regions.
Overlaps highlighted in red. `ratatui::widgets::Gauge` or a custom `Canvas` block is
sufficient.

---

### ENH-13 · Empty device / zero-size device warning in carving
| Field | Value |
|-------|-------|
| **Category** | Workflow |
| **Effort** | S |
| **Status** | done (Phase 118) |
| **File** | `ferrite-carver/src/scanner.rs:147` |

**Problem:** When `device_size == 0`, the scanner returns `Ok(())` with no error, no
log message, and no UI feedback. The carving progress bar sits at 0% indefinitely with
no indication of why.

**Fix:** Log `warn!("device reports zero size — aborting carve")` and propagate a
`CarveError::ZeroSizeDevice` to the TUI, which displays an amber status message:
`Device reported size 0. Check device selection.`

---

### ENH-14 · Text-block language detection
| Field | Value |
|-------|-------|
| **Category** | Workflow |
| **Effort** | S |
| **Status** | done (Phase 120) |
| **File** | `ferrite-textcarver/src/lib.rs` |

**Problem:** The text carver produces text blocks with `TextKind` classification but no
language identification. A 10 GB raw text scan result containing English, Chinese, and
Russian blocks requires manual inspection to locate relevant material.

**Fix:** Add the `whatlang` crate (pure Rust, no deps, < 1 ms per block). After
extracting a text block meeting the minimum-length threshold, call
`whatlang::detect(text)` and store `lang: Option<Lang>` on `TextHit`. Expose as a
filter in the Text Scan tab (filter dropdown: All / English / CJK / Cyrillic / Other).

---

### ENH-15 · Sector read-verify mode (anti-glitch)
| Field | Value |
|-------|-------|
| **Category** | Workflow |
| **Effort** | S |
| **Status** | done (Phase 120) |
| **File** | `ferrite-imaging/src/engine.rs`, `ferrite-core/src/config.rs` |

**Problem:** On drives with intermittent read errors (head flutter, marginal sectors),
a sector may return different data on consecutive reads. The current engine reads each
sector once and trusts the result.

**Fix:** Add `verify_reads: bool` (default: `false`) and `verify_passes: u8` (default: `2`)
to `ImagingConfig`. When enabled, read each sector `verify_passes` times during the
copy pass; if results differ, mark as `?` (unstable) in the mapfile and log a warning.
This also detects controller glitches and cable signal issues.

---

## Tier 4 — Future / Research
> Architecturally larger items. Require their own scoped planning session.

### ENH-16 · HFS+ full parser
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | XL |
| **Status** | open |

Full implementation of the HFS+ `FilesystemParser` trait. Detection (`0x482B`/`0x4858`)
already exists. Requires B-tree catalog file traversal, Unicode normalisation (NFD),
and HFS+ extended attributes. HFSX case-sensitivity variant must also be handled.

---

### ENH-17 · LVM / RAID / dynamic disk volume sets
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | XL |
| **Status** | open |

Multi-disk/multi-partition volume groups treated as unified logical volumes. Required
for Windows Storage Spaces, Linux LVM, and mdadm software RAID recovery. Needs a new
`ferrite-volume` crate with a `VolumeAssembler` trait.

---

### ENH-18 · Filesystem journal / log replay
| Field | Value |
|-------|-------|
| **Category** | Feature Gap |
| **Effort** | XL |
| **Status** | open |

Parse NTFS `$LogFile` and ext4 `jbd2` journal to reconstruct filesystem state at the
point of crash. Enables answering "what was the filesystem doing when it failed?" and
recovering files that were in-flight during a power-cut.

---

## Completion Checklist

| ID | Title | Tier | Effort | Status |
|----|-------|------|--------|--------|
| ENH-01 | Mapfile CRC integrity check | 1 | S | done (Phase 117) |
| ENH-02 | NTFS MFT scan cap — configurable | 1 | S | done (Phase 117) |
| ENH-03 | Sparse output — FS capability probe | 1 | S | done (Phase 117) |
| ENH-04 | HFS+ "parser unavailable" TUI message | 1 | S | done (Phase 117) |
| ENH-05 | FAT32 cluster bounds check | 1 | S | done (Phase 117) |
| ENH-06 | Incremental carving checkpoint/resume | 2 | M | done (pre-existing — Phase 42/65 infrastructure) |
| ENH-07 | Encrypted volume detection | 2 | M | done (Phase 118) |
| ENH-08 | NTFS Alternate Data Stream enumeration | 2 | M | done (Phase 119) |
| ENH-09 | S.M.A.R.T. pending-sector correlation | 2 | M | done (Phase 121) |
| ENH-10 | Post-extraction SHA-256 + NSRL lookup | 2 | M | done (Phase 118) |
| ENH-11 | Artifact confidence scoring | 2 | M | done (Phase 119) |
| ENH-12 | Partition disk-map visualisation | 3 | S | done (Phase 119) |
| ENH-13 | Zero-size device warning in carving | 3 | S | done (Phase 118) |
| ENH-14 | Text-block language detection | 3 | S | done (Phase 120) |
| ENH-15 | Sector read-verify mode | 3 | S | done (Phase 120) |
| ENH-16 | HFS+ full parser | 4 | XL | open |
| ENH-17 | LVM/RAID/dynamic disk volume sets | 4 | XL | open |
| ENH-18 | Filesystem journal/log replay | 4 | XL | open |

---

*Last updated: 2026-03-30 — audit by forensic engineering review of Phases 0–112.*
