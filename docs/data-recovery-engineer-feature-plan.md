# Ferrite — Senior Data Recovery Engineer Review

## Executive Summary

Ferrite has a solid, well-structured foundation. The five-pass imaging algorithm
is architecturally sound, the trait-based abstraction layer is clean, and the
test coverage is notably good for a greenfield tool. However, several gaps would
prevent it from being trusted on a real recovery engagement today. The issues
fall into three tiers: **critical blockers** that can cause data loss or drive
damage, **major gaps** that make professional use impractical, and **workflow
improvements** that differentiate a hobbyist tool from a professional one.

---

## Critical Blockers

### 1. No per-read timeout — drive will hang indefinitely on bad sectors

**File:** `crates/ferrite-blockdev/src/windows.rs` → `read_at()`

```rust
let ok = unsafe {
    ReadFile(self.handle, buf.as_mut_slice().as_mut_ptr().cast(),
        buf.len() as u32, &mut bytes_read, &mut ov)
};
```

`ReadFile` in synchronous mode on a failing HDD will block for however long the
drive's internal firmware decides to retry — typically 7 to 60 seconds per
sector on a consumer drive without TLER configured. The imaging thread is blocked
for that entire duration. On a drive with 500 bad sectors, that is potentially
8+ hours of just waiting, during which:

- The drive's heads are hammering against unreadable media
- The drive's internal temperature is rising
- The PCB and spindle motor are under continuous stress

**What professional tools do:** Force ERC/TLER via S.M.A.R.T. attribute write
(ID 193 / 0xC1 on many WD/Seagate drives) to cap the drive's internal retry time
to 7 seconds, or use OS-level read timeouts. At minimum, the imaging engine needs
a configurable per-read timeout with the ability to abort the `ReadFile` call and
mark the sector as bad after N milliseconds.

---

### 2. No temperature monitoring during imaging

The engine has no concept of thermal state. Running a failing drive at sustained
high I/O for hours in a warm environment routinely kills drives that could have
been saved with pauses. Professional recovery includes:

- Read drive temperature via S.M.A.R.T. every 30–60 seconds during imaging
- Pause imaging if temperature exceeds a configurable threshold (typically 55°C for HDDs)
- Resume automatically after cool-down

Currently there is no integration between `ferrite-smart` and `ferrite-imaging`.
They are completely siloed.

---

### 3. File browser can see files but cannot extract them

**File:** `crates/ferrite-filesystem/src/lib.rs`

```rust
pub trait FilesystemParser: Send + Sync {
    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64>;
    // ...
}
```

`read_file()` is fully implemented in NTFS, FAT32, and ext4. The TUI file browser
renders directory trees but the `e` (extract) key does not exist on that screen.
A user can browse to a deleted file and have no way to actually get it out. This
is the most fundamental operation in file-level recovery and it is unreachable
from the UI.

---

### 4. ext4 extents are not supported — most real ext4 files are inaccessible

**File:** `crates/ferrite-filesystem/src/ext4.rs`

The ext4 parser reads direct blocks and single-indirect blocks only. Since Linux
kernel 2.6.23 (2007), ext4 defaults to the extents tree (inode flag `0x80000`).
Any file written by a modern Linux system uses extents rather than the legacy
block map. This means the vast majority of files on real-world ext4 volumes will
either read 0 bytes or return garbage data silently. The filesystem is essentially
unusable for recovery from modern Linux drives without extent support.

---

## Major Gaps

### 5. No read-rate monitoring — cannot detect a drive entering a failure spiral

The `ProgressUpdate` struct tracks bytes and time but the imaging engine has no
concept of current read throughput. A healthy 3.5" HDD reads at 80–150 MB/s.
When that rate drops to 1 MB/s mid-image, it is a clinical sign that the drive's
heads are struggling. Professional tools:

- Track a rolling average read rate
- Alert when rate drops below a threshold (e.g., < 5 MB/s sustained for 30 seconds)
- Offer to skip ahead to the next partition boundary rather than grinding through a bad zone

Without this, operators cannot distinguish "imaging slowly but normally" from
"drive is failing right now."

---

### 6. No image integrity verification pass

After imaging completes, there is no way to verify the output. Professional
recovery requires:

- MD5 / SHA256 hash of the output image — for forensic chain of custody and to
  confirm the file was written correctly
- Optional verification read-back — re-read the image file and compare a sample
  of sectors against the source to confirm no filesystem corruption of the output

Currently, if the output filesystem fills up mid-image (write fails silently),
Ferrite will report "Complete" with no indication that the last N GiB are missing.

---

### 7. No raw sector hex viewer

When `FilesystemParser::open_filesystem()` fails — corrupted superblock, unknown
filesystem, encrypted volume — the only tool an engineer has is a raw sector hex
dump. Being able to navigate to LBA N and view its raw bytes is non-negotiable in
professional recovery. It is also essential for manual MBR/GPT repair, locating
superblock backups, and inspecting carve hit context.

This is a major missing screen: a hex viewer that accepts an LBA or byte offset,
reads one sector (or N sectors), and displays it as a classic hex dump
(`offset  hex bytes  ASCII`).

---

### 8. Carver has no fragmentation awareness

**File:** `crates/ferrite-carver/src/scanner.rs`

The current carver assumes all files are contiguous on disk. This is valid for
simple media (SD cards, USB drives, FAT32 volumes) but not for:

- HDDs with heavy fragmentation
- Any file that grew over time (documents, mailboxes, databases)
- Volumes that have been heavily written to before the failure

A fragmented JPEG will be extracted as a truncated file — often just the first
512 KB of a 3 MB image — with no indication that it is incomplete. At minimum,
extracted files should be validated after extraction so the operator knows which
hits are complete vs. truncated.

---

### 9. Missing critical file types for carving

The 10 built-in signatures cover common media but miss high-value recovery targets:

| Missing Type | Header | Why Important |
|---|---|---|
| SQLite database | `53 51 4C 69 74 65 20 66 6F 72 6D 61 74 20 33 00` | Browser history, iOS/Android backups, many apps |
| Windows Event Log (.evtx) | `45 4C 46 49 4C 45 00` | Forensic / incident response |
| Email (.eml) | `46 72 6F 6D 20` | High-value personal data |
| Outlook PST | `21 42 44 4E` | Entire email archives |
| WAV audio | `52 49 46 46` (then `57 41 56 45` at +8) | Audio recordings |
| AVI video | `52 49 46 46` (then `41 56 49 20` at +8) | Video recordings |
| MKV / WebM | `1A 45 DF A3` | Modern video |
| Word/Excel/PPT (.docx etc.) | `50 4B 03 04` + content type check | Same ZIP header, needs inner validation |
| Executable (.exe / PE) | `4D 5A` | Software recovery |
| VMDK / VHD / VHDX | Unique headers | Virtual disk images |

---

### 10. S.M.A.R.T. bad sector LBA list not fed to imaging

Modern `smartctl -l selftest` and `smartctl -l error` output includes the LBA
addresses of known bad sectors. This information should pre-populate the
mapfile's `BadSector` blocks before imaging starts, so Pass 1 skips those sectors
entirely and jumps straight to scraping them — saving significant time and
reducing further mechanical stress on those locations.

---

### 11. No LBA range selection for partial imaging

The imaging engine always images the entire device. Professional recovery
frequently needs:

- Image only a specific partition (e.g., skip the first healthy system partition,
  image only the data partition)
- Resume from a specific LBA (e.g., after hardware work, start from the area
  that was previously inaccessible)
- Reverse imaging (image the end of the drive first — useful when the beginning
  has severe bad sectors and the data is likely in the last 50%)

`ImagingConfig` has `output_path` and `mapfile_path` but no `start_lba` or
`end_lba`. Adding these to the config and initialising the mapfile with the LBA
range as `NonTried` (and everything outside as `Finished`) would handle this.

---

### 12. No session persistence

A real recovery job for a 4 TB drive takes 12–48 hours. The operator needs to
close Ferrite, come back, and have it remember:

- Which device was selected
- The destination and mapfile paths
- Which carving signatures were enabled
- The last known S.M.A.R.T. verdict

A simple JSON session file (`ferrite-session.json`) saved on exit and loaded on
startup would cover this entirely.

---

### 13. No structured recovery report

Every professional recovery engagement ends with a written report for the client.
Ferrite has all the data needed to generate one automatically:

- Device info (model, serial, capacity, firmware)
- S.M.A.R.T. verdict and key attribute values at time of imaging
- Final bad sector count and percentage of device recovered
- Partition table found
- Filesystems detected, file counts, deleted file counts
- Carving results: hits per type, files successfully extracted

A `ferrite report --output report.html` subcommand (or an Export key in the TUI)
would produce this from the mapfile + S.M.A.R.T. cache.

---

## Workflow Improvements

### 14. Write-blocker verification before imaging

Before starting any imaging session, Ferrite should confirm the source device
handle is truly read-only by attempting a zero-byte write at offset 0 and
asserting it fails with a permission error. This is a standard step in forensic
imaging workflows to prove that the write blocker (hardware or software) is
functioning.

---

### 15. Duplicate carve hit suppression

After a long carve scan, the same logical file often produces multiple hits
(e.g., JPEG thumbnails embedded in NTFS `$DATA` streams at different offsets, or
a file cached in the page file at another location). A content hash (SHA1 of the
first 4 KiB of each extracted file) should be used to deduplicate the hit list
before presenting results to the operator.

---

### 16. Imaging: configurable block size per pass

The 512 KiB default copy block is reasonable for Pass 1, but Scrape (Pass 4)
should use exactly one sector (512 B or 4 KiB) per read — not 512 KiB. Currently
`copy_block_size` is used for all passes. The Trim and Scrape passes should use a
sector-sized read to avoid reading healthy adjacent sectors when only probing one
bad sector.

**File:** `crates/ferrite-imaging/src/engine.rs` — the scrape pass already reads
sector-by-sector but uses the same `copy_block_size` buffer, which means it
requests a 512 KiB aligned read for every sector even during scraping.

---

### 17. S.M.A.R.T. — real-time temperature overlay during imaging

The S.M.A.R.T. and Imaging screens are separate. During a long imaging session,
the operator should be able to see the current drive temperature on the Imaging
screen without switching tabs. A one-line temperature indicator (queried every 60
seconds via a background thread) would be sufficient.

---

### 18. Carving: show extraction status per hit

After running `e` to extract a hit, the result (bytes written, filename,
success/error) is logged but not shown in the TUI. The hit list should update the
entry with a status indicator:
- `✓ 2.1 MiB → ferrite_jpg_1769472.jpg`
- `✗ read error at 0x1a2300`

---

### 19. Partition Analysis: export recovered partition table

The `PartitionTable` produced by the signature scanner is only displayed. There
should be an option to write a new MBR or GPT to the image file (not the source
device) based on the recovered layout. This would allow the image file to be
mounted by the OS for normal file access without needing Ferrite's filesystem
parsers.

---

### 20. exFAT and HFS+ detection

exFAT (`EXFAT   ` at offset 3) is ubiquitous on SD cards, camera memory, and USB
drives over 32 GB. It is arguably the most common filesystem on removable media
being brought to a recovery technician. HFS+ (`48 2B` at offset 1024) covers the
entire macOS install base prior to APFS. Neither is detected or parsed. At
minimum, both should be detected with a clear unsupported message rather than
silently returning `FilesystemType::Unknown`.

---

## Priority Matrix

| Priority | Item | Effort | Recovery Impact |
|---|---|---|---|
| 🔴 Critical | Per-read timeout (prevent drive thrashing) | High | Prevents physical damage |
| 🔴 Critical | ext4 extents support | High | Unlocks all modern Linux drives |
| 🔴 Critical | File extraction from browser | Low | Core use-case currently unreachable |
| 🟠 High | Temperature monitoring during imaging | Medium | Prevents heat-related failures |
| 🟠 High | Read rate monitoring + alert | Medium | Early warning of drive failure spiral |
| 🟠 High | Image integrity hash (SHA256) | Low | Forensic integrity |
| 🟠 High | LBA range selection for imaging | Medium | Targeted partition recovery |
| 🟠 High | SQLite + PST + WAV carving signatures | Low | High-value file types |
| 🟡 Medium | Sector hex viewer | Medium | Manual recovery fallback |
| 🟡 Medium | S.M.A.R.T. bad LBA → mapfile pre-population | Medium | Saves imaging time |
| 🟡 Medium | Carve hit validation (integrity check) | Medium | Eliminates false positives |
| 🟡 Medium | Session persistence | Low | Usability for long jobs |
| 🟡 Medium | Recovery report export | Medium | Professional deliverable |
| 🟡 Medium | exFAT + HFS+ detection (even if read-only) | Low | Avoids silent failure |
| 🟢 Low | Reverse imaging option | Low | Advanced technique |
| 🟢 Low | Configurable block size per pass | Low | Efficiency refinement |
| 🟢 Low | Duplicate carve hit suppression | Medium | Cleaner results |
| 🟢 Low | Partition table export to image file | Medium | Mount without Ferrite |

---

## Implementation Plan — Recovery Engineering Improvements

### Phase A — Critical, Low Effort

**A1. File extraction in File Browser**
- Add `e` key to `ferrite-tui/src/screens/file_browser.rs`
- Call `parser.read_file(entry, &mut File::create(...))` in a background thread
- Show per-entry extraction status in the hit list (`✓ 1.2 MiB saved` / `✗ read error`)
- Output filename: `ferrite_extract_<sanitised_name>`

**A2. Additional carving signatures**
- Add 8 new entries to `config/signatures.toml`: SQLite, WAV, AVI, MKV, EML, PE/EXE, VMDK, FLAC
- No code changes — signatures are `include_str!`-embedded at compile time

**A3. exFAT + HFS+ detection**
- Extend `ferrite-filesystem/src/lib.rs` `detect_filesystem()` with magic checks
- Return new `FilesystemType` variants `ExFat` and `HfsPlus` with a clear
  "not yet supported — detected only" message in the browser

**A4. SHA256 image integrity hash**
- Add `sha2` crate to `ferrite-imaging`
- Stream the hash alongside writes in the imaging engine
- Emit the hex digest in the final `ProgressUpdate` (new `image_sha256: Option<String>` field)
- Display on the Imaging screen when status is `Complete`

### Phase B — Critical, Higher Effort

**B1. Per-read timeout via overlapped I/O (Windows)**
- Reopen the device handle with `FILE_FLAG_OVERLAPPED` in `ferrite-blockdev/src/windows.rs`
- Replace the synchronous `ReadFile` + OVERLAPPED-offset pattern with true async
  overlapped I/O: `ReadFile` → `GetOverlappedResultEx(timeout_ms)` → `CancelIo` on timeout
- Add `BlockDeviceError::Timeout` variant so the imaging engine marks the sector
  as `BadSector` immediately rather than waiting

### Files Modified

| File | Change |
|---|---|
| `config/signatures.toml` | +8 new signatures |
| `crates/ferrite-blockdev/src/windows.rs` | Overlapped I/O + timeout |
| `crates/ferrite-blockdev/src/error.rs` | Add `Timeout` variant |
| `crates/ferrite-filesystem/src/lib.rs` | exFAT + HFS+ detection |
| `crates/ferrite-imaging/src/progress.rs` | `image_sha256` field on `ProgressUpdate` |
| `crates/ferrite-imaging/src/engine.rs` | SHA256 streaming during writes |
| `crates/ferrite-imaging/Cargo.toml` | Add `sha2` |
| `crates/ferrite-tui/src/screens/file_browser.rs` | `e` key extraction + status display |
| `crates/ferrite-tui/src/screens/imaging.rs` | Show SHA256 digest on completion |
| `aiChangeLog/phase-08.md` | Change log |
