//! Extraction helpers for [`CarvingState`] — split from `mod.rs` to keep
//! file sizes under the project hard limit (600 lines).

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ferrite_blockdev::AlignedBuffer;
use ferrite_carver::{post_validate, CarveHit, CarveQuality, Carver, CarvingConfig};
use ferrite_filesystem::MetadataIndex;

use super::{
    CarveMsg, CarveStatus, CarvingState, ExtractProgress, HitStatus, AUTO_EXTRACT_LOW_WATER,
};

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

    /// Drain the auto-extract queue and start a new extraction batch if none
    /// is currently running.  Also lifts the back-pressure scan pause once the
    /// queue drains below `AUTO_EXTRACT_LOW_WATER`.
    pub(super) fn pump_auto_extract(&mut self) {
        if self.extract_progress.is_some() {
            return; // a batch is already in flight
        }

        // Low-water resume: if the queue has drained enough, let the scan continue.
        if self.backpressure_paused && self.auto_extract_queue.len() < AUTO_EXTRACT_LOW_WATER {
            self.backpressure_paused = false;
            // Only clear the pause flag if the user hasn't manually paused too.
            if self.status == CarveStatus::Running {
                self.pause.store(false, Ordering::Relaxed);
            }
        }

        if self.auto_extract_queue.is_empty() {
            return;
        }
        const BATCH: usize = 500;
        let n = BATCH.min(self.auto_extract_queue.len());
        let work: Vec<(usize, CarveHit, String)> = self.auto_extract_queue.drain(..n).collect();
        self.start_extraction_batch(work);
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

        // Single worker: on a spinning disk (HDD/USB) multiple concurrent
        // workers issuing I/O at different offsets force constant head seeks,
        // making throughput worse than serial extraction.  A single worker
        // drains the offset-sorted queue in forward address order, giving the
        // best possible sequential-read behaviour.  This is also gentler on
        // potentially damaged recovery targets.
        let concurrency = 1;

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

            for _ in 0..concurrency {
                let queue = Arc::clone(&queue);
                let device = Arc::clone(&device);
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
