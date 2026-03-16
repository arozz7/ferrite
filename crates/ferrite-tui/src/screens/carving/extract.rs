//! Extraction helpers for [`CarvingState`] — split from `mod.rs` to keep
//! file sizes under the project hard limit (600 lines).

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ferrite_carver::{CarveHit, Carver, CarvingConfig};
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

// ── Extraction impl ────────────────────────────────────────────────────────────

impl CarvingState {
    /// Build an output path for `hit` inside `dir`.
    ///
    /// If the metadata index contains the original filename for this offset,
    /// uses it.  Otherwise falls back to `ferrite_<ext>_<offset>.<ext>`.
    pub(super) fn filename_for_hit(&self, hit: &CarveHit, dir: &str) -> String {
        if let Some(idx) = &self.meta_index {
            if let Some(meta) = idx.lookup(hit.byte_offset) {
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
        let config = CarvingConfig {
            signatures: vec![hit.signature.clone()],
            scan_chunk_size: 4 * 1024 * 1024,
            start_byte: 0,
            end_byte: None,
        };
        std::thread::spawn(move || {
            // Ensure output directory exists before writing.
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(dir = %dir, error = %e, "failed to create output directory");
                return;
            }
            let carver = Carver::new(device, config);
            if let Ok(mut f) = std::fs::File::create(&filename) {
                match carver.extract(&hit, &mut f) {
                    Ok(bytes) => {
                        // Truncated when we hit the cap regardless of whether the
                        // format uses a footer.  For footer-less formats with a
                        // size hint (e.g. MP4), hitting max_size means the box
                        // walker failed and we fell back to the hard cap.
                        let truncated = bytes >= hit.signature.max_size;
                        if let Some(ref idx) = meta_index {
                            apply_timestamps(&filename, hit.byte_offset, idx);
                        }
                        tracing::info!(path = %filename, bytes, "extracted file");
                        let _ = tx.send(CarveMsg::Extracted {
                            idx,
                            bytes,
                            truncated,
                        });
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
        self.extract_summary = None;
        let cancel = Arc::clone(&self.extract_cancel);
        let pause = Arc::clone(&self.extract_pause);
        let meta_index = self.meta_index.clone();

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
                    total_bytes: 0,
                    elapsed_secs: extract_start.elapsed().as_secs_f64(),
                });
                return;
            }

            enum WorkerMsg {
                Started {
                    idx: usize,
                },
                Completed {
                    idx: usize,
                    hit: Box<CarveHit>,
                    path: String,
                    result: Result<u64, String>,
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
                    let _ = done_tx.send(WorkerMsg::Started { idx });
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
                    let _ = done_tx.send(WorkerMsg::Completed {
                        idx,
                        hit: Box::new(hit),
                        path,
                        result,
                    });
                });
            }
            drop(done_tx); // let done_rx drain once all workers finish

            let mut completed = 0usize;
            let mut succeeded = 0usize;
            let mut truncated_count = 0usize;
            let mut failed = 0usize;
            let mut total_bytes = 0u64;
            let mut last_name = String::new();

            for msg in done_rx {
                match msg {
                    WorkerMsg::Started { idx } => {
                        let _ = tx.send(CarveMsg::ExtractionStarted { idx });
                    }
                    WorkerMsg::Completed {
                        idx,
                        hit,
                        path,
                        result,
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
                total_bytes,
                elapsed_secs: extract_start.elapsed().as_secs_f64(),
            });
        });
    }
}
