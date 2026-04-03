//! Extraction helpers for [`CarvingState`] — split from `mod.rs` to keep
//! file sizes under the project hard limit (600 lines).

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sha2::{Digest, Sha256};

use ferrite_blockdev::{AlignedBuffer, BlockDevice};
use ferrite_carver::{post_validate, CarveHit, CarveQuality, Carver, CarvingConfig};
use ferrite_filesystem::MetadataIndex;

use super::{CarveMsg, CarveStatus, CarvingState, ExtractProgress, HitStatus};

// ── Recovered-name helpers ────────────────────────────────────────────────────

/// Extract the target filename from a Windows Shell Link (.lnk) binary blob.
///
/// Navigates the LinkInfo structure to find `LocalBasePath + CommonPathSuffix`,
/// then returns the last path component.  Returns `None` when the data is too
/// short, has an invalid header, or has no `HasLinkInfo` flag.
pub(super) fn parse_lnk_target_name(data: &[u8]) -> Option<String> {
    if data.len() < 76 {
        return None;
    }
    // HeaderSize must be 0x4C000000 (LE = 76).
    if data[0..4] != [0x4C, 0x00, 0x00, 0x00] {
        return None;
    }
    let link_flags = u32::from_le_bytes(data[0x14..0x18].try_into().ok()?);
    let has_idlist = link_flags & 0x01 != 0;
    let has_link_info = link_flags & 0x02 != 0;
    if !has_link_info {
        return None;
    }

    // Offset to the LinkInfo block: skip the optional IDList.
    let mut li = 76usize;
    if has_idlist {
        if li + 2 > data.len() {
            return None;
        }
        let idlist_size = u16::from_le_bytes([data[li], data[li + 1]]) as usize;
        li += 2 + idlist_size;
    }

    // LinkInfo header (28 bytes minimum).
    if li + 28 > data.len() {
        return None;
    }
    let li_flags = u32::from_le_bytes(data[li + 8..li + 12].try_into().ok()?);
    // Bit 0: VolumeIDAndLocalBasePath — required for a local file path.
    if li_flags & 0x01 == 0 {
        return None;
    }
    let base_off = u32::from_le_bytes(data[li + 16..li + 20].try_into().ok()?) as usize;
    let suffix_off = u32::from_le_bytes(data[li + 24..li + 28].try_into().ok()?) as usize;

    let base_start = li + base_off;
    let suffix_start = li + suffix_off;
    if base_start >= data.len() {
        return None;
    }
    let base = read_ansi_nul(&data[base_start..]);
    let suffix = if suffix_start < data.len() {
        read_ansi_nul(&data[suffix_start..])
    } else {
        String::new()
    };
    let full = format!("{}{}", base, suffix);
    full.split(['\\', '/'])
        .rfind(|s| !s.is_empty())
        .map(str::to_string)
}

/// Extract the executable name from a Windows Prefetch (.pf) binary blob.
///
/// The canonical Prefetch filename is `<EXECNAME>-<HASH>.pf`.  This function
/// returns `EXECNAME` in lower-case from the fixed UTF-16LE field at offset
/// 0x10 (60 bytes, null-terminated).
pub(super) fn parse_pf_exe_name(data: &[u8]) -> Option<String> {
    // Need at least 0x4A bytes (header through end of exe-name field).
    if data.len() < 0x4A {
        return None;
    }
    // Bytes 4..8 must be the ASCII magic "SCCA".
    if data[4..8] != [0x53, 0x43, 0x43, 0x41] {
        return None;
    }
    // ExecutableName: 60 bytes at 0x10, UTF-16LE, null-terminated.
    let words: Vec<u16> = data[0x10..0x4A]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&w| w != 0)
        .collect();
    if words.is_empty() {
        return None;
    }
    let name = String::from_utf16_lossy(&words);
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    if safe.is_empty() {
        None
    } else {
        Some(safe.to_lowercase())
    }
}

/// Read a file's first `n` bytes and parse a recovered name for LNK/PF formats.
fn recovered_name_from_file(path: &str, ext: &str) -> Option<String> {
    // Read enough bytes for the header structures (LNK needs up to ~4 KiB for
    // LinkInfo; PF only needs 74 bytes — we read 4 KiB for both).
    let data = read_file_head(path, 4096);
    match ext {
        "lnk" => parse_lnk_target_name(&data),
        "pf" => parse_pf_exe_name(&data),
        _ => None,
    }
}

/// After extracting an LNK or PF file, try to rename it to a meaningful name.
///
/// On success the file is renamed to `<recovered_name>[r].<ext>` in the same
/// directory; the `[r]` suffix marks it as a heuristic rename.  If parsing
/// fails, the name is already taken, or the rename errors, the original path
/// is returned unchanged.
fn try_recovered_rename(path: &str, ext: &str) -> String {
    let Some(base) = recovered_name_from_file(path, ext) else {
        return path.to_string();
    };
    // Strip any trailing dot from the base name before appending the extension.
    let safe_base: String = base
        .trim_end_matches('.')
        .chars()
        .map(|c| {
            if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if safe_base.is_empty() {
        return path.to_string();
    }
    let new_filename = format!("{}[r].{}", safe_base, ext);
    let new_path = match std::path::Path::new(path).parent() {
        Some(p) if !p.as_os_str().is_empty() => {
            p.join(&new_filename).to_string_lossy().into_owned()
        }
        _ => new_filename,
    };
    if std::path::Path::new(&new_path).exists() {
        return path.to_string();
    }
    if std::fs::rename(path, &new_path).is_ok() {
        tracing::debug!(old = %path, new = %new_path, "recovered filename from carved content");
        new_path
    } else {
        path.to_string()
    }
}

fn read_ansi_nul(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

// ── CancelWriter ──────────────────────────────────────────────────────────────

/// Wraps any `Write` implementor and checks the `cancel` flag on every `write()`
/// call.  When the flag is set the write returns `ErrorKind::Interrupted`, which
/// propagates up through `carver.extract()` so large-file extractions abort at
/// the next chunk boundary rather than running to completion.
struct CancelWriter<W: Write> {
    inner: W,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl<W: Write> Write for CancelWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "extraction cancelled",
            ));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

// ── Timestamp helper ──────────────────────────────────────────────────────────

/// Apply the original file timestamps to `path` using the metadata index.
///
/// Looks up `byte_offset` in `index`; if a match with a modification (or
/// creation) timestamp is found, sets both the access time and the
/// modification time on the file.  Errors are silently ignored — timestamp
/// stamping is best-effort on damaged media.
fn apply_timestamps(path: &str, byte_offset: u64, index: &MetadataIndex) {
    let meta = match index.lookup(byte_offset) {
        Some(m) => m,
        None => return,
    };
    let unix_secs = match meta.modified.or(meta.created) {
        Some(t) => t,
        None => return,
    };
    let ft = filetime::FileTime::from_unix_time(unix_secs as i64, 0);
    if let Err(e) = filetime::set_file_times(path, ft, ft) {
        tracing::debug!(path, error = %e, "could not set file timestamps");
    }
}

// ── SHA-256 sidecar ───────────────────────────────────────────────────────────

/// Write a `<path>.sha256` sidecar file containing the SHA-256 hash of the
/// extracted file in GNU `sha256sum`-compatible format:
///
/// ```text
/// <hex>  <filename>\n
/// ```
///
/// Errors are silently ignored — sidecar generation is best-effort and must
/// never abort an otherwise-successful extraction.
pub(super) fn write_sha256_sidecar(path: &str) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return,
    };
    let hash = Sha256::digest(&data);
    let hex = format!("{:x}", hash);
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    let sidecar_path = format!("{}.sha256", path);
    let content = format!("{}  {}\n", hex, filename);
    let _ = std::fs::write(&sidecar_path, content);
}

// ── Integrity helpers ─────────────────────────────────────────────────────────

/// Read up to `max_bytes` from the **tail** of `path`.
///
/// Used for post-extraction structural validation.  If the file is shorter
/// than `max_bytes`, the entire file is returned.  Errors return an empty Vec.
fn read_file_tail(path: &str, max_bytes: u64) -> Vec<u8> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let file_len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let skip = file_len.saturating_sub(max_bytes);
    if skip > 0 && f.seek(std::io::SeekFrom::Start(skip)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    buf
}

/// Read up to `max_bytes` from the **head** (beginning) of `path`.
///
/// Used for post-extraction CRC-32 verification of early chunks (PNG).
fn read_file_head(path: &str, max_bytes: usize) -> Vec<u8> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut buf = vec![0u8; max_bytes];
    match f.read(&mut buf) {
        Ok(n) => {
            buf.truncate(n);
            buf
        }
        Err(_) => Vec::new(),
    }
}

/// Compute a fast u64 fingerprint from the first 4 KiB of a hit on `device`.
///
/// Uses `std::hash::DefaultHasher` — sufficient for probabilistic duplicate
/// detection; not a cryptographic hash.  Returns `None` when the device read
/// fails (the hit will NOT be treated as a duplicate in that case).
fn hit_fingerprint(device: &dyn ferrite_blockdev::BlockDevice, byte_offset: u64) -> Option<u64> {
    const FP_BYTES: usize = 4096;
    let mut buf = AlignedBuffer::new(FP_BYTES, 512);
    let n = device.read_at(byte_offset, &mut buf).ok()?;
    if n == 0 {
        return None;
    }
    let data = &buf.as_slice()[..n];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    Some(hasher.finish())
}

// ── Extraction impl ────────────────────────────────────────────────────────────

impl CarvingState {
    /// Build an output path for `hit` inside `dir`.
    ///
    /// When the metadata index resolves the byte offset to a filesystem entry,
    /// the original relative path is preserved: `<dir>/<original/sub/path>`.
    /// This recreates the original folder structure under the output directory.
    /// Falls back to `ferrite_<ext>_<offset>.<ext>` when no metadata exists.
    pub(super) fn filename_for_hit(&self, hit: &CarveHit, dir: &str) -> String {
        if let Some(idx) = &self.meta_index {
            if let Some(meta) = idx.lookup(hit.byte_offset) {
                // Build a safe relative path from the original filesystem path,
                // stripping leading separators and rejecting `..` traversal.
                let rel: std::path::PathBuf = meta
                    .path
                    .trim_start_matches('/')
                    .trim_start_matches('\\')
                    .split(['/', '\\'])
                    .filter(|s| !s.is_empty() && *s != "..")
                    .map(|s| s.replace([':', '*', '?', '<', '>', '|', '"'], "_"))
                    .collect();
                if rel.components().count() > 0 {
                    return std::path::Path::new(dir)
                        .join(rel)
                        .to_string_lossy()
                        .into_owned();
                }
                // Fallback: sanitised bare name when path is unusable.
                let safe = meta
                    .name
                    .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
                if !safe.is_empty() {
                    return format!("{dir}\\{safe}");
                }
            }
        }
        format!(
            "{dir}\\ferrite_{}_{}.{}",
            hit.signature.extension, hit.byte_offset, hit.signature.extension
        )
    }

    pub(super) fn extract_selected(&mut self) {
        let entry = match self.hits.get_mut(self.hit_sel) {
            Some(e) => e,
            None => return,
        };
        let hit = entry.hit.clone();
        entry.status = HitStatus::Extracting;

        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        let tx = match &self.tx {
            Some(t) => t.clone(),
            None => return,
        };
        let idx = self.hit_sel;

        let dir = if self.output_dir.is_empty() {
            "carved".to_string()
        } else {
            self.output_dir.clone()
        };
        let filename = self.filename_for_hit(&hit, &dir);
        let meta_index = self.meta_index.clone();
        let seen_fingerprints = Arc::clone(&self.seen_fingerprints);
        let skip_truncated_single = self.skip_truncated;
        let skip_corrupt_single = self.skip_corrupt;
        let config = CarvingConfig {
            signatures: vec![hit.signature.clone()],
            scan_chunk_size: 4 * 1024 * 1024,
            start_byte: 0,
            end_byte: None,
        };
        std::thread::spawn(move || {
            // Duplicate check: fingerprint the first 4 KiB at the hit offset.
            if let Some(fp) = hit_fingerprint(device.as_ref(), hit.byte_offset) {
                let mut set = seen_fingerprints.lock().unwrap();
                if !set.insert(fp) {
                    // Already seen — skip extraction.
                    let _ = tx.send(CarveMsg::Duplicate { idx });
                    return;
                }
            }

            // Ensure the output directory (and any metadata-derived subdir) exists.
            let file_parent = std::path::Path::new(&filename)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from(&dir));
            if let Err(e) = std::fs::create_dir_all(&file_parent) {
                tracing::warn!(dir = %dir, error = %e, "failed to create output directory");
                return;
            }
            let carver = Carver::new(device, config);
            if let Ok(mut f) = std::fs::File::create(&filename) {
                match carver.extract(&hit, &mut f) {
                    Ok(0) => {
                        // Size-hint resolved below min_size — remove empty file.
                        let _ = std::fs::remove_file(&filename);
                        let _ = tx.send(CarveMsg::Skipped { idx });
                    }
                    Ok(bytes) => {
                        // Post-extraction min_size check: the pre-extraction
                        // check compares against the *planned* extraction size
                        // (max_size for footer-based formats), which is always
                        // large.  The actual extracted bytes may be much smaller
                        // when the footer is found early (e.g. tiny JPEG
                        // thumbnails, GIF spacer pixels).
                        if hit.signature.min_size > 0 && bytes < hit.signature.min_size {
                            let _ = std::fs::remove_file(&filename);
                            let _ = tx.send(CarveMsg::Skipped { idx });
                            return;
                        }
                        let truncated = bytes >= hit.signature.max_size;
                        if let Some(ref meta_idx) = meta_index {
                            apply_timestamps(&filename, hit.byte_offset, meta_idx);
                        }
                        let quality = if !truncated
                            && matches!(
                                hit.signature.extension.as_str(),
                                "png"
                                    | "pdf"
                                    | "db"
                                    | "evtx"
                                    | "wav"
                                    | "avi"
                                    | "webp"
                                    | "aiff"
                                    | "exe"
                                    | "flac"
                                    | "elf"
                                    | "regf"
                                    | "tif"
                                    | "nef"
                                    | "arw"
                                    | "cr2"
                                    | "rw2"
                                    | "orf"
                                    | "pef"
                                    | "sr2"
                                    | "dcr"
                                    | "mp4"
                                    | "mov"
                                    | "m4v"
                                    | "3gp"
                                    | "m4a"
                                    | "heic"
                                    | "cr3"
                                    | "mkv"
                                    | "webm"
                            ) {
                            match hit.signature.extension.as_str() {
                                "png" => post_validate::validate_png_file(Path::new(&filename)),
                                "pdf" => post_validate::validate_pdf_file(Path::new(&filename)),
                                "db" => post_validate::validate_sqlite_file(Path::new(&filename)),
                                "evtx" => post_validate::validate_evtx_file(Path::new(&filename)),
                                "wav" | "avi" | "webp" | "aiff" => {
                                    post_validate::validate_riff_file(Path::new(&filename))
                                }
                                "exe" => post_validate::validate_exe_file(Path::new(&filename)),
                                "flac" => post_validate::validate_flac_file(Path::new(&filename)),
                                "elf" => post_validate::validate_elf_file(Path::new(&filename)),
                                "regf" => post_validate::validate_regf_file(Path::new(&filename)),
                                "mp4" | "mov" | "m4v" | "3gp" | "m4a" | "heic" | "cr3" => {
                                    post_validate::validate_isobmff_file(Path::new(&filename))
                                }
                                "mkv" | "webm" => {
                                    post_validate::validate_ebml_file(Path::new(&filename))
                                }
                                _ => post_validate::validate_tiff_file(Path::new(&filename)),
                            }
                        } else {
                            let head = read_file_head(&filename, 8192);
                            let tail = read_file_tail(&filename, 65536);
                            post_validate::validate_extracted(
                                &hit.signature.extension,
                                &head,
                                &tail,
                                truncated,
                                bytes,
                            )
                        };
                        if skip_truncated_single && matches!(quality, CarveQuality::Truncated) {
                            let _ = std::fs::remove_file(&filename);
                            let _ = tx.send(CarveMsg::Skipped { idx });
                        } else if skip_corrupt_single && matches!(quality, CarveQuality::Corrupt) {
                            let _ = std::fs::remove_file(&filename);
                            let _ = tx.send(CarveMsg::SkippedCorrupt { idx });
                        } else {
                            let filename = match hit.signature.extension.as_str() {
                                "lnk" | "pf" => {
                                    try_recovered_rename(&filename, &hit.signature.extension)
                                }
                                _ => filename,
                            };
                            write_sha256_sidecar(&filename);
                            tracing::info!(path = %filename, bytes, "extracted file");
                            let _ = tx.send(CarveMsg::Extracted {
                                idx,
                                bytes,
                                truncated,
                                quality,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(path = %filename, error = %e, "extraction failed");
                    }
                }
            }
        });
    }

    pub(super) fn extract_all_selected(&mut self) {
        // Already extracting — don't start a second batch.
        if self.extract_progress.is_some() {
            return;
        }
        let dir = if self.output_dir.is_empty() {
            "carved".to_string()
        } else {
            self.output_dir.clone()
        };

        // Collect work items: (global_index, hit, output_path)
        let work: Vec<(usize, CarveHit, String)> = self
            .hits
            .iter()
            .enumerate()
            .filter(|(_, e)| e.selected && matches!(e.status, HitStatus::Unextracted))
            .map(|(idx, e)| {
                let path = self.filename_for_hit(&e.hit, &dir);
                (idx, e.hit.clone(), path)
            })
            .collect();

        self.start_extraction_batch(work);
    }

    /// Start a new extraction batch from the next unprocessed hits in
    /// `self.hits`, if no batch is currently in flight.
    ///
    /// Uses a lazy-pull model: `next_auto_extract_idx` tracks how far through
    /// `self.hits` we have submitted to extraction.  This keeps the effective
    /// "queue depth" bounded to one batch (≤500 items) regardless of how many
    /// hits a single dense scan chunk produces.
    ///
    /// Back-pressure (scan pause) is applied after starting the batch when
    /// more hits are waiting beyond the current batch window.
    pub(super) fn pump_auto_extract(&mut self) {
        if self.extract_progress.is_some() {
            // A batch is already in flight — ensure back-pressure is active so
            // the scan stays paused.  This catches the case where the index
            // caught up to hits.len() (so the batch was started without the
            // previous back-pressure check firing), and new hits have since
            // arrived while the batch is still running.
            if !self.backpressure_paused && self.status == CarveStatus::Running {
                self.pause.store(true, Ordering::Relaxed);
                self.backpressure_paused = true;
            }
            return;
        }

        let dir = if self.output_dir.is_empty() {
            "carved".to_string()
        } else {
            self.output_dir.clone()
        };

        // Collect up to BATCH Unextracted hits starting from next_auto_extract_idx.
        // Hits that are already extracted (from a resumed session) are skipped.
        const BATCH: usize = 500;
        let mut work: Vec<(usize, CarveHit, String)> = Vec::new();
        while work.len() < BATCH && self.next_auto_extract_idx < self.hits.len() {
            let i = self.next_auto_extract_idx;
            self.next_auto_extract_idx += 1;
            if self.hits[i].status != HitStatus::Unextracted {
                continue;
            }
            let hit = self.hits[i].hit.clone();
            let path = self.filename_for_hit(&hit, &dir);
            work.push((i, hit, path));
        }

        if work.is_empty() {
            // All known hits have been submitted — lift back-pressure so the
            // scan can find new hits.  If hits arrive during the next batch
            // the early-return block above re-applies back-pressure.
            if self.backpressure_paused {
                self.backpressure_paused = false;
                if self.status == CarveStatus::Running {
                    self.pause.store(false, Ordering::Relaxed);
                }
            }
            return;
        }

        self.start_extraction_batch(work);

        // Always pause the scan while any extraction batch is in flight.
        // Without this, the scan races ahead between the time back-pressure
        // is "applied" and the time the scan thread actually checks the flag,
        // producing more hits per burst than each 500-item batch can consume,
        // causing the pending queue to grow unboundedly over time.
        if !self.backpressure_paused && self.status == CarveStatus::Running {
            self.pause.store(true, Ordering::Relaxed);
            self.backpressure_paused = true;
        }
    }

    /// Core extraction coordinator: takes a pre-built work list and starts
    /// the async extraction pipeline.
    ///
    /// `work` is `(hit_idx, hit, output_path)`.  Use `usize::MAX` as `hit_idx`
    /// for hits not in `self.hits` (beyond `DISPLAY_CAP`); status updates for
    /// those hits are silently ignored.
    pub(super) fn start_extraction_batch(&mut self, work: Vec<(usize, CarveHit, String)>) {
        if work.is_empty() {
            return;
        }
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        // For file-backed image sources, open a second independent handle so
        // the extractor thread and the scanner thread can call read_at
        // concurrently without blocking each other on the shared Mutex<File>.
        let device = device
            .try_clone_handle()
            .unwrap_or_else(|| Arc::clone(&device));
        let tx = match &self.tx {
            Some(t) => t.clone(),
            None => return,
        };
        let dir = if self.output_dir.is_empty() {
            "carved".to_string()
        } else {
            self.output_dir.clone()
        };
        // Sort by byte offset so extraction reads the source drive in forward
        // address order.  On spinning disks (including USB HDDs) this converts
        // random head seeks into a near-sequential pass, which is the single
        // biggest throughput improvement available for single-drive recovery.
        let mut work = work;
        work.sort_unstable_by_key(|(_, hit, _)| hit.byte_offset);

        let total = work.len();

        // Mark displayable hits as Queued.
        for (idx, _, _) in &work {
            if *idx != usize::MAX {
                if let Some(e) = self.hits.get_mut(*idx) {
                    e.status = HitStatus::Queued;
                }
            }
        }

        self.extract_cancel.store(false, Ordering::Relaxed);
        self.extract_pause.store(false, Ordering::Relaxed);
        // In auto-extract mode the summary accumulates across all batches so the
        // user sees a running total rather than rapid per-file flicker.  For
        // manual batch extractions we reset each time so the summary reflects
        // only the files the user explicitly requested.
        if !self.auto_extract {
            self.extract_summary = None;
        }
        let cancel = Arc::clone(&self.extract_cancel);
        let pause = Arc::clone(&self.extract_pause);
        let meta_index = self.meta_index.clone();
        let seen_fingerprints = Arc::clone(&self.seen_fingerprints);
        let skip_truncated = self.skip_truncated;
        let skip_corrupt = self.skip_corrupt;

        self.extract_progress = Some(ExtractProgress {
            done: 0,
            total,
            total_bytes: 0,
            last_name: String::new(),
            start: Instant::now(),
        });

        // Single extraction worker.  Extraction involves creating many small
        // files, which on an HDD (including external USB HDDs) generates random
        // metadata I/O (MFT updates, directory entries) that serialises at the
        // platter regardless of how many writers are open.  Multiple concurrent
        // writers multiply the seek penalty and actually reduce throughput.
        // The scanner and extractor now use independent file handles (see
        // try_clone_handle above), so the scanner is never blocked by writes.
        let worker_devices: Vec<Arc<dyn BlockDevice>> = vec![Arc::clone(&device)];

        // Shared work queue drained by all workers.
        let queue: Arc<Mutex<VecDeque<(usize, CarveHit, String)>>> =
            Arc::new(Mutex::new(work.into()));

        // Coordinator thread: spawns workers, collects per-file results, forwards
        // progress to the TUI via the private done channel.
        std::thread::spawn(move || {
            let extract_start = Instant::now();
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(dir = %dir, error = %e, "failed to create output directory");
                let _ = tx.send(CarveMsg::ExtractionDone {
                    succeeded: 0,
                    truncated: 0,
                    failed: 0,
                    duplicates: 0,
                    skipped_trunc: 0,
                    skipped_corrupt: 0,
                    total_bytes: 0,
                    elapsed_secs: extract_start.elapsed().as_secs_f64(),
                });
                return;
            }

            enum WorkerMsg {
                Started {
                    idx: usize,
                },
                Duplicate {
                    idx: usize,
                },
                /// Truncated file deleted because skip-truncated mode is active.
                Skipped {
                    idx: usize,
                },
                /// Corrupt file deleted because skip-corrupt mode is active.
                SkippedCorrupt {
                    idx: usize,
                },
                Completed {
                    idx: usize,
                    hit: Box<CarveHit>,
                    path: String,
                    result: Result<u64, String>,
                    quality: CarveQuality,
                },
            }
            let (done_tx, done_rx) = std::sync::mpsc::channel::<WorkerMsg>();

            for worker_device in worker_devices {
                let queue = Arc::clone(&queue);
                let device = worker_device;
                let done_tx = done_tx.clone();
                let cancel = Arc::clone(&cancel);
                let pause = Arc::clone(&pause);
                let meta_index = meta_index.clone();
                let seen_fingerprints = Arc::clone(&seen_fingerprints);

                std::thread::spawn(move || loop {
                    while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let item = queue.lock().unwrap().pop_front();
                    let (idx, hit, path) = match item {
                        None => break,
                        Some(i) => i,
                    };

                    // Duplicate check: fingerprint first 4 KiB at hit offset.
                    if let Some(fp) = hit_fingerprint(device.as_ref(), hit.byte_offset) {
                        let mut set = seen_fingerprints.lock().unwrap();
                        if !set.insert(fp) {
                            let _ = done_tx.send(WorkerMsg::Duplicate { idx });
                            continue;
                        }
                    }

                    let _ = done_tx.send(WorkerMsg::Started { idx });
                    // Create metadata-derived subdirectories if needed.
                    if let Some(parent) = std::path::Path::new(&path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let config = CarvingConfig {
                        signatures: vec![hit.signature.clone()],
                        scan_chunk_size: 4 * 1024 * 1024,
                        start_byte: 0,
                        end_byte: None,
                    };
                    let carver = Carver::new(Arc::clone(&device), config);
                    let result = std::fs::File::create(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|f| {
                            let mut writer = CancelWriter {
                                inner: f,
                                cancel: Arc::clone(&cancel),
                            };
                            carver.extract(&hit, &mut writer).map_err(|e| e.to_string())
                        });
                    if result.is_ok() {
                        if let Some(ref meta_idx) = meta_index {
                            apply_timestamps(&path, hit.byte_offset, meta_idx);
                        }
                    }
                    // Zero-byte extraction: size-hint resolved below min_size.
                    if matches!(result, Ok(0)) {
                        let _ = std::fs::remove_file(&path);
                        let _ = done_tx.send(WorkerMsg::Skipped { idx });
                        return;
                    }
                    // Post-extraction min_size check against actual bytes written.
                    if let Ok(bytes) = &result {
                        if hit.signature.min_size > 0 && *bytes < hit.signature.min_size {
                            let _ = std::fs::remove_file(&path);
                            let _ = done_tx.send(WorkerMsg::Skipped { idx });
                            return;
                        }
                    }
                    // Post-extraction quality check: run structural validator.
                    // PNG uses a seek-based chunk walk (no dead zone); all other
                    // formats use the head + tail buffer approach.
                    let quality = match &result {
                        Ok(bytes) => {
                            let truncated = *bytes >= hit.signature.max_size;
                            if !truncated
                                && matches!(
                                    hit.signature.extension.as_str(),
                                    "png"
                                        | "pdf"
                                        | "db"
                                        | "evtx"
                                        | "wav"
                                        | "avi"
                                        | "webp"
                                        | "aiff"
                                        | "exe"
                                        | "flac"
                                        | "elf"
                                        | "regf"
                                        | "tif"
                                        | "nef"
                                        | "arw"
                                        | "cr2"
                                        | "rw2"
                                        | "orf"
                                        | "pef"
                                        | "sr2"
                                        | "dcr"
                                        | "mp4"
                                        | "mov"
                                        | "m4v"
                                        | "3gp"
                                        | "m4a"
                                        | "heic"
                                        | "cr3"
                                        | "mkv"
                                        | "webm"
                                )
                            {
                                match hit.signature.extension.as_str() {
                                    "png" => post_validate::validate_png_file(Path::new(&path)),
                                    "pdf" => post_validate::validate_pdf_file(Path::new(&path)),
                                    "db" => post_validate::validate_sqlite_file(Path::new(&path)),
                                    "evtx" => post_validate::validate_evtx_file(Path::new(&path)),
                                    "wav" | "avi" | "webp" | "aiff" => {
                                        post_validate::validate_riff_file(Path::new(&path))
                                    }
                                    "exe" => post_validate::validate_exe_file(Path::new(&path)),
                                    "flac" => post_validate::validate_flac_file(Path::new(&path)),
                                    "elf" => post_validate::validate_elf_file(Path::new(&path)),
                                    "regf" => post_validate::validate_regf_file(Path::new(&path)),
                                    "mp4" | "mov" | "m4v" | "3gp" | "m4a" | "heic" | "cr3" => {
                                        post_validate::validate_isobmff_file(Path::new(&path))
                                    }
                                    "mkv" | "webm" => {
                                        post_validate::validate_ebml_file(Path::new(&path))
                                    }
                                    _ => post_validate::validate_tiff_file(Path::new(&path)),
                                }
                            } else {
                                let head = read_file_head(&path, 8192);
                                let tail = read_file_tail(&path, 65536);
                                post_validate::validate_extracted(
                                    &hit.signature.extension,
                                    &head,
                                    &tail,
                                    truncated,
                                    *bytes,
                                )
                            }
                        }
                        Err(_) => CarveQuality::Unknown,
                    };
                    // Skip-truncated mode: delete the file and report as Skipped.
                    if skip_truncated && matches!(quality, CarveQuality::Truncated) {
                        let _ = std::fs::remove_file(&path);
                        let _ = done_tx.send(WorkerMsg::Skipped { idx });
                    } else if skip_corrupt && matches!(quality, CarveQuality::Corrupt) {
                        let _ = std::fs::remove_file(&path);
                        let _ = done_tx.send(WorkerMsg::SkippedCorrupt { idx });
                    } else {
                        // Attempt to recover a meaningful filename from the
                        // carved file content for LNK and Prefetch files.
                        let path = if result.is_ok() {
                            match hit.signature.extension.as_str() {
                                "lnk" | "pf" => {
                                    try_recovered_rename(&path, &hit.signature.extension)
                                }
                                _ => path,
                            }
                        } else {
                            path
                        };
                        if result.is_ok() {
                            write_sha256_sidecar(&path);
                        }
                        let _ = done_tx.send(WorkerMsg::Completed {
                            idx,
                            hit: Box::new(hit),
                            path,
                            result,
                            quality,
                        });
                    }
                });
            }
            drop(done_tx); // let done_rx drain once all workers finish

            let mut completed = 0usize;
            let mut succeeded = 0usize;
            let mut truncated_count = 0usize;
            let mut failed = 0usize;
            let mut duplicates = 0usize;
            let mut skipped_trunc = 0usize;
            let mut skipped_corrupt = 0usize;
            let mut total_bytes = 0u64;
            let mut last_name = String::new();

            for msg in done_rx {
                match msg {
                    WorkerMsg::Started { idx } => {
                        let _ = tx.send(CarveMsg::ExtractionStarted { idx });
                    }
                    WorkerMsg::Duplicate { idx } => {
                        duplicates += 1;
                        completed += 1;
                        let _ = tx.send(CarveMsg::Duplicate { idx });
                        let _ = tx.send(CarveMsg::ExtractionProgress {
                            done: completed,
                            total,
                            total_bytes,
                            last_name: last_name.clone(),
                        });
                    }
                    WorkerMsg::Skipped { idx } => {
                        skipped_trunc += 1;
                        completed += 1;
                        let _ = tx.send(CarveMsg::Skipped { idx });
                        let _ = tx.send(CarveMsg::ExtractionProgress {
                            done: completed,
                            total,
                            total_bytes,
                            last_name: last_name.clone(),
                        });
                    }
                    WorkerMsg::SkippedCorrupt { idx } => {
                        skipped_corrupt += 1;
                        completed += 1;
                        let _ = tx.send(CarveMsg::SkippedCorrupt { idx });
                        let _ = tx.send(CarveMsg::ExtractionProgress {
                            done: completed,
                            total,
                            total_bytes,
                            last_name: last_name.clone(),
                        });
                    }
                    WorkerMsg::Completed {
                        idx,
                        hit,
                        path,
                        result,
                        quality,
                    } => {
                        completed += 1;
                        match result {
                            Ok(bytes) => {
                                let truncated = bytes >= hit.signature.max_size;
                                total_bytes += bytes;
                                if truncated {
                                    truncated_count += 1;
                                } else {
                                    succeeded += 1;
                                }
                                last_name = std::path::Path::new(&path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(&path)
                                    .to_string();
                                tracing::info!(path = %path, bytes, "extracted file");
                                let _ = tx.send(CarveMsg::Extracted {
                                    idx,
                                    bytes,
                                    truncated,
                                    quality,
                                });
                            }
                            Err(e) => {
                                if e.contains("cancelled") {
                                    tracing::debug!(path = %path, "extraction cancelled");
                                } else {
                                    failed += 1;
                                    tracing::warn!(path = %path, error = %e, "extraction failed");
                                }
                            }
                        }
                        let _ = tx.send(CarveMsg::ExtractionProgress {
                            done: completed,
                            total,
                            total_bytes,
                            last_name: last_name.clone(),
                        });
                    }
                }
            }
            let _ = tx.send(CarveMsg::ExtractionDone {
                succeeded,
                truncated: truncated_count,
                failed,
                duplicates,
                skipped_trunc,
                skipped_corrupt,
                total_bytes,
                elapsed_secs: extract_start.elapsed().as_secs_f64(),
            });
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid LNK blob with a single local-file LinkInfo.
    ///
    /// Layout (all offsets from file start):
    ///   0x00..0x04  HeaderSize = 0x4C
    ///   0x04..0x14  CLSID (zeroed — not validated by parser)
    ///   0x14..0x18  LinkFlags = HasLinkInfo (bit 1 only)
    ///   0x18..0x4C  remainder of header (zeroed)
    ///   0x4C..      LinkInfo block
    ///     +0  LinkInfoSize  = 28 + path_len + 2  (header + base + empty suffix)
    ///     +4  LinkInfoHeaderSize = 28
    ///     +8  LinkInfoFlags = VolumeIDAndLocalBasePath (1)
    ///     +12 VolumeIDOffset = 28  (overlaps with LocalBasePath for simplicity)
    ///     +16 LocalBasePathOffset = 28
    ///     +20 CommonNetworkRelativeLinkOffset = 0
    ///     +24 CommonPathSuffixOffset = 28 + path_len (points to null byte)
    ///     +28 LocalBasePath (null-terminated ANSI)
    ///     +28+path_len  CommonPathSuffix = 0x00
    fn build_lnk(local_base_path: &str) -> Vec<u8> {
        let path_bytes: Vec<u8> = local_base_path
            .bytes()
            .chain(std::iter::once(0u8))
            .collect();
        let li_size = 28u32 + path_bytes.len() as u32 + 1; // +1 for empty suffix null
        let suffix_off = 28u32 + path_bytes.len() as u32;

        let mut data = vec![0u8; 76 + li_size as usize + 1];
        // HeaderSize = 0x4C
        data[0..4].copy_from_slice(&[0x4C, 0x00, 0x00, 0x00]);
        // LinkFlags: HasLinkInfo only
        data[0x14..0x18].copy_from_slice(&[0x02, 0x00, 0x00, 0x00]);
        // LinkInfo at offset 76
        let li = 76usize;
        data[li..li + 4].copy_from_slice(&li_size.to_le_bytes());
        data[li + 4..li + 8].copy_from_slice(&28u32.to_le_bytes()); // header size
        data[li + 8..li + 12].copy_from_slice(&1u32.to_le_bytes()); // flags
        data[li + 12..li + 16].copy_from_slice(&28u32.to_le_bytes()); // VolumeIDOffset
        data[li + 16..li + 20].copy_from_slice(&28u32.to_le_bytes()); // LocalBasePathOffset
        data[li + 24..li + 28].copy_from_slice(&suffix_off.to_le_bytes()); // suffix offset
                                                                           // LocalBasePath string
        data[li + 28..li + 28 + path_bytes.len()].copy_from_slice(&path_bytes);
        // CommonPathSuffix: empty (null byte already zero-initialised)
        data
    }

    /// Build a minimal valid PF blob with the given exe name.
    fn build_pf(exe_name: &str) -> Vec<u8> {
        let mut data = vec![0u8; 0x4A];
        // Version 0x17 (Vista/Win7)
        data[0..4].copy_from_slice(&[0x17, 0x00, 0x00, 0x00]);
        // Signature "SCCA"
        data[4..8].copy_from_slice(b"SCCA");
        // ExecutableName: UTF-16LE at 0x10, max 29 chars + null
        for (i, c) in exe_name.chars().take(29).enumerate() {
            let w = c as u16;
            data[0x10 + i * 2] = (w & 0xFF) as u8;
            data[0x10 + i * 2 + 1] = (w >> 8) as u8;
        }
        data
    }

    #[test]
    fn lnk_name_extracted_from_link_info() {
        let data = build_lnk(r"C:\Windows\System32\notepad.exe");
        assert_eq!(
            parse_lnk_target_name(&data),
            Some("notepad.exe".to_string())
        );
    }

    #[test]
    fn pf_name_extracted_from_header() {
        let data = build_pf("NOTEPAD.EXE");
        assert_eq!(parse_pf_exe_name(&data), Some("notepad.exe".to_string()));
    }

    #[test]
    fn fallback_kept_when_parse_fails() {
        // Garbage data — neither LNK nor PF magic present.
        let data = vec![0xAAu8; 64];
        assert!(parse_lnk_target_name(&data).is_none());
        assert!(parse_pf_exe_name(&data).is_none());
    }
}
