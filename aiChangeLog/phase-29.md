# Phase 29 — Auto-Extract Flow Control, I/O Optimisation & Signature Hardening

## Overview
Three interrelated improvements driven by real-world testing on a 4 TB USB HDD:
1. Back-pressure scan gating to prevent the auto-extract queue growing unboundedly
2. Separate pause flags so scan gating never stalls extraction workers
3. Extraction I/O optimisation (sort by offset, single worker) for spinning disks
4. Signature quality review — reduced false positives and capped disk-waste exposure

---

## Task 1 — Auto-Extract Back-Pressure (Phase 28 follow-up)

**Problem:** Scanner produces hits orders of magnitude faster than extraction.
With auto-extract on the queue grew without bound, eventually causing OOM and
making the UI unresponsive.

**Solution:** High/low water marks + extraction-in-flight trigger.

### Constants (`mod.rs`)
- `AUTO_EXTRACT_HIGH_WATER = 100` — pause scan when queue exceeds this
- `AUTO_EXTRACT_LOW_WATER  = 10`  — resume scan when queue drains below this

### New field (`CarvingState`)
- `backpressure_paused: bool` — tracks whether the scan is paused by
  back-pressure (vs. manual user pause)

### Logic (`events.rs`)
- `HitBatch` handler: engage back-pressure immediately when `extract_progress.is_some()`
  (primary trigger) OR queue > `HIGH_WATER` (secondary safety net)
- `ExtractionDone` handler: lift back-pressure before starting next batch, then
  re-check (scan resumes between batches)

### Logic (`extract.rs` — `pump_auto_extract`)
- Low-water resume: clear `backpressure_paused` + clear `pause` AtomicBool when
  queue < `LOW_WATER` and status is `Running`

### Logic (`input.rs` — `toggle_pause` / `cancel_scan`)
- Manual `p` press clears `backpressure_paused` so user can always override
- `cancel_scan` clears `backpressure_paused` so the scan thread unblocks

### UI (`render.rs`)
- Status bar: `" scan paused — queue full "` (cyan) when back-pressure is active
- Compact scan row: `⏳` prefix + queue depth hint while back-pressure is engaged

---

## Task 2 — Separate Scan / Extraction Pause Flags

**Problem:** A single `pause: Arc<AtomicBool>` was shared by both the scan thread
and extraction workers.  Setting it for back-pressure accidentally froze extraction.

**Solution:** Split into two independent flags.

| Flag | Owner | Purpose |
|------|-------|---------|
| `pause` | Scan thread only | Back-pressure gating + manual scan pause |
| `extract_pause` | Extraction workers only | Manual `p` pause during extraction |

### Files changed
- `mod.rs` — added `extract_pause: Arc<AtomicBool>` field, init + reset
- `extract.rs` — `start_extraction_batch` uses `extract_pause` for workers
- `input.rs` — `toggle_pause` during extraction uses `extract_pause`
- `render.rs` — extraction progress panel reads `extract_pause` for paused state
- `events.rs` — `ExtractionDone` clears both flags independently

---

## Task 3 — Extraction I/O Optimisation

**Problem:** On a USB HDD at 90-100% utilisation, scan was running at 1.5 MB/s
(should be 80-150 MB/s).  Multiple extraction workers issuing I/O at random offsets
caused constant head seeks.

**Root cause:** Work items were extracted in queue-arrival order (essentially random
disk offsets), and `clamp(2, 8)` concurrency forced the HDD head to serve multiple
seek targets simultaneously.

### Changes (`extract.rs` — `start_extraction_batch`)

**Sort by byte offset:**
```rust
work.sort_unstable_by_key(|(_, hit, _)| hit.byte_offset);
```
Turns random seeks into a near-sequential forward pass — the single largest
throughput improvement available for single-drive recovery.

**Concurrency → 1:**
```rust
let concurrency = 1;
```
One worker draining the sorted queue reads in address order.  Multiple workers
at different offsets on the same HDD is strictly worse than serial.  Also gentler
on potentially damaged recovery targets.

---

## Task 4 — Signature Quality Review

Full review of all 28 signatures for false-positive risk and disk-waste exposure.
Applied the following changes to `config/signatures.toml`:

| Signature | Change | Reason |
|-----------|--------|--------|
| **PE Executable** | Header `4D 5A` → `4D 5A 90 00` | "MZ" (2 bytes) had same false-positive problem as old BMP. `90 00` is the standard MSVC linker stub — covers ~99% of modern PE files |
| **PE Executable** | max_size 500 MiB → 100 MiB, `min_size = 4096` | Most EXE/DLL files are well under 100 MiB |
| **MP4 Video** | Header `00 00 00 20 66 74 79 70` → `?? ?? ?? ?? 66 74 79 70` | Previous header only matched ftyp box size = exactly 32 bytes, missing most MP4/MOV/M4A variants. Wildcard size, anchor "ftyp" |
| **MP4 Video** | Added `min_size = 4096` | Reject tiny false positive hits |
| **ZIP / Office** | max_size 500 MiB → 100 MiB, `min_size = 512` | Every local-file-header inside an existing ZIP generates a hit; unfootered ZIPs were each wasting 500 MiB |
| **Email (EML)** | **Removed** | `"From "` (5 bytes) appears in any English text, HTML, source code. Output is an mbox fragment, not a usable standalone email |
| **XML Document** | max_size 50 MiB → 5 MiB, `min_size = 256` | XML fragments appear embedded inside DOCX/Office files; standalone XML worth recovering is almost always small |
| **MKV Video** | max_size 40 GiB → 8 GiB | Covers typical 1080p films; limits disk waste on false positives |
| **VMDK Disk Image** | max_size 100 GiB → 10 GiB, `min_size = 65536` | Niche use case; 100 GiB cap created catastrophic waste on false positives |
| **Outlook PST/OST** | Added `min_size = 65536` | 4-byte false positive was triggering 20 GiB extraction attempts |
| **BMP Image** | (Phase 28) Header `42 4D` → `42 4D ?? ?? ?? ?? 00 00 00 00` | Reserved bytes always zero — anchoring eliminated near-BMP-level false positive rate |
| **MP3 Audio** | (Phase 28) Header `49 44 33` → `49 44 33 ?? 00`, `min_size = 4096` | Revision byte is always 0x00 in all ID3v2 versions |

Unchanged (header length, discriminators, and caps appropriate):
JPEG JFIF, JPEG Exif, PNG, PDF, GIF, RAR, 7-Zip, SQLite, WAV, AVI, OGG,
Windows Event Log, FLAC, RTF, HTML, vCard, iCalendar, OLE2 Compound

Updated count assertion in `ferrite-carver/src/lib.rs`: 28 → 27 (EML removed).

---

## Files Modified

| File | Change |
|------|--------|
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `AUTO_EXTRACT_HIGH/LOW_WATER` constants; `backpressure_paused` + `extract_pause` fields |
| `crates/ferrite-tui/src/screens/carving/events.rs` | Back-pressure engage/lift logic; `ExtractionDone` flag handling |
| `crates/ferrite-tui/src/screens/carving/extract.rs` | Sort-by-offset; concurrency=1; `extract_pause`; `pump_auto_extract` low-water resume |
| `crates/ferrite-tui/src/screens/carving/input.rs` | `toggle_pause` / `cancel_scan` clear `backpressure_paused`; extraction uses `extract_pause` |
| `crates/ferrite-tui/src/screens/carving/render.rs` | Back-pressure status label; `render_compact_scan_progress` queue hint; extraction panel uses `extract_pause` |
| `config/signatures.toml` | Signature hardening — see table above |
| `crates/ferrite-carver/src/lib.rs` | Updated signature count assertion 28 → 27 |

## Test Results
- `cargo build --workspace` — clean
- `cargo test --workspace` — all tests pass
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — clean
