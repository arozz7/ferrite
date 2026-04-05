# Phase 123 — False-Positive Suppression & I/O Strain Reduction

## Problem Statement

An 80 GB disk image produced ~2.9 million carving hits, the vast majority
attributable to ADTS (AAC audio) false positives.  Each false positive
triggered a full write+delete cycle on the output drive (File::create →
carver.extract returning Ok(0) → remove_file), causing enormous I/O strain
and inflating the "Pending" counter into the millions.

## Root Causes Identified

1. **ADTS pre-validator too weak** — only checked one frame header; random
   data satisfying a 2-byte sync word (FF F1 / FF F9) plus a valid
   sampling_freq_index passed without verifying the claimed frame length led
   to the next frame.
2. **No pre-flight gate before File::create** — for skip_on_failure signatures
   (Adts), the frame-walker hint ran *after* creating the output file.
3. **No API for in-memory extraction** — library had no public surface to
   extract to a Vec<u8> for small-file pre-validation.
4. **No user-visible warning** — when hit density exceeded 500 K the user had
   no indication that false positives were likely overwhelming real hits.

## Changes

### Phase 1 — ADTS 3-frame chain validation (`ferrite-carver/src/pre_validate.rs`)

- `validate_aac(data, pos)` now chains **3 consecutive ADTS frame verifications**
  within the in-memory scan buffer before accepting a hit:
  - **Frame 1**: layer=00, SFI ∈ [0,12], frame_len ∈ [7, 8191]
  - **Frame 2**: valid sync word + SFI == frame 1 SFI + frame_len ∈ [7, 8191]
  - **Frame 3**: valid sync word only
- `need()` returns true (conservative accept) when the buffer is too short to
  reach frame 2 or 3 — avoids false negatives at 4 MiB chunk boundaries.
- Two new helper functions: `adts_frame_len(data, pos)` and
  `adts_sync_valid(data, pos)`.
- **6 new tests** (total AAC tests: 16):
  - `aac_three_frame_chain_accepted`
  - `aac_broken_chain_frame2_bad_sync_rejected`
  - `aac_broken_chain_frame2_sfi_mismatch_rejected`
  - `aac_broken_chain_frame3_bad_sync_rejected`
  - `aac_conservative_accept_at_chunk_boundary_frame2`
  - `aac_conservative_accept_at_chunk_boundary_frame3`

### Phase 2 — Short-pattern signature audit (no code changes)

Enumerated all 9 signatures with header ≤ 3 bytes:
- **TS** (1 byte): 3–5 sync bytes at 188-byte stride ✓
- **PCX** (1 byte): 68-byte header with 6 independent field checks ✓
- **GZip** (2 bytes): CM=8 + FLG reserved + XFL + OS whitelist ✓
- **AAC×2** (2 bytes): upgraded to 3-frame chain (Phase 1) ✓
- **Shebang** (2 bytes): `#!/` + `b`/`u` + 3 printable bytes ✓
- **SWF×3** (3 bytes): version range + file_len [21, 100MB] ✓

All validators already adequate; no changes required.

### Phase 3 — Pre-flight skip_on_failure gate (`ferrite-carver/src/scanner.rs`, `ferrite-tui/src/screens/carving/extract.rs`)

**`Carver::is_viable_hit(&self, hit: &CarveHit) -> bool`** (new public method):
- For `skip_on_failure()` hints (only `SizeHint::Adts` currently), reads the
  device and runs the frame-walker before returning.  Returns `false` if the
  walker returns `None` (definitive false positive).
- For all other signatures returns `true` unconditionally.

**Wired into both extraction paths in `extract.rs`**:
- Single-hit path (line ~379): if `!carver.is_viable_hit(&hit)` → send
  `CarveMsg::Skipped` and return; no file created.
- Batch worker path: moved `is_viable_hit` check BEFORE `WorkerMsg::Started`,
  so a false positive skips the `ExtractionStarted` notification entirely.

### Phase 4 — In-memory extraction API (`ferrite-carver/src/scanner.rs`)

**`Carver::extract_to_vec(&self, hit: &CarveHit) -> Result<Vec<u8>>`** (new public method):
- Delegates to `extract()` writing into a `Vec<u8>`.
- Returns empty vec when `extract()` returns `Ok(0)`.
- Enables future callers to validate content before committing to disk.

Note: Only 1 signature (XML, 512 KB) currently falls under a 512 KB in-memory
threshold; full batch-path in-memory extraction not wired (minimal benefit).

### Phase 5 — High hit density warning (`ferrite-tui/src/screens/carving/`)

**`HIGH_DENSITY_THRESHOLD = 500_000`** constant in `mod.rs`.

**`CarvingState::render_high_density_warning(frame, area)`** in `render_progress.rs`:
- Yellow-on-black banner shown when `total_hits_scanned >= HIGH_DENSITY_THRESHOLD`.
- Detects whether any enabled signature has extension `aac` or name containing
  `ADTS`; if so, names AAC specifically in the advice string.

**Wired into `render_hits_panel`** in `render.rs`:
- Inserted as a 1-row layout slice between the compact scan-progress line and
  the extraction overview gauge.
- Visible during scan and after scan completes.

## Files Changed

| File | Change |
|------|--------|
| `crates/ferrite-carver/src/pre_validate.rs` | 3-frame ADTS chain validator + 6 tests |
| `crates/ferrite-carver/src/scanner.rs` | `is_viable_hit()`, `extract_to_vec()` |
| `crates/ferrite-tui/src/screens/carving/extract.rs` | Pre-flight check in single-hit + batch paths |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | `HIGH_DENSITY_THRESHOLD` constant |
| `crates/ferrite-tui/src/screens/carving/render.rs` | Warning banner wired into layout |
| `crates/ferrite-tui/src/screens/carving/render_progress.rs` | `render_high_density_warning()` |

## Test Results

- `cargo test --workspace`: **1129 passed, 0 failed**
- `cargo clippy --workspace -- -D warnings`: clean
- `cargo fmt --all`: formatted
