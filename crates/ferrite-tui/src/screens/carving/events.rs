//! Background-channel drain (`tick`) for [`CarvingState`].

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;

use ferrite_filesystem::build_metadata_index;

use super::{
    checkpoint, CarveMsg, CarveStatus, CarvingState, ExtractionSummary, HitEntry, HitStatus,
    AUTO_EXTRACT_HIGH_WATER, DISPLAY_CAP,
};

use std::sync::atomic::Ordering as AtomicOrdering;

impl CarvingState {
    /// Drain the background carving channel and the preview channel.
    pub fn tick(&mut self) {
        // Poll disk space every ~5 seconds (at ~10 fps that is ~50 ticks).
        self.disk_space_tick = self.disk_space_tick.wrapping_add(1);
        if self.disk_space_tick.is_multiple_of(50) || self.disk_avail_bytes.is_none() {
            self.poll_disk_space();
        }

        // When the user has requested a pause, promote to Paused the moment
        // the scan thread sets paused_ack (it does so just before entering
        // its spin-wait, so the drive is genuinely no longer advancing).
        if self.status == CarveStatus::Pausing && self.paused_ack.load(AtomicOrdering::Relaxed) {
            self.status = CarveStatus::Paused;
            self.paused_since = Some(std::time::Instant::now());
        }

        // Drain the main carving channel.  Each iteration borrows `self.rx`
        // only for the duration of `try_recv()`, so other fields can be freely
        // mutated inside the match arms (NLL field disjointness).
        'carve: loop {
            let msg = {
                let rx = match &self.rx {
                    Some(r) => r,
                    None => break 'carve,
                };
                rx.try_recv()
            };
            match msg {
                Ok(CarveMsg::Progress(p)) => {
                    self.scan_progress = Some(p);
                }
                Ok(CarveMsg::HitBatch(batch)) => {
                    let batch_len = batch.len();
                    self.total_hits_found += batch_len;

                    // Queue for auto-extract before moving into self.hits.
                    if self.auto_extract {
                        let dir = if self.output_dir.is_empty() {
                            "carved".to_string()
                        } else {
                            self.output_dir.clone()
                        };
                        for (i, hit) in batch.iter().enumerate() {
                            let display_idx = self.hits.len() + i;
                            let idx = if display_idx < DISPLAY_CAP {
                                display_idx
                            } else {
                                usize::MAX
                            };
                            let path = self.filename_for_hit(hit, &dir);
                            self.auto_extract_queue.push_back((idx, hit.clone(), path));
                        }
                    }

                    // Add to display list up to the cap.
                    for hit in batch {
                        if self.hits.len() < DISPLAY_CAP {
                            self.hits.push(HitEntry {
                                hit,
                                status: HitStatus::Unextracted,
                                selected: false,
                                quality: None,
                            });
                        }
                    }

                    // Checkpoint flush: every 1000 new displayable hits.
                    if self.hits.len().saturating_sub(self.checkpoint_flushed) >= 1000 {
                        if let Some(cp) = self.checkpoint_path.clone() {
                            let new_hits = &self.hits[self.checkpoint_flushed..];
                            for entry in new_hits {
                                let _ = checkpoint::append(&cp, &entry.hit, &entry.status);
                            }
                            self.checkpoint_flushed = self.hits.len();
                        }
                    }

                    // Pump auto-extract pipeline if a batch is not already running.
                    if self.auto_extract {
                        self.pump_auto_extract();
                    }

                    // Back-pressure: pause the scan whenever an extraction batch
                    // is in flight, OR the queue has grown past the high-water
                    // mark.  The primary trigger is the in-flight batch: at high
                    // hit densities the scan enqueues thousands of items per
                    // second and a simple size threshold is too slow to react.
                    if self.auto_extract
                        && !self.backpressure_paused
                        && self.status == CarveStatus::Running
                        && (self.extract_progress.is_some()
                            || self.auto_extract_queue.len() > AUTO_EXTRACT_HIGH_WATER)
                    {
                        self.pause.store(true, Ordering::Relaxed);
                        self.backpressure_paused = true;
                    }
                }
                Ok(CarveMsg::Done) => {
                    self.hit_sel = 0;
                    // Flush all remaining displayable hits to checkpoint.
                    if let Some(cp) = self.checkpoint_path.clone() {
                        let new_hits = &self.hits[self.checkpoint_flushed..];
                        for entry in new_hits {
                            let _ = checkpoint::append(&cp, &entry.hit, &entry.status);
                        }
                        self.checkpoint_flushed = self.hits.len();
                    }
                    // Spawn background thread to build filename index from filesystem metadata.
                    if let (Some(device), Some(meta_tx)) = (self.device.as_ref(), self.tx.as_ref())
                    {
                        self.meta_index_building = true;
                        let device = Arc::clone(device);
                        let meta_tx = meta_tx.clone();
                        std::thread::spawn(move || {
                            let index = build_metadata_index(device);
                            let _ = meta_tx.send(CarveMsg::MetadataReady(index));
                        });
                    }
                    self.status = CarveStatus::Done;
                    // Keep rx alive so extraction results can still arrive.
                }
                Ok(CarveMsg::MetadataReady(index)) => {
                    self.meta_index = Some(Arc::new(index));
                    self.meta_index_building = false;
                }
                Ok(CarveMsg::Extracted {
                    idx,
                    bytes,
                    truncated,
                    quality,
                }) => {
                    if let Some(entry) = self.hits.get_mut(idx) {
                        entry.status = if truncated {
                            HitStatus::Truncated { bytes }
                        } else {
                            HitStatus::Ok { bytes }
                        };
                        entry.quality = Some(quality);
                    }
                }
                Ok(CarveMsg::Duplicate { idx }) => {
                    if let Some(entry) = self.hits.get_mut(idx) {
                        entry.status = HitStatus::Duplicate;
                    }
                    self.duplicates_suppressed += 1;
                }
                Ok(CarveMsg::ExtractionStarted { idx }) => {
                    if let Some(entry) = self.hits.get_mut(idx) {
                        entry.status = HitStatus::Extracting;
                    }
                }
                Ok(CarveMsg::ExtractionProgress {
                    done,
                    total,
                    total_bytes,
                    last_name,
                }) => {
                    if let Some(p) = &mut self.extract_progress {
                        p.done = done;
                        p.total = total;
                        p.total_bytes = total_bytes;
                        p.last_name = last_name;
                    }
                }
                Ok(CarveMsg::ExtractionDone {
                    succeeded,
                    truncated,
                    failed,
                    duplicates,
                    total_bytes,
                    elapsed_secs,
                }) => {
                    self.extract_progress = None;
                    self.extract_cancel.store(false, Ordering::Relaxed);
                    self.extract_pause.store(false, Ordering::Relaxed);
                    // Lift back-pressure so the scan can resume between batches.
                    // Only clear the scan pause when no manual pause is active.
                    if self.backpressure_paused && self.status == CarveStatus::Running {
                        self.backpressure_paused = false;
                        self.pause.store(false, Ordering::Relaxed);
                    }
                    // Only show summary when at least one file was attempted.
                    if succeeded + truncated + failed + duplicates > 0 {
                        if self.auto_extract {
                            // Accumulate into the running session total so rapid
                            // per-file completions don't cause visible flicker.
                            if let Some(existing) = &mut self.extract_summary {
                                existing.succeeded += succeeded;
                                existing.truncated += truncated;
                                existing.failed += failed;
                                existing.duplicates += duplicates;
                                existing.total_bytes += total_bytes;
                                existing.elapsed_secs += elapsed_secs;
                            } else {
                                self.extract_summary = Some(ExtractionSummary {
                                    succeeded,
                                    truncated,
                                    failed,
                                    duplicates,
                                    total_bytes,
                                    elapsed_secs,
                                });
                            }
                        } else {
                            self.extract_summary = Some(ExtractionSummary {
                                succeeded,
                                truncated,
                                failed,
                                duplicates,
                                total_bytes,
                                elapsed_secs,
                            });
                        }
                    }
                    // Continue draining the auto-extract queue if enabled.
                    if self.auto_extract {
                        self.pump_auto_extract();
                    }
                }
                Ok(CarveMsg::Error(e)) => {
                    self.status = CarveStatus::Error(e);
                    self.rx = None;
                    self.tx = None;
                    break 'carve;
                }
                Err(mpsc::TryRecvError::Empty) => break 'carve,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    break 'carve;
                }
            }
        }

        // Drain the background preview channel (one result per tick is enough).
        let preview_result = self.preview_rx.as_ref().map(|rx| rx.try_recv());
        match preview_result {
            Some(Ok(result)) => {
                self.current_preview = result;
                self.preview_loading = false;
                self.preview_rx = None;
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.preview_loading = false;
                self.preview_rx = None;
            }
            Some(Err(mpsc::TryRecvError::Empty)) | None => {}
        }
    }

    /// Check available disk space at the output directory and cache it.
    fn poll_disk_space(&mut self) {
        let dir = if self.output_dir.is_empty() {
            ".".to_string()
        } else {
            self.output_dir.clone()
        };
        // Walk up to a directory that exists (output_dir may not exist yet).
        let check = {
            let p = std::path::Path::new(&dir);
            if p.exists() {
                dir.clone()
            } else {
                p.parent()
                    .and_then(|pp| pp.to_str())
                    .unwrap_or(".")
                    .to_string()
            }
        };
        self.disk_avail_bytes = fs2::available_space(&check).ok();
    }
}
