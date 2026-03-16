# Ferrite — User Manual

**Version 0.1.0 · Pure-Rust Storage Diagnostics & Data Recovery**

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Prerequisites](#2-prerequisites)
3. [Building & Installing](#3-building--installing)
4. [Launching Ferrite](#4-launching-ferrite)
5. [Interface Overview](#5-interface-overview)
6. [Screen 1 — Drive Selection](#6-screen-1--drive-selection)
7. [Screen 2 — Health Dashboard](#7-screen-2--health-dashboard)
8. [Screen 3 — Imaging](#8-screen-3--imaging)
9. [Screen 4 — Partition Analysis](#9-screen-4--partition-analysis)
10. [Screen 5 — File Browser](#10-screen-5--file-browser)
11. [Screen 6 — File Carving](#11-screen-6--file-carving)
12. [Configuration Files](#12-configuration-files)
13. [Mapfile Format](#13-mapfile-format)
14. [Logging & Diagnostics](#14-logging--diagnostics)
15. [Permissions & Safety](#15-permissions--safety)
16. [Filesystem Coverage & Known Limitations](#16-filesystem-coverage--known-limitations)
17. [Troubleshooting](#17-troubleshooting)
18. [Glossary](#18-glossary)

---

## 1. Introduction

Ferrite is a terminal-based storage diagnostics and data recovery application written
in pure Rust.  It is designed to help users assess the health of failing drives,
create resilient byte-for-byte images, recover lost partition tables, browse
filesystem contents (including deleted files), and carve individual files from raw
disk images — all from a single interactive terminal UI.

### Core principles

| Principle | Detail |
|---|---|
| **Read-only source** | Ferrite never writes to the device being analysed or recovered |
| **Resilient imaging** | A five-pass algorithm (modelled after GNU ddrescue) retries bad sectors progressively |
| **ddrescue-compatible mapfiles** | Progress is saved in a format interchangeable with `ddrescue` |
| **Pure Rust** | No C library dependencies (libsmartmon, libparted, libtsk all rejected) |
| **Non-destructive** | Every operation on a source device is read-only; all output goes to separate files |

---

## 2. Prerequisites

### Required

| Requirement | Notes |
|---|---|
| **Rust 1.75+** | Install via [rustup.rs](https://rustup.rs). Only needed to build from source |
| **Administrator / root privileges** | Required to open raw block devices (`\\.\PhysicalDriveN` on Windows, `/dev/sdX` on Linux) |
| **`smartctl` 7.0+** | Required for the Health Dashboard only. Part of the [smartmontools](https://www.smartmontools.org) package. Must be on `PATH` |

### Optional

| Optional item | Effect if absent |
|---|---|
| Sufficient free disk space | Cannot save an image file or extracted carve hits |
| A mapfile from a previous run | Imaging starts from scratch; no resume capability |

### Platform support

| Platform | Block device access | S.M.A.R.T. |
|---|---|---|
| Windows 10 / 11 | `\\.\PhysicalDriveN` via `CreateFileW` + direct I/O | `smartctl --json -a` |
| Linux | `/dev/sdX`, `/dev/nvme0n1`, etc. | `smartctl --json -a` |

---

## 3. Building & Installing

```powershell
# Clone the repository
git clone https://github.com/arozz7/ferrite
cd ferrite

# Release build (optimised binary)
cargo build --release

# The binary is produced at:
#   Windows: target\release\ferrite.exe
#   Linux:   target/release/ferrite
```

To install system-wide on Linux:

```bash
cargo install --path .
```

To run tests and verify the build:

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

---

## 4. Launching Ferrite

### Windows (must be run as Administrator)

```powershell
# Right-click PowerShell → "Run as administrator", then:
.\target\release\ferrite.exe
```

### Linux (must be run as root or with appropriate udev rules)

```bash
sudo ./target/release/ferrite
```

### Environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG=info` | Enable structured log output (written to stderr, below the TUI) |
| `RUST_LOG=ferrite_tui=debug` | Debug-level logs for the TUI crate only |
| `RUST_LOG=off` | Suppress all log output (default when not set) |

Log output does not interfere with the TUI; it is written to stderr and is only
visible after the TUI exits, or when piped/redirected.

---

## 5. Interface Overview

Ferrite uses a full-screen terminal interface built with **ratatui 0.29** and
**crossterm 0.28**.

```
┌─ Ferrite ────────────────────────────────────────────────────────────────┐
│  Drives  │  Health  │  Imaging  │  Partitions  │  Files  │  Carving     │  ← Tab bar
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│                        Screen content area                               │
│                                                                          │
├──────────────────────────────────────────────────────────────────────────┤
│  ↑/↓: navigate  Enter: select device  r: refresh list  Tab: next  q: quit│  ← Help bar
└──────────────────────────────────────────────────────────────────────────┘
```

### Tab bar

The tab bar at the top shows all six screens.  The currently active screen is
highlighted in **bold yellow**.

### Help bar

The bottom row shows context-sensitive key hints for the active screen.  It updates
automatically whenever the screen or mode changes.

### Global key bindings

These work from every screen at all times:

| Key | Action |
|---|---|
| `Tab` | Move to the next screen (wraps from Carving back to Drives) |
| `Shift-Tab` | Move to the previous screen (wraps from Drives back to Carving) |
| `q` | Quit Ferrite (suppressed while a text-input field is active) |

### Recommended workflow

```
Drive Selection → Health Dashboard → Imaging → Partition Analysis → File Browser → Carving
```

Start by selecting a device on the Drives screen.  This propagates the device
reference to all other screens so they are ready to operate immediately.

---

## 6. Screen 1 — Drive Selection

**Purpose:** Discover all block devices on the system, inspect their metadata, and
designate one as the active device for all subsequent operations.

### Layout

```
┌─ Drive Selection — press r to refresh ─────────────────────────────────┐
│   #  Path                      Model                Serial       Size   │
│ ▶ 0  \\.\PhysicalDrive0        WDC WD40EZRZ-00W     WD-WCC7...  3.6 GiB│
│   1  \\.\PhysicalDrive1        Samsung 860 EVO      S3EVNX...  500.0GiB│
└─────────────────────────────────────────────────────────────────────────┘
```

### How device enumeration works

**Windows:** Ferrite probes `\\.\PhysicalDrive0` through `\\.\PhysicalDrive31`.
Any path that opens successfully with `CreateFileW` (read-only, `FILE_FLAG_NO_BUFFERING`)
is included.  Model and serial are queried via `IOCTL_STORAGE_QUERY_PROPERTY`.
Size is queried via `IOCTL_DISK_GET_LENGTH_INFO`.

**Linux:** `enumerate_devices()` reads the platform block-device list and returns
the device paths.  Opening each path uses `O_RDONLY | O_DIRECT`.

Enumeration runs automatically the first time the screen is displayed.  It happens
on a background thread so the UI remains responsive.

### Columns

| Column | Content |
|---|---|
| `#` | Device index (0-based) |
| `Path` | Raw device path (e.g. `\\.\PhysicalDrive0` or `/dev/sda`) |
| `Model` | Drive model string from the device descriptor |
| `Serial` | Serial number from the device descriptor |
| `Size` | Total device capacity (displayed in GiB, MiB, or bytes) |

If a device cannot be opened (typically a permission error), the Model column
shows `"open failed (admin required?)"` and no size is displayed.

### Key bindings

| Key | Action |
|---|---|
| `↑` / `↓` | Move the selection highlight up or down |
| `Enter` | Open the selected device and set it as the active device for all screens |
| `r` | Re-enumerate all block devices (refreshes the list) |

### What happens after selecting a device

Pressing `Enter` on a device calls `WindowsBlockDevice::open()` (or the Linux
equivalent), which opens a read-only handle to the raw block device.  The resulting
`Arc<dyn BlockDevice>` is cloned and propagated to every other screen.  All
subsequent operations (health query, imaging, partition reading, filesystem opening,
and carving) will operate on this device.

> **Note:** If the device cannot be opened (e.g. you ran Ferrite without elevated
> privileges), an error is shown.  No changes are made to the device.

---

## 7. Screen 2 — Health Dashboard

**Purpose:** Query the drive's S.M.A.R.T. (Self-Monitoring, Analysis and Reporting
Technology) data via `smartctl` and display a health verdict with supporting metrics.

### Requirements

`smartctl` (from the **smartmontools** package) must be installed and accessible
on `PATH`.  Ferrite runs `smartctl --json -a <device>` and parses the JSON output.

### Layout

```
┌─ Health Dashboard — press r to refresh ─────────────────────────────────┐
│ ┌─ Summary ────────────────────────────────────────────────────────────┐ │
│ │  Verdict: ✓ HEALTHY                                                  │ │
│ │  Model: WDC WD40EZRZ-00W                                             │ │
│ │  Serial: WD-WCC7K5XXXXXX                                             │ │
│ │  Temp: 32°C   Power-on hours: 14233 h                               │ │
│ │  SMART self-test: PASSED                                             │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│ ┌─ Attributes (↑/↓ to scroll) ─────────────────────────────────────────┐ │
│ │ ID  Name                       Val  Wst  Thr  Raw          Failed?   │ │
│ │   1 Raw_Read_Error_Rate        200  200    0  0                       │ │
│ │   5 Reallocated_Sector_Ct      200  200  140  0                       │ │
│ │ ...                                                                  │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

### Health verdict

The verdict is computed by `ferrite-smart::assess_health()` against the thresholds
defined in `config/smart_thresholds.toml`.

| Verdict | Colour | Meaning |
|---|---|---|
| `✓ HEALTHY` | Green | All checks passed; drive appears normal |
| `⚠ WARNING` | Yellow | One or more warning thresholds exceeded; imaging is still allowed but be cautious |
| `✗ CRITICAL` | Red + Bold | One or more critical thresholds exceeded; imaging is still permitted but the drive may fail at any time |

Ferrite currently **does not block imaging** even on a Critical verdict — the
decision to proceed is left to the operator.

### Verdict assessment rules (in priority order)

| Priority | Condition | Result |
|---|---|---|
| 1 | `smart_passed == false` (self-test failed) | Critical |
| 2 | Temperature ≥ 60°C | Critical |
| 2 | Temperature ≥ 50°C | Warning |
| 3 | Reallocated Sectors (ID 5) ≥ 50 | Critical |
| 3 | Reallocated Sectors (ID 5) ≥ 1 | Warning |
| 4 | Pending Sectors (ID 197) ≥ 10 | Critical |
| 4 | Pending Sectors (ID 197) ≥ 1 | Warning |
| 5 | Uncorrectable Sectors (ID 198) ≥ 5 | Critical |
| 5 | Uncorrectable Sectors (ID 198) ≥ 1 | Warning |
| 6 | Spin-up time (ID 3) ≥ 20,000 ms | Critical |
| 6 | Spin-up time (ID 3) ≥ 10,000 ms | Warning |
| 7 | NVMe `critical_warning` flag non-zero | Warning |
| 8 | NVMe available spare below threshold | Critical |
| 9 | NVMe media/data integrity errors > 0 | Critical |
| 10 | Any attribute's `when_failed` is non-empty | Warning |

The highest severity from any triggered rule becomes the overall verdict.  All
triggered reasons are accumulated and shown below the verdict label.

### Attribute table columns

| Column | Meaning |
|---|---|
| `ID` | S.M.A.R.T. attribute number (1–255) |
| `Name` | Attribute name from smartctl |
| `Val` | Current normalised value (1–253; higher is usually better) |
| `Wst` | Worst recorded normalised value ever seen |
| `Thr` | Failure threshold (Val < Thr → pre-failure) |
| `Raw` | Raw counter value (hours, sector counts, etc.) |
| `Failed?` | `"now"`, `"past"`, or empty |

Attributes whose `Failed?` column is non-empty are highlighted **red**.

### NVMe devices

For NVMe drives, smartctl does not return individual attributes.  Instead, Ferrite
displays NVMe-specific fields: critical warning byte, media errors, available spare
percentage, and percentage used.

### Key bindings

| Key | Action |
|---|---|
| `r` | Re-query S.M.A.R.T. (spawns a new background thread) |
| `↑` / `↓` | Scroll the attribute table |

---

## 8. Screen 3 — Imaging

**Purpose:** Create a byte-for-byte image (sector copy) of the selected drive to an
output file using a resilient five-pass algorithm.  Progress is tracked in a mapfile
so interrupted sessions can be resumed.

### Layout

```
┌─ Imaging Engine ────────────────────────────────────────────────────────┐
│ ┌─ Configuration ──────────────────────────────────────────────────────┐ │
│ │  Source  : \\.\PhysicalDrive0                                        │ │
│ │  Dest    : C:\recovery\drive0.img                                    │ │
│ │  Mapfile : C:\recovery\drive0.map                                    │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│ ┌─ Progress ───────────────────────────────────────────────────────────┐ │
│ │ ████████████████████░░░░░░░░░░░░░░   Copy — 47.3%                   │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│ ┌─ Statistics ─────────────────────────────────────────────────────────┐ │
│ │  Finished: 89.4GiB  Bad: 0B  Non-tried: 100.6GiB  Elapsed: 01:23:45│ │
│ └──────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

### Before starting

1. Select a device on the Drives screen.
2. Press `d` to enter the **destination path** (where the image file will be written).
3. (Recommended) Press `m` to enter a **mapfile path** (enables resume and progress persistence).
4. Press `s` to start imaging.

> **The destination file must not be on the same physical drive being imaged.**
> Writing to the source drive could overwrite data you are trying to recover.

### Text field editing

When you press `d` or `m`, the corresponding input field activates and is highlighted
in **bold yellow** with a block cursor (`█`).

| Key | Action in edit mode |
|---|---|
| Any printable character | Appends to the path |
| `Backspace` | Deletes the last character |
| `Enter` or `Esc` | Confirms the entry and exits edit mode |

While a field is in edit mode, the global `q` quit key is suppressed so you can
type the character `q` in a path.

### The five-pass imaging algorithm

Ferrite implements a progressive, fault-tolerant copy algorithm:

#### Pass 1 — Copy
Reads the source device in large blocks (default: **512 KiB**).  Each block's
sector range is marked as either `Finished` (success) or `NonTrimmed` (read error).
This pass is fast and recovers the vast majority of a healthy drive.

#### Pass 2 — Trim
For each `NonTrimmed` block (a range where at least one sector failed), Ferrite
binary-searches the range to find the exact failing sector boundaries.  The result
is a precise set of smaller `NonScraped` ranges surrounding the bad sectors.

#### Pass 3 — Sweep
Reads any remaining unvisited areas between `Finished` regions.  Catches sectors
that were skipped because a larger block failed.

#### Pass 4 — Scrape
Reads each `NonScraped` range one sector at a time.  Sectors that succeed are
marked `Finished`; sectors that fail are marked `BadSector`.

#### Pass 5 — Retry
Re-reads every `BadSector` up to **3 times** (configurable via `max_retries`).
Some drives can return data on a subsequent attempt even after an initial failure.

### Block status codes

| Symbol | Status | Meaning |
|---|---|---|
| `?` | NonTried | Not yet read |
| `*` | NonTrimmed | Read failed; exact bad sector location not yet narrowed down |
| `/` | NonScraped | Trim completed; scrape not attempted |
| `-` | BadSector | All retries exhausted; sector is unreadable |
| `+` | Finished | Successfully read and written to the image file |

### Progress display

| Statistic | Meaning |
|---|---|
| **Phase** | Current pass name (Copy / Trim / Sweep / Scrape / Retry) |
| **Percentage** | `(finished + non_trimmed + non_scraped) / device_size` |
| **Finished** | Bytes successfully read and written |
| **Bad** | Bytes confirmed unreadable (`BadSector`) |
| **Non-tried** | Bytes not yet attempted |
| **Elapsed** | Wall-clock time since imaging started (HH:MM:SS) |

### Resuming an interrupted session

If a mapfile path was set and imaging was interrupted (power loss, crash, `c` cancel),
restart Ferrite, select the same device, enter the **same destination path** and
**same mapfile path**, and press `s`.  Ferrite will load the mapfile, skip all
`Finished` sectors, and resume from where it left off.

> **Important:** The device size recorded in the mapfile must match the current
> device exactly.  If the device has been replaced, Ferrite will refuse to resume.

### Mapfile auto-save

The mapfile is written atomically (to a `.tmp` file then renamed) every **30 seconds**
during imaging.  This means at most 30 seconds of progress can be lost if Ferrite
crashes.

### Key bindings

| Key | Action |
|---|---|
| `d` | Activate the destination path field for editing |
| `m` | Activate the mapfile path field for editing |
| `s` | Start imaging (no-op if already running or if destination is empty) |
| `c` | Cancel imaging (marks cancel flag; the current block completes before stopping) |
| `Esc` | Exit text-input edit mode (same as `Enter`) |

---

## 9. Screen 4 — Partition Analysis

**Purpose:** Read and display the partition table (MBR or GPT) from the selected
device.  When the partition table is corrupt or missing, scan the raw device for
filesystem signatures to reconstruct a likely partition layout.

### Layout

```
┌─ Partition Analysis — r: read  s: scan ──────────────────────────────────┐
│  Table type: GPT   Sector size: 512 B   Partitions: 3                    │
│                                                                           │
│  #  Type       Start LBA    End LBA       Size       Name / Info          │
│  1  GPT        2048         1050623       512.0 MiB  EFI System Partition │
│  2  GPT        1050624      135268351     64.0 GiB   Microsoft basic data │
│  3  GPT        135268352    937703086     382.1 GiB  Microsoft basic data │
└───────────────────────────────────────────────────────────────────────────┘
```

### Reading vs. scanning

**Read (`r`):** Parses the partition table at the beginning (and, for GPT, the end)
of the device.  This is fast (< 1 second) and works on intact drives.  Supports:

- **MBR** — Master Boot Record with up to four primary partitions.
- **GPT** — GUID Partition Table with up to 128 partitions, UUID-based type and
  partition GUIDs, and human-readable partition names.
- **Protective MBR** — Detected and promoted to GPT automatically.

**Scan (`s`):** Reads the device in steps (default sector size) looking for
filesystem magic bytes:

| Filesystem | Magic location | Bytes |
|---|---|---|
| NTFS | Offset 3, length 8 | `NTFS    ` (ASCII) |
| FAT32 | Offset 82, length 8 | `FAT32   ` (ASCII) |
| FAT16 | Offset 54, length 8 | `FAT16   ` (ASCII) |
| ext4 | Offset 1080, length 2 | `0x53 0xEF` (superblock magic) |

Each hit is converted to a recovered `PartitionEntry` with type `Recovered { fs_type }`.
Boundaries are estimated from the hit positions.  Scanning can be slow on large
devices because it reads every sector in sequence.

### Partition table types

| Type | Description |
|---|---|
| MBR | Classic Master Boot Record layout |
| GPT | GUID Partition Table (modern, used on UEFI systems) |
| Recovered | Synthesised from a signature scan; not read from a real table |

### Partition entry columns

| Column | Meaning |
|---|---|
| `#` | 1-based partition index |
| `Type` | MBR hex type byte, `GPT`, or `FsType` for recovered partitions |
| `Start LBA` | First logical block address of the partition |
| `End LBA` | Last logical block address (inclusive) |
| `Size` | Partition size in GiB / MiB / bytes |
| `Name / Info` | GPT partition name, or `—` for MBR / recovered partitions |

### Key bindings

| Key | Action |
|---|---|
| `↑` / `↓` | Scroll the partition list |
| `r` | Read the partition table from the device |
| `s` | Scan the raw device for filesystem signatures |

---

## 10. Screen 5 — File Browser

**Purpose:** Open the filesystem on the selected device, navigate its directory tree,
list file metadata, and optionally reveal deleted files.

### Layout

```
┌─ File Browser — d: toggle deleted  o: open filesystem ──────────────────┐
│  [Ntfs] / Windows / System32                                             │
│                                                                          │
│  Name                           Size      Type                           │
│  📁 AdvancedInstallers           —         DIR                            │
│  📁 Boot                         —         DIR                            │
│     bootmgr.efi                  413.2K    FILE                           │
│     cmd.exe [deleted]            270.0K    FILE                           │
└─────────────────────────────────────────────────────────────────────────┘
```

### Opening a filesystem

Press `o` to open the filesystem on the selected device.  Ferrite will:

1. Detect the filesystem type (NTFS / FAT32 / ext4 / Unknown) by reading magic bytes.
2. Spawn a background thread to parse the filesystem structures.
3. Load and display the root directory once ready.

If the filesystem type cannot be determined, an error is shown.  In that case, use
the Imaging screen to create an image first, then use a separate partition carving
workflow.

### Supported filesystems

| Filesystem | Root dir | Directory navigation | Deleted files | File extraction |
|---|---|---|---|---|
| NTFS | ✓ | ✓ | ✓ (in-use flag) | ✓ (resident data) |
| FAT32 | ✓ | ✓ | ✓ (0xE5 marker) | ✓ |
| ext4 | ✓ | ✓ | ✓ (inode == 0) | ✓ (direct + single-indirect) |

### Breadcrumb bar

The top of the screen always shows:

```
[Ntfs] / Windows / System32
```

The detected filesystem type appears in square brackets.  Path segments are separated
by ` / `.  The root directory is shown as `/`.

### Directory entries

| Icon | Meaning |
|---|---|
| `📁` | Directory (shown in **bold blue**) |
| (none) | Regular file |
| `[deleted]` suffix | Entry was found to be deleted |

Deleted entries are shown in **dark grey**.  They are only displayed when the deleted
file toggle is active.

### Deleted file detection

Each filesystem parser detects deletion differently:

- **FAT32:** The first byte of the directory entry name is `0xE5`.
- **NTFS:** The MFT record's in-use flag is cleared.
- **ext4:** The inode number in the directory entry is 0.

> **Note:** Ferrite marks entries as deleted based on metadata.  Whether the
> underlying data clusters or extents are still intact and readable depends on
> whether they have been overwritten.  Deleted file detection is best-effort.

### Known limitations (MVP)

- **NTFS:** Assumes a contiguous MFT.  Fragmented MFT tables may not be fully parsed.
- **ext4:** Only direct blocks and single-indirect blocks are supported.  Files using
  double- or triple-indirect blocks may show incorrect sizes or fail to read.
- **Timestamps:** `created` and `modified` timestamps are always `None` in the
  current implementation (shown as `—`).

### Key bindings

| Key | Action |
|---|---|
| `o` | Open (or re-open) the filesystem on the selected device |
| `↑` / `↓` | Move the directory entry selection |
| `Enter` | Open the selected directory (navigate into it) |
| `Backspace` | Go up one directory level |
| `d` | Toggle visibility of deleted files |

---

## 11. Screen 6 — File Carving

**Purpose:** Scan the raw byte stream of the selected device (or image file, if used
as the source) for file-type magic bytes and extract matching files.  File carving
does not rely on any filesystem structure and works even on completely wiped drives.

### Layout

```
┌─ Carving [idle] — Space: toggle  s: scan  e: extract  ←/→: switch panel ─┐
│ ┌─ Signatures (Space=toggle) ──────────┐ ┌─ Hits (4) — e: extract ──────┐│
│ │ [✓] JPEG Image                       │ │  JPEG Image @ offset 0x1a2000 ││
│ │ [✓] PNG Image                        │ │  JPEG Image @ offset 0x3f8000 ││
│ │ [✓] PDF Document                     │ │  PNG Image  @ offset 0x5c0000 ││
│ │ [ ] ZIP/Office                       │ │  PDF Document @ offset 0x8a000 ││
│ │ [✓] GIF Image                        │ └──────────────────────────────┘│
│ │ [✓] BMP Image                        │                                  │
│ │ [✓] MP3 Audio                        │                                  │
│ │ [✓] MP4 Video                        │                                  │
│ │ [✓] RAR Archive                      │                                  │
│ │ [✓] 7-Zip Archive                    │                                  │
│ └──────────────────────────────────────┘                                  │
└───────────────────────────────────────────────────────────────────────────┘
```

### Built-in file signatures

All ten signatures are loaded from `config/signatures.toml` (compiled into the binary):

| # | Name | Extension | Header (hex) | Footer (hex) | Max size |
|---|---|---|---|---|---|
| 1 | JPEG Image | `jpg` | `FF D8 FF` | `FF D9` | 10 MiB |
| 2 | PNG Image | `png` | `89 50 4E 47 0D 0A 1A 0A` | `49 45 4E 44 AE 42 60 82` | 50 MiB |
| 3 | PDF Document | `pdf` | `25 50 44 46` | `25 25 45 4F 46` | 100 MiB |
| 4 | ZIP / Office | `zip` | `50 4B 03 04` | `50 4B 05 06` | 500 MiB |
| 5 | GIF Image | `gif` | `47 49 46 38` | `00 3B` | 10 MiB |
| 6 | BMP Image | `bmp` | `42 4D` | (none) | 50 MiB |
| 7 | MP3 Audio | `mp3` | `49 44 33` | (none) | 50 MiB |
| 8 | MP4 Video | `mp4` | `00 00 00 20 66 74 79 70` | (none) | 4 GiB |
| 9 | RAR Archive | `rar` | `52 61 72 21 1A 07` | (none) | 500 MiB |
| 10 | 7-Zip Archive | `7z` | `37 7A BC AF 27 1C` | (none) | 500 MiB |

All signatures start enabled by default.

### How scanning works

1. Ferrite reads the device in 1 MiB chunks.
2. Each chunk overlaps the previous by `max(header_length) - 1` bytes to catch
   headers that span a chunk boundary.
3. For each enabled signature, Ferrite uses `memchr` to quickly locate bytes
   matching the first byte of the header, then checks the full header at each
   candidate position.
4. The scan runs in parallel using **rayon** (one thread per physical CPU core).
5. All hits are sorted by byte offset before being displayed.

Scanning runs on a background thread; the TUI remains responsive.  Because the
`scan()` API does not support mid-scan cancellation, pressing `c` marks the
operation cancelled on the TUI side, but the background thread finishes before
results are discarded.

### How extraction works

For each hit, Ferrite streams bytes from the hit offset:

- **Signatures with a footer** (`jpg`, `png`, `pdf`, `zip`, `gif`): Streams bytes
  until the footer sequence is found (inclusive), then stops.  The stream is capped
  at `max_size` even if no footer is found.
- **Signatures without a footer** (`bmp`, `mp3`, `mp4`, `rar`, `7z`): Streams
  exactly `max_size` bytes from the hit offset (or until end of device).

Extraction is performed on a background thread so the UI is not blocked.

### Extracted file naming

Extracted files are written to the **current working directory** (wherever Ferrite
was launched from) using the following naming pattern:

```
ferrite_<extension>_<byte_offset>.<extension>
```

Examples:
```
ferrite_jpg_1769472.jpg       ← JPEG found at byte offset 1,769,472
ferrite_png_6291456.png       ← PNG found at byte offset 6,291,456
ferrite_pdf_8978432.pdf
```

This naming scheme guarantees uniqueness even when multiple hits of the same type
are found at different offsets.

### Two-panel navigation

The screen is split into two panels:

| Panel | Content | Activated by |
|---|---|---|
| Left | Signature list (toggle enabled/disabled) | `←` arrow or default on entry |
| Right | Hits list (select and extract) | `→` arrow |

The **active panel** is indicated by a **bold yellow** title.

### Key bindings

| Key | Action |
|---|---|
| `←` | Focus the left panel (Signatures) |
| `→` | Focus the right panel (Hits); only available after a scan completes |
| `↑` / `↓` | Navigate the focused panel |
| `Space` | Toggle the selected signature on/off (left panel only) |
| `s` | Start a carving scan using all enabled signatures |
| `e` | Extract the selected hit to the current directory (right panel) |

---

## 12. Configuration Files

Ferrite ships two TOML configuration files in the `config/` directory at the
workspace root.  Both files are **compiled into the binary** via `include_str!`
macros, so they do not need to be present at runtime.

### `config/signatures.toml`

Defines the file-type signatures used by the carving engine.  Format:

```toml
[[signature]]
name      = "JPEG Image"
extension = "jpg"
header    = "FF D8 FF"
footer    = "FF D9"
max_size  = 10485760   # bytes (10 MiB)
```

| Field | Type | Description |
|---|---|---|
| `name` | String | Human-readable label shown in the TUI |
| `extension` | String | File extension (without leading dot) for extracted files |
| `header` | Hex string | Space-separated hex bytes marking the start of the file |
| `footer` | Hex string | Space-separated hex bytes marking the end (empty string = no footer) |
| `max_size` | Integer | Maximum bytes to extract per hit |

### `config/smart_thresholds.toml`

Controls what threshold values trigger Warning and Critical verdicts on the Health
Dashboard.  Default values:

```toml
[temperature]
warning_c  = 50
critical_c = 60

[reallocated_sectors]   # SMART attribute ID 5
warning_count  = 1
critical_count = 50

[pending_sectors]       # SMART attribute ID 197
warning_count  = 1
critical_count = 10

[uncorrectable_sectors] # SMART attribute ID 198
warning_count  = 1
critical_count = 5

[spin_up_time_ms]       # SMART attribute ID 3 (HDD only)
warning_ms  = 10000
critical_ms = 20000
```

To customise thresholds, edit `config/smart_thresholds.toml` before building.
The modified values will be compiled into the binary.

---

## 13. Mapfile Format

Ferrite uses a mapfile format that is **fully compatible with GNU ddrescue**.  You can
resume a Ferrite imaging session with `ddrescue` and vice versa.

### File structure

```
# Ferrite mapfile — generated by ferrite-imaging
# Command line information is not applicable for Ferrite
0x00000000  ?  0
0x00000000  0x00200000  +
0x00200000  0x00040000  *
0x00240000  0x7FC00000  ?
```

Line 1–2: Comment lines (begin with `#`).

Line 3: **Status line** — current position, current block status, current pass number.

Subsequent lines: **Block lines** — each on its own row with three space-separated
fields:

```
<start_offset_hex>  <size_hex>  <status_char>
```

| Field | Example | Meaning |
|---|---|---|
| Start offset | `0x00200000` | Byte offset from the beginning of the device |
| Size | `0x00040000` | Size of this block in bytes |
| Status | `+` | Block status (see table below) |

### Block status characters

| Char | Name | Meaning |
|---|---|---|
| `?` | NonTried | Block not yet read |
| `*` | NonTrimmed | Read failed; bad-sector boundary not yet located |
| `/` | NonScraped | Boundary located; scrape pending |
| `-` | BadSector | All retries exhausted; data is unreadable |
| `+` | Finished | Block successfully read and written |

### Interoperability with `ddrescue`

Because the format is identical, you can:

- Start an imaging session in Ferrite, stop it, and continue with `ddrescue` using
  the same mapfile and image file.
- Start with `ddrescue --no-split` for a fast first pass, then open Ferrite to
  continue with its Trim, Sweep, Scrape, and Retry passes for the bad sectors.

---

## 14. Logging & Diagnostics

Ferrite uses the `tracing` crate for structured logging.  Log output is written to
**stderr** and does not appear on screen during normal operation.

### Enable logging

Set the `RUST_LOG` environment variable before launching Ferrite:

```powershell
# Windows (PowerShell)
$env:RUST_LOG = "info"
.\ferrite.exe 2> ferrite.log

# Linux
RUST_LOG=info ./ferrite 2>ferrite.log
```

### Log level filters

| Level | Value | What is logged |
|---|---|---|
| Off | `RUST_LOG=off` | Nothing (default) |
| Error | `RUST_LOG=error` | Fatal and non-fatal errors only |
| Warn | `RUST_LOG=warn` | Warnings + errors |
| Info | `RUST_LOG=info` | High-level operations (enumeration complete, imaging done, etc.) |
| Debug | `RUST_LOG=debug` | Per-block imaging decisions, S.M.A.R.T. parse steps |
| Trace | `RUST_LOG=trace` | Key events, every block read, every channel message |

### Crate-specific filters

```bash
# Only show imaging logs at debug level, everything else at info
RUST_LOG=info,ferrite_imaging=debug ./ferrite
```

---

## 15. Permissions & Safety

### Why elevated privileges are needed

Block devices on both Windows and Linux require administrator / root access to open
in read mode.  Without elevation:

- Windows: `CreateFileW` on `\\.\PhysicalDriveN` returns `ERROR_ACCESS_DENIED`.
- Linux: `open("/dev/sdX", O_RDONLY | O_DIRECT)` returns `EACCES`.

Ferrite will show `"open failed (admin required?)"` in the Drive Selection list for
any device that cannot be opened.

### Read-only guarantee

Ferrite opens source devices with **read-only** flags at the OS level:

- **Windows:** `GENERIC_READ` only — no write access is requested from the kernel.
- **Linux:** `O_RDONLY` — the kernel refuses any write calls.

The `BlockDevice` trait does not expose a write method.  Even if a bug were
introduced in a parser or the carving engine, it could not write to the source device
through the `BlockDevice` abstraction.

### Output file safety

- The image file created by the Imaging screen is an ordinary file, not a device.
  It is opened with standard read/write access.
- Extracted carve files are written to the current working directory as ordinary
  files.
- The mapfile is written atomically (`.tmp` + rename) to prevent corruption on crash.

---

## 16. Filesystem Coverage & Known Limitations

### NTFS

| Feature | Status |
|---|---|
| Root directory listing | ✓ |
| Subdirectory navigation | ✓ |
| Deleted file detection (in-use flag) | ✓ |
| Resident file data reading | ✓ |
| Non-resident file data (data runs) | ✓ (contiguous MFT assumed) |
| Fragmented MFT | Not supported |
| Alternate data streams | Not supported |
| Compressed / encrypted files | Not supported |
| Timestamps | Not displayed (always `—`) |

### FAT32

| Feature | Status |
|---|---|
| Root directory listing | ✓ |
| Subdirectory navigation | ✓ |
| Deleted file detection (`0xE5` marker) | ✓ |
| File data reading (FAT chain) | ✓ |
| Long file names (LFN) | ✓ |
| FAT16 | Detected by partition scanner; not opened by filesystem parser |
| Timestamps | Not displayed (always `—`) |

### ext4

| Feature | Status |
|---|---|
| Root directory listing | ✓ |
| Subdirectory navigation | ✓ |
| Deleted file detection (inode == 0) | ✓ |
| Direct block reads | ✓ |
| Single-indirect block reads | ✓ |
| Double-indirect block reads | Not supported |
| Triple-indirect block reads | Not supported |
| Extents (ext4 feature) | Not supported |
| Timestamps | Not displayed (always `—`) |
| Journal recovery | Not supported |

---

## 17. Troubleshooting

### No drives appear in the Drive Selection list

**Causes and fixes:**

| Cause | Fix |
|---|---|
| Not running as administrator | Re-launch as Administrator (Windows) or `sudo` (Linux) |
| No physical drives on this machine | Expected on virtual machines with no raw disk pass-through |
| Drives visible but unreadable | Check Windows Device Manager or `lsblk` for drive status |

Press `r` on the Drive Selection screen to re-enumerate after fixing permissions.

### S.M.A.R.T. query fails with "Error: …"

| Cause | Fix |
|---|---|
| `smartctl` not found | Install smartmontools; ensure `smartctl` is on your `PATH` |
| `smartctl` requires root | Re-launch Ferrite with elevated privileges |
| USB drive not supported | Some external enclosures block S.M.A.R.T. passthrough |
| NVMe driver issue | Update NVMe drivers; smartctl ≥ 7.0 required for NVMe JSON |

### Imaging produces an image smaller than the source device

This can happen if the mapfile was loaded from a different device.  Ensure:
- The mapfile was created for this exact device (path + size must match).
- The output image file has not been truncated.

Delete the mapfile to force a fresh start if necessary.

### File Browser shows "Error: …" after pressing `o`

| Cause | Fix |
|---|---|
| Filesystem type is `Unknown` | The device may be unpartitioned, encrypted, or use an unsupported filesystem |
| Device has multiple partitions | Ferrite opens the raw device, not individual partitions; if the first partition does not start at offset 0, detection may fail |
| ext4 with extents | Not supported in the current MVP; use ext4 inspection tools on a mounted image |

### Carving scan produces many false positives

Some file types (especially `BMP` and `MP3`) have very short headers that can match
at random positions in binary data.  Disable the signatures for those types by
pressing `Space` before starting the scan.

### Carving extraction writes 0 bytes or stops early

- **Footer not found:** The file may be fragmented or the data between the header
  and footer may have been overwritten.  The maximum extraction window (`max_size`)
  is used instead.
- **Device read error:** The sectors containing the file may be bad.  Consider
  imaging the device first and carving from the image file.

### Ferrite crashes and the terminal is left in a broken state

Ferrite installs a panic hook that restores the terminal (disables raw mode, exits
the alternate screen buffer) before printing the panic message.  If the terminal
is still broken, run:

```bash
# Linux / macOS
reset

# Windows (in the same PowerShell window)
[console]::TreatControlCAsInput = $false
```

---

## 18. Glossary

| Term | Definition |
|---|---|
| **Bad sector** | A disk sector that cannot be read after all retry attempts |
| **Block device** | A storage device accessed in fixed-size chunks (sectors); e.g. `/dev/sda`, `\\.\PhysicalDrive0` |
| **Carving** | Extracting files from raw binary data by recognising their header/footer magic bytes, without relying on filesystem metadata |
| **CRC** | Cyclic Redundancy Check — used in GPT headers and sector checksums |
| **ddrescue** | GNU ddrescue is a data recovery tool that uses a similar multi-pass algorithm and the same mapfile format as Ferrite |
| **Direct I/O** | Reads that bypass the OS page cache and go directly to the device (`O_DIRECT` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows) |
| **ERC / TLER** | Error Recovery Control / Time-Limited Error Recovery — a drive firmware setting that caps per-sector retry time; Ferrite does not configure this but benefits when it is set |
| **Footer** | A magic byte sequence at the end of a file (e.g. `FF D9` for JPEG) used by the carver to determine the file boundary |
| **GPT** | GUID Partition Table — the modern partition scheme used on UEFI systems |
| **Header** | The magic byte sequence at the start of a file used by the carver to detect file type (e.g. `FF D8 FF` for JPEG) |
| **Image file** | A byte-for-byte copy of a block device saved as a regular file |
| **LBA** | Logical Block Address — a zero-based index of a 512-byte (or 4 KiB) sector on a block device |
| **Mapfile** | A text file tracking the copy status of every sector; compatible with GNU ddrescue |
| **MBR** | Master Boot Record — the legacy partition table format stored in the first 512 bytes of a device |
| **MFT** | Master File Table — the central metadata store in NTFS |
| **NVMe** | Non-Volatile Memory Express — a protocol for SSDs connected over PCIe |
| **S.M.A.R.T.** | Self-Monitoring, Analysis and Reporting Technology — firmware-level drive health monitoring |
| **Sector** | The smallest addressable unit on a block device (typically 512 bytes; some modern drives use 4 KiB) |
| **Sector alignment** | The requirement that read offsets and buffer sizes are multiples of the sector size when using direct I/O |
| **Signature** | In file carving, the combination of a header magic, optional footer magic, and maximum size that identifies a file type |
| **smartctl** | The command-line tool from the smartmontools package used to query S.M.A.R.T. data |
