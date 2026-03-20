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
12. [Screen 7 — Hex Viewer](#12-screen-7--hex-viewer)
13. [Screen 8 — Quick Recover](#13-screen-8--quick-recover)
14. [Screen 9 — Artifact Scanner](#14-screen-9--artifact-scanner)
15. [Screen 10 — Text Block Scanner](#15-screen-10--text-block-scanner)
16. [Configuration Files](#16-configuration-files)
17. [Mapfile Format](#17-mapfile-format)
18. [Logging & Diagnostics](#18-logging--diagnostics)
19. [Permissions & Safety](#19-permissions--safety)
20. [Filesystem Coverage & Known Limitations](#20-filesystem-coverage--known-limitations)
21. [Troubleshooting](#21-troubleshooting)
22. [Glossary](#22-glossary)

---

## 1. Introduction

Ferrite is a terminal-based storage diagnostics and data recovery application written
in pure Rust.  It is designed to help users assess the health of failing drives,
create resilient byte-for-byte images, recover lost partition tables, browse
filesystem contents (including deleted files), carve individual files from raw
disk images, scan for forensic artifacts, and extract text blocks — all from a
single interactive terminal UI.

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
┌─ Ferrite ──────────────────────────────────────────────────────────────────────────────────┐
│ Drives │ Health │ Imaging │ Partitions │ Files │ Carving │ Hex │ Quick Recover │ Artifacts │ Text Scan │  ← Tab bar
├────────────────────────────────────────────────────────────────────────────────────────────┤
│                                                                                            │
│                              Screen content area                                           │
│                                                                                            │
├────────────────────────────────────────────────────────────────────────────────────────────┤
│  ↑/↓: navigate  Enter: select device  r: refresh list  Tab: next  q: quit                 │  ← Help bar
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

### Tab bar

The tab bar at the top shows all ten screens.  The currently active screen is
highlighted in **bold yellow**.

### Help bar

The bottom row shows context-sensitive key hints for the active screen.  It updates
automatically whenever the screen or mode changes.

### Global key bindings

These work from every screen at all times:

| Key | Action |
|---|---|
| `Tab` | Move to the next screen (wraps from Text Scan back to Drives) |
| `Shift-Tab` | Move to the previous screen (wraps from Drives back to Text Scan) |
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

When a device is selected, Ferrite immediately runs a **write-blocker pre-flight
check** in the background.  If the check detects that the device is writable
(unexpected for a read-only session), an amber warning is shown below the device
list.  The check does not block device selection.

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
| `s` | Cycle sort order (by Path / by Size descending) |
| `/` | Open filter bar — type to narrow the list by path, model, or serial |
| `Esc` | Clear the active filter |
| `f` | Open an image file as the source device (see below) |
| `o` | Open the saved-session manager |

### Opening an image file as the source device

Press `f` to open the **image-file overlay**.  Type (or paste) the full path to a
`.img` file previously created on the Imaging screen, then press `Enter`.  The image
file is opened via `FileBlockDevice` and propagated to all other screens exactly as
a physical drive would be.

```
┌─ Open Image File ─────────────────────────────────────────────────┐
│                                                                     │
│  Path: E:\images\drive9.img█                                       │
│                                                                     │
│  Enter path to .img file  ·  Esc: cancel                           │
└─────────────────────────────────────────────────────────────────────┘
```

If the path is invalid or the file cannot be opened, an error is shown in the
overlay and the input remains active so you can correct the path.  Press `Esc` to
close the overlay without selecting anything.

> **Recommended workflow for critically damaged drives:**
> 1. Select the physical drive and go to the **Imaging** screen.
> 2. Image the drive (even a partial image is useful — bad sectors are zero-filled).
> 3. Return to the Drive Selection screen and press `f` to open the resulting `.img` file.
> 4. All tabs now operate on the image — the drive is no longer stressed by repeated reads.

### What happens after selecting a device

Pressing `Enter` on a device (or `Enter` after typing an image path with `f`) opens
a read-only handle to the block device or file.  The resulting `Arc<dyn BlockDevice>`
is cloned and propagated to every other screen.  All subsequent operations (health
query, imaging, partition reading, filesystem opening, and carving) will operate on
this device.

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
2. **Option A — Auto-generate filenames (recommended for quick starts):**
   Leave Dest empty and press `s`.  Ferrite generates a filename from the drive
   serial and today's date, e.g. `ST8000DM004_ABC12345_20260319.img`, and places
   it in the current working directory.  A companion `.map` mapfile is also
   generated automatically.
   You can also type just a directory path in Dest (e.g. `m:\restore\`) and press
   `s` — Ferrite will place the auto-named file inside that directory.
3. **Option B — Explicit path:**
   Press `d` to enter the full destination file path (e.g. `m:\restore\disk.img`).
   Press `m` to enter a mapfile path (e.g. `m:\restore\disk.map`).
   Press `s` to start.

> **Path separators:** You may type either `\` or `\\` between path components;
> Ferrite normalises consecutive backslashes automatically.

> **File-exists protection:** If an auto-generated name already exists, Ferrite
> appends `_1`, `_2`, … before the extension to avoid overwriting previous images.

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

### Safety features

#### Write-blocker pre-flight

When a device is selected, Ferrite runs `ferrite-imaging::write_blocker::check()` in
the background.  If the device is unexpectedly writable, an amber warning badge is
shown on the Imaging screen header before any scan begins.

#### SHA-256 integrity sidecar

When imaging completes successfully, Ferrite writes a `.sha256` sidecar file next to
the image file (e.g. `drive0.img.sha256`).  The format is compatible with
`sha256sum -c`:

```
<hex-digest>  drive0.img
```

If an imaging session is **resumed** from a mapfile, the progress bar title is shown
in **amber** as a reminder that the SHA-256 hash covers only the data written in
prior sessions combined with the current session.  The sidecar is updated on
completion.

#### ThermalGuard

Ferrite monitors the drive temperature during imaging via a background `ThermalGuard`
thread.  If the temperature exceeds the configured threshold (default: 55°C), imaging
is automatically paused and a `[⚠ THERMAL PAUSE]` badge appears on the progress bar.
Imaging resumes automatically once the temperature drops below the resume threshold.
ThermalGuard is configured in `config/smart_thresholds.toml`.

#### Low read-rate alert

If the sustained read rate drops below **5 MB/s**, the progress bar turns **amber**
and a `[⚠ LOW RATE]` label appears in the title.  The rate line in the Statistics
panel is also shown in amber.  This is an early warning that the drive may be
struggling; consider pausing to let it recover, or note that the remaining sectors
may be bad.

#### Watchdog — unresponsive drive detection

If no `Progress` update arrives for **10 or more seconds** while imaging is
running (and not paused), the Statistics panel shows:

```
⚠ No read progress for 47s — drive may be unresponsive (try Reverse mode with r, or cancel with c)
```

This most commonly occurs when a critically damaged drive's USB controller holds a
pending read at the hardware level far longer than the 30-second software timeout.
Recommended actions:

1. Press `r` to enable **Reverse** mode (imaging from the end of the drive backwards),
   then `s` to restart.  This bypasses bad sectors near the beginning of the drive.
2. Press `c` to cancel, then check `smartctl -a` for reallocated sector count and
   pending sector count to assess how much data is likely recoverable.
3. If you have an image file from a previous partial run, open it with the `f` key
   on the Drive Selection screen and proceed directly to carving.

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
| `l` | Activate the start LBA field (leave empty for beginning of device) |
| `e` | Activate the end LBA field (leave empty for end of device) |
| `b` | Activate the block size field (KiB; default 512 KiB) |
| `r` | Toggle Reverse mode (image from end to start — useful when start of drive is unresponsive) |
| `p` | Pause / resume imaging manually |
| `s` | Start imaging — auto-generates filename if Dest is empty or a directory |
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
list file metadata, reveal deleted files, and extract individual files.

### Layout

```
┌─ File Browser — d: toggle deleted  o: open filesystem  e: extract ──────┐
│  [Ntfs] / Windows / System32                                             │
│                                                                          │
│  Name                           Size      Type      Recovery             │
│  📁 AdvancedInstallers           —         DIR                            │
│  📁 Boot                         —         DIR                            │
│     bootmgr.efi                  413.2K    FILE                           │
│     cmd.exe [deleted]            270.0K    FILE      [HIGH]               │
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
| ext4 | ✓ | ✓ | ✓ (inode == 0) | ✓ (direct, single-indirect, and extents) |

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

### Recovery chance (deleted files)

When deleted file display is active, each deleted entry shows a **RecoveryChance**
indicator in the Recovery column:

| Badge | Colour | Meaning |
|---|---|---|
| `[HIGH]` | Green | Data clusters / extents appear intact and unoverwritten |
| `[MED]` | Yellow | Partial overwrite detected; some data may be recoverable |
| `[LOW]` | Red | Significant overwrite; recovery unlikely |
| `[ ? ]` | Dark gray | Recovery chance could not be determined |

### Deleted file detection

Each filesystem parser detects deletion differently:

- **FAT32:** The first byte of the directory entry name is `0xE5`.
- **NTFS:** The MFT record's in-use flag is cleared.
- **ext4:** The inode number in the directory entry is 0.

> **Note:** Ferrite marks entries as deleted based on metadata.  Whether the
> underlying data clusters or extents are still intact and readable depends on
> whether they have been overwritten.  Deleted file detection is best-effort.

### File extraction

Press `e` on a selected file entry to extract it.  Extracted files are written to
`./ferrite_output/fs_recovery/` relative to where Ferrite was launched.  The
original filename is preserved.  A status message confirms extraction success or
reports any read errors.

### Known limitations

- **NTFS:** Assumes a contiguous MFT.  Fragmented MFT tables may not be fully parsed.
- **ext4:** Double- and triple-indirect block reads are not supported.  Files that
  use only those block addressing modes may show incorrect sizes or fail to read.
  Files using the extents feature (the common case on modern ext4) are fully
  supported.
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
| `e` | Extract the selected file to `./ferrite_output/fs_recovery/` |

---

## 11. Screen 6 — File Carving

**Purpose:** Scan the raw byte stream of the selected device (or image file) for
file-type magic bytes and extract matching files.  File carving does not rely on any
filesystem structure and works even on completely wiped or reformatted drives.

### Layout

```
┌─ Carving [scanning…] — s: scan  e: extract  ←/→: switch panel ──────────┐
│ ┌─ Signatures ──────────────────────────────┐ ┌─ Hits (42) [↓ LIVE] ───┐│
│ │ ▶ Images (9/10)                           │ │ ✓ JPEG  @ 0x001a2000   ││
│ │ ▼ RAW Photos (15/15)                      │ │ ~ PNG   @ 0x003f8000   ││
│ │   [✓] ARW                                 │ │ ✗ PDF   @ 0x008a0000   ││
│ │   [✓] CR2                                 │ │   MP4   @ 0x012c0000   ││
│ │   [✓] NEF                                 │ └────────────────────────┘│
│ │ ▶ Video (17/17)                           │                            │
│ │ ▶ Audio (10/10)                           │                            │
│ │ ▶ Archives (8/8)                          │                            │
│ │ ▶ Documents (17/17)                       │                            │
│ │ ▶ Office & Email (4/4)                    │                            │
│ │ ▶ System (18/18)                          │                            │
│ └───────────────────────────────────────────┘                            │
└───────────────────────────────────────────────────────────────────────────┘
```

### Signature groups

All 99 built-in signatures are organised into 8 collapsible groups.  Groups start
**collapsed** by default.  The header shows how many signatures are enabled:
`▶ Video (17/17)` means all 17 Video signatures are active.

| Group | Count | Contents |
|---|---|---|
| Images | 10 | JPEG×2, PNG, GIF, BMP, TIFF×2, WebP, PSD, ICO |
| RAW Photos | 15 | ARW, CR2, NEF, RW2, RAF, HEIC×2, ORF, PEF, CR3, SR2, DCR, CRW, MRW, X3F |
| Video | 17 | MP4, MOV, M4V, 3GP, AVI, MKV, WebM, WMV, FLV, MPG, RM, SWF×3, TS, M2TS, WTV |
| Audio | 10 | MP3, WAV, FLAC, OGG, M4A, MIDI, AIFF, WavPack, APE, AU |
| Archives | 8 | ZIP, RAR, 7-Zip, GZip, XZ, BZip2, ISO, TAR |
| Documents | 17 | PDF, XML, HTML, RTF, VCF, ICS, EML, EPUB, ODT, CDR, TTF, WOFF, CHM, Blender, InDesign, PHP, Shebang |
| Office & Email | 4 | ZIP-Office, OLE2, PST, MSG |
| System | 18 | SQLite, EVTX, EXE, ELF, VMDK, REGF, VHD, VHDX, QCOW2, Mach-O, KDBX, KDB, E01, PCAP×2, DMP, plist, LUKS, DICOM |

Signatures are loaded from `config/signatures.toml` (compiled into the binary).

### Custom user signatures

Press `u` to open the **Custom Signature Panel**.  You can add, edit, delete, and
import user-defined signatures that appear as a ninth "Custom" group.  Custom
signatures support the same header/footer/max_size fields as built-in signatures.

### How scanning works

1. Ferrite reads the device in 1 MiB chunks with overlap to catch headers spanning
   chunk boundaries.
2. For each enabled signature, `memchr` locates candidate bytes; the full header is
   verified at each position.
3. Structural pre-validators run for many formats before a hit is accepted (e.g. TIFF
   IFD walk, PDF parser, PE/ELF header check, RIFF FOURCC check).
4. Hits are sorted by byte offset and displayed in the right panel.
5. Scanning runs on a background thread; the TUI remains fully responsive.

### Carve quality indicators

After extraction, each hit is post-validated and assigned a **CarveQuality** tag
shown as a prefix in the hits list:

| Symbol | Quality | Meaning |
|---|---|---|
| `✓` | Complete | File structure validated; appears intact |
| `~` | Truncated | Footer found but file structure suggests incomplete data |
| `✗` | Corrupt | Internal structure validation failed (bad CRC, invalid markers, etc.) |
| ` ` | Unknown | No post-validator available for this format |

### Auto-follow (live scanning)

While a scan is in progress, the hits panel automatically scrolls to keep the latest
hit visible and shows a **`[↓ LIVE]`** badge in the panel title.  As soon as you
press `↑` or `↓` to navigate the hits list, auto-follow disables and the badge
disappears.  This lets you inspect early hits without losing track of the live
stream.

### Deduplication

Ferrite computes a 4 KiB fingerprint for each extracted file.  If a new hit produces
an identical fingerprint to one already extracted, it is tagged `[DUP]` and skipped.
A summary line shows `⊘ N skipped` at the end of extraction.  Toggle dedup with
`D`.

### Skip modes

| Mode | Key | Behaviour |
|---|---|---|
| Skip-corrupt | `Shift+C` | Files post-validated as `Corrupt` are automatically deleted after extraction |
| Skip-truncated | `t` | Files post-validated as `Truncated` are automatically deleted after extraction |

Both modes are persisted in the session JSON file so they survive a restart.

### How extraction works

For each hit, Ferrite uses format-specific **size hints** where possible (PE/EXE,
ELF, RAR, MKV/WebM, TS/M2TS, PNG, GIF, PDF linearized, etc.) to determine the
exact file length before writing.  For formats without a size hint, extraction falls
back to footer search then max_size cap.

Extracted files are written to `./ferrite_output/carving/` using the pattern:

```
ferrite_<extension>_<byte_offset>.<extension>
```

Example:
```
ferrite_jpg_1769472.jpg
ferrite_png_6291456.png
```

Zero-byte files are deleted automatically after extraction.

### Two-panel navigation

| Panel | Content | Activated by |
|---|---|---|
| Left | Signature groups + individual sigs (toggle enabled/disabled) | `←` arrow or default on entry |
| Right | Hits list (quality tag, format, offset) | `→` arrow |

The **active panel** is indicated by a **bold yellow** title.

### Key bindings

| Key | Action |
|---|---|
| `←` / `→` | Switch focus between Signatures panel (left) and Hits panel (right) |
| `↑` / `↓` | Navigate the focused panel |
| `Space` | Toggle signature on/off (left panel, sig row) OR toggle all sigs in group on/off (left panel, group row) |
| `Enter` | Expand / collapse signature group (left panel, group row) |
| `s` | Start carving scan |
| `e` | Extract selected hit (right panel) |
| `E` | Extract all hits |
| `Shift+C` | Toggle skip-corrupt mode |
| `t` | Toggle skip-truncated mode |
| `D` | Toggle duplicate suppression |
| `u` | Open custom user signature panel |

---

## 12. Screen 7 — Hex Viewer

**Purpose:** Inspect the raw bytes of the selected device sector-by-sector.  Useful
for manually verifying carve hit offsets, examining partition table bytes, and
diagnosing filesystem corruption.

### Layout

```
┌─ Hex Viewer — offset: 0x00000000 ───────────────────────────────────────┐
│ 00000000  4D 5A 90 00 03 00 00 00  04 00 00 00 FF FF 00 00  │MZ..........│
│ 00000010  B8 00 00 00 00 00 00 00  40 00 00 00 00 00 00 00  │........@...│
│ 00000020  00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00  │............│
│ ...                                                                      │
└─────────────────────────────────────────────────────────────────────────┘
```

The display shows:

- **Offset column** (left): byte offset from the start of the device in hexadecimal.
- **Hex columns** (centre): 16 bytes per row, split into two groups of 8, displayed
  in hex.  Null bytes are shown in dark gray; printable ASCII bytes are highlighted.
- **ASCII column** (right): the same 16 bytes rendered as ASCII characters.
  Non-printable bytes are shown as `.`.

### Navigation

| Key | Action |
|---|---|
| `↑` / `↓` | Scroll one row (16 bytes) up or down |
| `PgUp` / `PgDn` | Scroll one screen at a time |
| `g` | Open the offset jump dialog — type a hex offset and press Enter |
| `Home` | Jump to offset 0 |

### Offset jump dialog

Press `g` to activate the jump dialog.  Type a hexadecimal offset (with or without a
`0x` prefix) and press `Enter`.  The viewer will seek to that byte offset, aligned
to the nearest 16-byte row boundary.  Press `Esc` to cancel without navigating.

---

## 13. Screen 8 — Quick Recover

**Purpose:** Scan the selected device for deleted files using filesystem metadata
(NTFS MFT, FAT32 directory entries, or ext4 inodes) and recover them in bulk — faster
than full file carving when the filesystem is partially intact.

### Layout

```
┌─ Quick Recover — [NTFS] ────────────────────────────────────────────────┐
│  Filter: _______________                                                 │
│                                                                          │
│  [HIGH] documents\report.docx              14.2 KB                      │
│  [HIGH] pictures\photo_001.jpg             3.8 MB                       │
│  [MED ] pictures\photo_002.jpg             3.1 MB    [✓]                 │
│  [LOW ] videos\clip.mp4                    720.4 MB                     │
│  [ ?  ] downloads\setup.exe               1.2 MB    [✓]                 │
│                                                                          │
│  ─────────────────────────────────────────────────────────────────────  │
│  2 files checked  •  R: recover checked files                           │
└─────────────────────────────────────────────────────────────────────────┘
```

### How it works

After a device is selected, Quick Recover:

1. Detects the filesystem type.
2. Opens the appropriate parser and calls `deleted_files()` to enumerate all deleted
   entries found in the filesystem metadata.
3. Scores each entry with a `RecoveryChance` based on whether the data clusters or
   extents appear to still contain the original data.
4. Displays the result list sorted by recovery chance (HIGH first).

All detection runs on a background thread; the list populates progressively.

### Recovery chance badges

| Badge | Colour | Meaning |
|---|---|---|
| `[HIGH]` | Green | Data appears intact; recovery very likely to succeed |
| `[MED]` | Yellow | Partial overwrite or uncertainty detected |
| `[LOW]` | Red | Heavily overwritten; recovery unlikely |
| `[ ? ]` | Dark gray | Could not be determined |

### Selecting files for recovery

| Key | Action |
|---|---|
| `↑` / `↓` | Navigate the file list |
| `Space` | Toggle the check mark on the selected file |
| `a` | Check all files with `[HIGH]` recovery chance |
| `A` | Check all files in the list |
| `R` | Recover all checked files |
| `/` | Activate the filter box — type to filter by filename |

### Output

Recovered files are written to `./ferrite_output/quick_recover/` preserving the
original filename.  If two deleted files share the same name, a numeric suffix is
appended.

---

## 14. Screen 9 — Artifact Scanner

**Purpose:** Scan the raw byte stream for forensic personally identifiable information
(PII) artifacts: email addresses, URLs, credit card numbers, IBANs, Windows file
paths, and US Social Security Numbers.

> **Privacy notice:** A consent dialog is shown before scanning begins.  No data
> leaves the local machine; all processing is done in-process with no network access.

### Layout

```
┌─ Artifact Scanner ──────────────────────────────────────────────────────┐
│ ████████████████████████░░░░░░░░░░░░░  scanning… 63.4%                  │
│ Filter: email___________                                                 │
│                                                                          │
│  Email    user@example.com               offset: 0x00a4c200             │
│  URL      https://internal.corp/admin    offset: 0x00a4c340             │
│  CCard    **** **** **** 4242            offset: 0x00b10000             │
│  IBAN     GB29 NWBK 6016 1331 9268 19   offset: 0x00b10200             │
│  WinPath  C:\Users\alice\Documents\...  offset: 0x00c00100             │
│                                                                          │
│  Total hits: 47   •   X: export CSV                                     │
└─────────────────────────────────────────────────────────────────────────┘
```

### Consent dialog

On first launch (and whenever the screen is entered without a prior consent), Ferrite
displays a one-time consent dialog explaining the nature of the scan.  Press `Y` to
consent and begin; press `N` or `Esc` to cancel.

### Scanner types

| Kind | Description | Notes |
|---|---|---|
| `Email` | RFC 5321-compatible email addresses | Deduplicated per address |
| `URL` | HTTP and HTTPS URLs | Deduplicated per URL |
| `CCard` | Luhn-valid 13–19 digit credit card numbers | Displayed masked to last 4 digits |
| `IBAN` | International Bank Account Numbers (ISO 13616) | Deduplicated per IBAN |
| `WinPath` | Windows absolute paths (`C:\...`, `\\server\share\...`) | Truncated to 120 chars for display |
| `SSN` | US Social Security Numbers (`XXX-XX-XXXX`) | Common invalid patterns excluded |

### Implementation notes

- Regex patterns are compiled once via `OnceLock` and reused for the entire scan.
- A 4 KiB overlap buffer ensures patterns that span 1 MiB read boundaries are not
  missed.
- Per-kind `HashSet` deduplication prevents the same string from appearing twice.
- Credit card numbers are validated with the Luhn algorithm before being recorded.
- All scanning runs on a background thread; the hit list updates live.

### Filter and export

Type in the filter box to show only hits whose kind or value contains the search
string.  Press `X` to export all hits (unfiltered) to a CSV file:

```
ferrite_artifacts_<timestamp>.csv
```

CSV columns: `kind`, `value`, `offset_hex`.

### Key bindings

| Key | Action |
|---|---|
| `s` | Start a new scan (shows consent dialog if not yet consented) |
| `↑` / `↓` | Navigate the hit list |
| `/` | Activate the filter box |
| `X` | Export all hits to CSV |

---

## 15. Screen 10 — Text Block Scanner

**Purpose:** Extract coherent text blocks from raw binary data without relying on any
filesystem or file format.  Useful for recovering plain text documents, source code,
CSV data, configuration files, and log files from unstructured regions of a drive.

> **Privacy notice:** A consent dialog is shown before scanning begins, identical to
> the one on the Artifact Scanner screen.

### Layout

```
┌─ Text Block Scanner ────────────────────────────────────────────────────┐
│ ████████████████████████░░░░░░░░░░░░░  scanning… 41.2%                  │
│ Kind filter: [All]                                                       │
│                                                                          │
│  [Prose   ] offset: 0x00120000  len: 4.2 KB  "The quick brown fox…"     │
│  [Code    ] offset: 0x00340000  len: 12.1 KB "fn main() { …"            │
│  [CSV     ] offset: 0x00780000  len: 2.8 KB  "name,age,email\r\n…"      │
│  [Log     ] offset: 0x009a0000  len: 8.0 KB  "2024-01-15 10:32:…"       │
│                                                                          │
│  Blocks found: 31   •   W: write files                                  │
└─────────────────────────────────────────────────────────────────────────┘
```

### Consent dialog

A one-time consent dialog is displayed before the first scan.  Press `Y` to proceed
or `N` / `Esc` to cancel.

### TextKind variants

The classifier assigns one of nine kinds to each block:

| Kind | Description |
|---|---|
| `Prose` | Natural language paragraphs (high word-level entropy, mixed punctuation) |
| `Code` | Source code (keywords, braces, indentation patterns) |
| `CSV` | Comma- or tab-separated values (consistent delimiter density) |
| `Log` | Log file lines (timestamp prefix, repetitive structure) |
| `Config` | INI / TOML / YAML style configuration text |
| `Html` | HTML / XML markup (tag density) |
| `Json` | JSON structure detected |
| `Path` | Predominantly file paths and directory listings |
| `Other` | Printable text that does not match any above category |

### Scanning algorithm

Ferrite uses a **gap-tolerant sliding window** approach:

1. A window advances byte-by-byte across the device.
2. Bytes with values 0x09 (tab), 0x0A (LF), 0x0D (CR), and 0x20–0x7E (printable
   ASCII) are counted as valid text bytes.
3. A block starts when the printable ratio over the window exceeds a threshold
   (default: 85%).
4. The block continues as long as the printable ratio stays above a lower threshold
   (default: 70%), tolerating short binary gaps (e.g., embedded null terminators).
5. The block ends when the printable ratio falls below the lower threshold.
6. Blocks shorter than a minimum length (default: 512 bytes) are discarded.
7. The classifier is run on each accepted block to assign a `TextKind`.

All scanning runs on a background thread.

### Kind filter

Press `k` to cycle through the nine TextKind filters plus "All".  The hits list
immediately updates to show only blocks matching the selected kind.

### Writing extracted blocks

Press `W` to write all discovered text blocks (from the unfiltered set) to
`./ferrite_output/text_scan/`.  Each block is written as a `.txt` file named by its
byte offset:

```
block_0x00120000.txt
block_0x00340000.txt
```

### Key bindings

| Key | Action |
|---|---|
| `s` | Start a new scan |
| `↑` / `↓` | Navigate the block list |
| `k` | Cycle the TextKind filter |
| `W` | Write all blocks to `./ferrite_output/text_scan/` |

---

## 16. Configuration Files

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
Dashboard, and configures ThermalGuard imaging pause thresholds.  Default values:

```toml
[temperature]
warning_c  = 50
critical_c = 60

[thermal_guard]
pause_c  = 55
resume_c = 45

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

## 17. Mapfile Format

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

## 18. Logging & Diagnostics

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

## 19. Permissions & Safety

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
- Extracted carve files are written to `./ferrite_output/carving/` as ordinary files.
- Recovered filesystem files are written to `./ferrite_output/fs_recovery/`.
- Quick Recover output goes to `./ferrite_output/quick_recover/`.
- Text block output goes to `./ferrite_output/text_scan/`.
- The mapfile is written atomically (`.tmp` + rename) to prevent corruption on crash.

---

## 20. Filesystem Coverage & Known Limitations

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
| Extents (ext4 feature) | ✓ (full extent tree supported) |
| Double-indirect block reads | Not supported |
| Triple-indirect block reads | Not supported |
| Timestamps | Not displayed (always `—`) |
| Journal recovery | Not supported |

---

## 21. Troubleshooting

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

### Imaging pauses with `[⚠ THERMAL PAUSE]`

ThermalGuard has detected that the drive temperature exceeded the configured pause
threshold (default 55°C).  Imaging will resume automatically when the temperature
drops to the resume threshold (default 45°C).  Ensure the drive has adequate
airflow.  If the drive keeps overheating, consider shorter imaging sessions with
cooling breaks between runs.

### Imaging shows `[⚠ LOW RATE]` warning

The sustained read rate has dropped below 5 MB/s.  This can indicate:
- The drive is reaching bad sectors and is retrying internally.
- The drive is throttling due to temperature.
- The drive hardware is degrading.

Consider pausing and checking the Health Dashboard.  Imaging the drive over multiple
short sessions can reduce the risk of complete drive failure mid-session.

### File Browser shows "Error: …" after pressing `o`

| Cause | Fix |
|---|---|
| Filesystem type is `Unknown` | The device may be unpartitioned, encrypted, or use an unsupported filesystem |
| Device has multiple partitions | Ferrite opens the raw device, not individual partitions; if the first partition does not start at offset 0, detection may fail |

### Carving scan produces many false positives

Some file types (especially `BMP`, `MP3`, and `PHP`) have very short or common
headers that can match at random positions in binary data.  Disable the signatures
for those types by navigating to the Signatures panel (`←`), locating the signature,
and pressing `Space` before starting the scan.  Consider enabling skip-corrupt mode
(`Shift+C`) so that structurally invalid hits are discarded automatically.

### Carving extraction writes 0 bytes or stops early

- **Footer not found:** The file may be fragmented or the data between the header
  and footer may have been overwritten.  The maximum extraction window (`max_size`)
  is used instead.
- **Device read error:** Since Phase 87, Ferrite zero-fills bad sectors during
  extraction rather than aborting.  Extracted files may contain zero-filled gaps
  in place of unreadable sectors; post-validation will mark them `Truncated (~)` or
  `Corrupt (✗)`.  Imaging the device first and carving from the image file gives the
  best results on heavily damaged drives (see *Working with critically damaged drives*
  below).
- **Zero-byte cleanup:** Ferrite deletes zero-byte output files automatically.
  If many files are zero bytes, the source regions may be entirely unreadable.

### Working with critically damaged drives

When a drive shows **S.M.A.R.T. CRITICAL** and produces I/O errors at the very
first sector (LBA 0), the Partitions and File Browser tabs will not be able to read
partition or filesystem metadata.  The recommended approach is:

**Step 1 — Image the drive**

Go to the **Imaging** screen and start a session.  The 5-pass algorithm reads
everything it can, skips unreadable sectors (marking them in the mapfile), and
writes a partial `.img` file.  Even a 60% complete image contains useful data.

Key tips:
- Set a mapfile path so you can resume if Ferrite is interrupted.  Or leave Dest
  empty and press `s` — Ferrite auto-generates a filename from the drive serial
  and today's date and creates a matching mapfile alongside it.
- **If the progress bar shows 0% for more than 10 seconds,** the watchdog line will
  appear in Statistics: `⚠ No read progress for Xs — drive may be unresponsive`.
  This usually means the first sector is physically dead.  Press `c` to cancel,
  enable **Reverse** mode (`r` key), and restart — this images from the end of the
  drive backwards, collecting data from the healthy regions before hitting the dead
  zone near LBA 0.
- The built-in ERC/TLER timeout (30 s per sector) will advance past bad sectors
  automatically.  On a USB drive the xHCI host controller may hold the I/O at
  hardware level for longer than the software timeout; the watchdog will alert you
  when this happens.
- For very slow drives, imaging over multiple short sessions reduces the risk of
  complete failure mid-session.

**Step 2 — Open the image file**

Return to the **Drive Selection** screen and press `f`.  Type the full path to
the `.img` file and press `Enter`.  All other tabs now operate on the image file —
no further hardware reads hit the dying drive.

**Step 3 — Carve from the image**

Go to the **Carving** screen and start a new scan.  Because the source is now a
regular file, reads are instant and reliable.  The carver will zero-fill any sectors
that were unreadable during imaging, producing files with gaps rather than missing
files entirely.

**Step 4 — Check CarveQuality**

After extraction, hits are tagged:
- `✓` Complete — structure intact
- `~` Truncated — file reached the end of device or a footer was not found
- `✗` Corrupt — internal structure check failed (often indicates a zero-filled gap
  landed inside a critical header or CRC block)

Enable **skip-corrupt mode** (`Shift+C` on the Carving screen) to automatically
discard structurally invalid files and keep only the usable ones.

### Quick Recover finds no deleted files

- The filesystem may have been reformatted, which overwrites directory metadata.
- Use File Carving (Screen 6) instead, which works without filesystem metadata.

### Artifact Scanner or Text Scanner finds no results

- The area of the drive containing those files may have been overwritten.
- The scan covers the entire raw device; ensure the correct device is selected.
- Re-running after giving consent will restart the scan from the beginning.

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

## 22. Glossary

| Term | Definition |
|---|---|
| **Artifact** | A recoverable data fragment with forensic value, such as an email address or file path found in raw binary data |
| **Bad sector** | A disk sector that cannot be read after all retry attempts |
| **Block device** | A storage device accessed in fixed-size chunks (sectors); e.g. `/dev/sda`, `\\.\PhysicalDrive0` |
| **CarveQuality** | A post-extraction assessment of a carved file: Complete (✓), Truncated (~), Corrupt (✗), or Unknown |
| **Carving** | Extracting files from raw binary data by recognising their header/footer magic bytes, without relying on filesystem metadata |
| **CRC** | Cyclic Redundancy Check — used in GPT headers, PNG chunks, and sector checksums |
| **ddrescue** | GNU ddrescue is a data recovery tool that uses a similar multi-pass algorithm and the same mapfile format as Ferrite |
| **Direct I/O** | Reads that bypass the OS page cache and go directly to the device (`O_DIRECT` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows) |
| **ERC / TLER** | Error Recovery Control / Time-Limited Error Recovery — a drive firmware setting that caps per-sector retry time; Ferrite does not configure this but benefits when it is set |
| **Extents** | An ext4 data addressing scheme where a contiguous run of blocks is described by a single start+length record, replacing the older indirect-block scheme |
| **Footer** | A magic byte sequence at the end of a file (e.g. `FF D9` for JPEG) used by the carver to determine the file boundary |
| **GPT** | GUID Partition Table — the modern partition scheme used on UEFI systems |
| **Header** | The magic byte sequence at the start of a file used by the carver to detect file type (e.g. `FF D8 FF` for JPEG) |
| **Image file** | A byte-for-byte copy of a block device saved as a regular file |
| **LBA** | Logical Block Address — a zero-based index of a 512-byte (or 4 KiB) sector on a block device |
| **Luhn algorithm** | A checksum formula used to validate credit card numbers |
| **Mapfile** | A text file tracking the copy status of every sector; compatible with GNU ddrescue |
| **MBR** | Master Boot Record — the legacy partition table format stored in the first 512 bytes of a device |
| **MFT** | Master File Table — the central metadata store in NTFS |
| **NVMe** | Non-Volatile Memory Express — a protocol for SSDs connected over PCIe |
| **PII** | Personally Identifiable Information — data that can identify an individual, such as email addresses or financial account numbers |
| **RecoveryChance** | An enum assigned to deleted filesystem entries: High / Medium / Low / Unknown, based on whether the data clusters are still intact |
| **S.M.A.R.T.** | Self-Monitoring, Analysis and Reporting Technology — firmware-level drive health monitoring |
| **Sector** | The smallest addressable unit on a block device (typically 512 bytes; some modern drives use 4 KiB) |
| **Sector alignment** | The requirement that read offsets and buffer sizes are multiples of the sector size when using direct I/O |
| **SHA-256** | A cryptographic hash function; Ferrite writes a `.sha256` sidecar file after imaging to allow integrity verification |
| **Signature** | In file carving, the combination of a header magic, optional footer magic, and maximum size that identifies a file type |
| **Size hint** | A format-specific algorithm that reads structural metadata (e.g. PE headers, PNG chunk lengths) to determine the exact length of a carved file before extraction |
| **smartctl** | The command-line tool from the smartmontools package used to query S.M.A.R.T. data |
| **TextKind** | A content classification assigned to extracted text blocks: Prose, Code, CSV, Log, Config, Html, Json, Path, or Other |
| **ThermalGuard** | Ferrite's background imaging safety system that auto-pauses imaging when drive temperature exceeds a configurable threshold |
| **Write-blocker** | A hardware or software mechanism that prevents writes to a source device; Ferrite performs a software pre-flight check on device selection |
