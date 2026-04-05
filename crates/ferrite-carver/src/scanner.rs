//! Parallel file-carving scanner and extractor.
//!
//! [`Carver`] reads the device in overlapping chunks, uses [`memchr`] for fast
//! first-byte matching, and [`rayon`] to search all signatures concurrently
//! within each chunk.  Streaming I/O and footer detection live in
//! [`crate::carver_io`]; signature-search helpers live in [`crate::scan_search`].

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::{trace, warn};

use ferrite_blockdev::BlockDevice;

use crate::carver_io::{
    read_bytes_clamped, stream_bytes, stream_until_footer, stream_until_last_footer,
};
use crate::error::Result;
use crate::scan_search::find_all;
use crate::signature::{CarvingConfig, Signature};
use crate::size_hint::read_size_hint;

// â”€â”€ Public types â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// A single file-carving hit returned by [`Carver::scan`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CarveHit {
    /// Absolute byte offset of the file header on the device.
    pub byte_offset: u64,
    /// The signature that matched.
    pub signature: Signature,
}

/// Periodic progress update emitted by [`Carver::scan_with_progress`].
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// Bytes scanned so far.
    pub bytes_scanned: u64,
    /// Total device size in bytes.
    pub device_size: u64,
    /// Number of hits found so far (before deduplication).
    pub hits_found: usize,
    /// Scan window start byte (from `CarvingConfig::start_byte`).
    pub scan_start: u64,
    /// Scan window end byte (from `CarvingConfig::end_byte`, or `device_size`).
    pub scan_end: u64,
}

/// Internal context passed through `scan_impl` for progress/cancel/pause/bytes signalling.
type ScanCtx<'a> = Option<(
    &'a std::sync::mpsc::SyncSender<ScanProgress>,
    &'a Arc<std::sync::atomic::AtomicBool>, // cancel
    &'a Arc<std::sync::atomic::AtomicBool>, // pause
    &'a Arc<std::sync::atomic::AtomicBool>, // paused_ack
    &'a Arc<AtomicU64>,                     // bytes_read counter
)>;

/// Signature-based file carving engine.
///
/// Constructed with a block device and a [`CarvingConfig`]; the device is
/// read-only and shared across threads via `Arc`.
pub struct Carver {
    device: Arc<dyn BlockDevice>,
    config: CarvingConfig,
}

impl Carver {
    pub fn new(device: Arc<dyn BlockDevice>, config: CarvingConfig) -> Self {
        Self { device, config }
    }

    /// Returns a reference to the config.
    pub fn config(&self) -> &CarvingConfig {
        &self.config
    }

    // â”€â”€ Scanning â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Scan the entire device and return all detected file-carving hits,
    /// sorted by byte offset.
    pub fn scan(&self) -> Result<Vec<CarveHit>> {
        let mut all_hits = Vec::new();
        let mut collect = |batch: Vec<CarveHit>| all_hits.extend(batch);
        self.scan_impl(None, &mut collect)?;
        all_hits.sort_by_key(|h| h.byte_offset);
        Ok(all_hits)
    }

    /// Same as [`scan`] but sends a [`ScanProgress`] update after each chunk
    /// and respects cancel/pause signals.
    ///
    /// - If `cancel` is set the scan stops between chunks and returns partial
    ///   hits accumulated so far (not an error).
    /// - If `pause` is set the scan spin-waits between chunks until cleared.
    ///
    /// Progress updates are best-effort (`try_send`) â€” a full channel does not
    /// stall the scan.
    pub fn scan_with_progress(
        &self,
        tx: &std::sync::mpsc::SyncSender<ScanProgress>,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        pause: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        paused_ack: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        bytes_read: &Arc<AtomicU64>,
    ) -> Result<Vec<CarveHit>> {
        let mut all_hits = Vec::new();
        let mut collect = |batch: Vec<CarveHit>| all_hits.extend(batch);
        self.scan_impl(
            Some((tx, cancel, pause, paused_ack, bytes_read)),
            &mut collect,
        )?;
        all_hits.sort_by_key(|h| h.byte_offset);
        Ok(all_hits)
    }

    /// Same as [`scan_with_progress`] but streams each chunk's hits to
    /// `on_hits` immediately instead of accumulating in memory.
    ///
    /// Hits within each chunk are sorted by byte offset before being delivered.
    /// Returns `Ok(())` on completion, including early cancellation (partial
    /// results have already been delivered via `on_hits`).
    ///
    /// `bytes_read` is a monotonic counter incremented by the number of bytes
    /// read each chunk; share the same `Arc` with a
    /// [`ferrite_core::ThermalGuard`] for speed-based thermal inference.
    pub fn scan_streaming(
        &self,
        tx: &std::sync::mpsc::SyncSender<ScanProgress>,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        pause: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        paused_ack: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        bytes_read: &Arc<AtomicU64>,
        on_hits: &mut impl FnMut(Vec<CarveHit>),
    ) -> Result<()> {
        self.scan_impl(Some((tx, cancel, pause, paused_ack, bytes_read)), on_hits)
    }

    fn scan_impl(
        &self,
        progress: ScanCtx<'_>,
        on_hits: &mut impl FnMut(Vec<CarveHit>),
    ) -> Result<()> {
        let device_size = self.device.size();
        if device_size == 0 {
            tracing::warn!("carving scan aborted: device reports size 0 — check device selection or image file");
            return Ok(());
        }
        if self.config.signatures.is_empty() {
            return Ok(());
        }

        let scan_start = self.config.start_byte.min(device_size);
        let scan_end = self.config.end_byte.unwrap_or(device_size).min(device_size);
        if scan_start >= scan_end {
            return Ok(());
        }

        let chunk_size = self.config.scan_chunk_size;
        // Overlap = longest header - 1, so a header starting just before the
        // chunk boundary can still be fully matched.
        let overlap: usize = self
            .config
            .signatures
            .iter()
            .map(|s| s.header.len().saturating_sub(1))
            .max()
            .unwrap_or(0);

        let mut total_hits = 0usize;
        let mut offset = scan_start;
        // Track the last reported hit offset per suppression key.
        // Key = suppress_group when set; otherwise the signature name.
        // This allows related signatures (e.g. TS + M2TS) to share a gap counter
        // so a hit from one suppresses nearby hits from the other.
        let mut last_hit_by_sig: HashMap<String, u64> = HashMap::new();

        while offset < scan_end {
            let remaining = (scan_end - offset) as usize;
            let read_size = (chunk_size + overlap).min(remaining);

            // Only report hits whose header starts strictly before the
            // non-overlap boundary, preventing duplicates in the next chunk.
            let is_last = offset + chunk_size as u64 >= scan_end;
            let report_end = if is_last {
                read_size
            } else {
                chunk_size.min(read_size)
            };

            let data = match read_bytes_clamped(self.device.as_ref(), offset, read_size) {
                Ok(d) => d,
                Err(e) => {
                    warn!(offset, error = %e, "read error during scan â€” skipping chunk");
                    offset += chunk_size as u64;
                    continue;
                }
            };

            // Search all signatures in parallel within this chunk.
            let mut chunk_hits: Vec<CarveHit> = self
                .config
                .signatures
                .par_iter()
                .flat_map(|sig| find_all(sig, &data, offset, report_end))
                .collect();

            // Enforce min_size: skip hits where the device doesn't have enough
            // bytes left to satisfy the minimum extraction length.
            chunk_hits.retain(|h| {
                h.signature.min_size == 0
                    || h.byte_offset.saturating_add(h.signature.min_size) <= device_size
            });

            // Sort within chunk so callers receive hits in ascending offset order.
            chunk_hits.sort_by_key(|h| h.byte_offset);

            // Apply min_hit_gap: suppress hits that fall within the gap window
            // of the last reported hit for the same signature.  Must run after
            // sorting so offsets are processed in order.
            if self.config.signatures.iter().any(|s| s.min_hit_gap > 0) {
                chunk_hits.retain(|h| {
                    if h.signature.min_hit_gap == 0 {
                        return true;
                    }
                    let key = h
                        .signature
                        .suppress_group
                        .as_deref()
                        .unwrap_or(&h.signature.name);
                    let accept = match last_hit_by_sig.get(key) {
                        None => true, // first hit for this group — always accept
                        Some(&last) => {
                            h.byte_offset >= last.saturating_add(h.signature.min_hit_gap)
                        }
                    };
                    if accept {
                        last_hit_by_sig.insert(key.to_owned(), h.byte_offset);
                    }
                    accept
                });
            }

            total_hits += chunk_hits.len();

            trace!(
                offset,
                chunk_bytes = data.len(),
                hits = chunk_hits.len(),
                "scanned chunk"
            );

            on_hits(chunk_hits);
            offset += chunk_size as u64;

            if let Some((tx, cancel, pause, paused_ack, bytes_read)) = &progress {
                bytes_read.fetch_add(read_size as u64, Ordering::Relaxed);
                let _ = tx.try_send(ScanProgress {
                    bytes_scanned: offset.min(scan_end),
                    device_size,
                    hits_found: total_hits,
                    scan_start,
                    scan_end,
                });
                // Spin-wait while paused; set paused_ack so the TUI can
                // transition from “Pausing” to “Paused” once we are here.
                if pause.load(Ordering::Relaxed) {
                    paused_ack.store(true, Ordering::Relaxed);
                    while pause.load(Ordering::Relaxed) {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::yield_now();
                    }
                    paused_ack.store(false, Ordering::Relaxed);
                }
                // Honour cancel — partial hits already delivered via on_hits.
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    // â”€â”€ Extraction â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Returns `true` when the hit can be extracted without creating a file.
    ///
    /// For signatures where `SizeHint::skip_on_failure()` is `true` (e.g. ADTS),
    /// this runs the frame-walker size hint against the device to determine
    /// whether the hit is a genuine frame sequence.  When the walker returns
    /// `None` (false positive), the caller should skip `File::create` entirely --
    /// there is nothing to write and no file to clean up.
    ///
    /// For all other signatures this returns `true` unconditionally (the
    /// extraction should proceed, falling back to `max_size` if needed).
    pub fn is_viable_hit(&self, hit: &CarveHit) -> bool {
        let live_sig = self
            .config
            .signatures
            .iter()
            .find(|s| s.name == hit.signature.name)
            .unwrap_or(&hit.signature);

        if let Some(hint) = &live_sig.size_hint {
            if hint.skip_on_failure() {
                return read_size_hint(
                    self.device.as_ref(),
                    hit.byte_offset,
                    hint,
                    live_sig.max_size,
                )
                .is_some();
            }
        }
        true
    }

    /// Extract the file for `hit` into a heap `Vec<u8>` and return it.
    ///
    /// This is the in-memory variant of [`extract`].  Use it for small files
    /// (where `signature.max_size` is ≤ a few hundred KiB) when the caller
    /// needs to inspect the data before deciding whether to write it to disk.
    /// Cancellation is not supported — the caller should only use this for
    /// files small enough that the device read completes in milliseconds.
    ///
    /// Returns `Ok(vec)` where `vec` may be empty (size-hint resolved below
    /// `min_size`, or `skip_on_failure` returned `None`).
    pub fn extract_to_vec(&self, hit: &CarveHit) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        let written = self.extract(hit, &mut buf)?;
        if written == 0 {
            buf.clear();
        }
        Ok(buf)
    }

    /// Extract the file for `hit` and write it to `writer`.
    ///
    /// **No footer:** streams exactly `signature.max_size` bytes (or until the
    /// device ends).
    ///
    /// **With footer:** streams from `hit.byte_offset` until the first footer
    /// occurrence (inclusive), capped at `signature.max_size`.  If the footer
    /// is not found, all bytes up to the cap are written.
    ///
    /// Returns the total number of bytes written.
    pub fn extract(&self, hit: &CarveHit, writer: &mut dyn Write) -> Result<u64> {
        // Prefer the live config's signature over the frozen checkpoint copy.
        // This allows size_hint improvements added after a session was started
        // to be applied retroactively when a checkpoint is resumed.
        let live_sig_opt = if hit.signature.size_hint.is_none() {
            self.config
                .signatures
                .iter()
                .find(|s| s.name == hit.signature.name)
        } else {
            None
        };
        let sig = live_sig_opt.unwrap_or(&hit.signature);

        let device_size = self.device.size();

        if hit.byte_offset >= device_size {
            return Ok(0);
        }

        // If the signature carries a size hint, read the true file length from
        // the embedded field.  Fall back to max_size if the read fails or the
        // parsed value exceeds max_size (corrupt / stale data).
        //
        // Additionally, never extract more bytes than physically remain on the
        // device from hit.byte_offset onwards.  This prevents a bogus size-hint
        // value (false-positive hit whose header fields contain garbage) from
        // producing a file larger than the source device — which is impossible
        // for any real file.
        //
        // For frame-walking hints (Adts, OggStream, etc.) `None` means the data
        // does not match the expected structure — skip the file entirely rather
        // than falling back to max_size, which would produce a large false-positive.
        let remaining_on_device = device_size.saturating_sub(hit.byte_offset);
        let (extraction_size, hint_resolved) = if let Some(hint) = &sig.size_hint {
            match read_size_hint(self.device.as_ref(), hit.byte_offset, hint, sig.max_size) {
                Some(size) => (size.min(sig.max_size).min(remaining_on_device), true),
                None => {
                    if hint.skip_on_failure() {
                        trace!(
                            sig = %sig.name,
                            offset = hit.byte_offset,
                            "skipping extraction: frame-walker hint returned None (false positive)"
                        );
                        return Ok(0);
                    }
                    (sig.max_size.min(remaining_on_device), false)
                }
            }
        } else {
            (sig.max_size.min(remaining_on_device), false)
        };

        // If the resolved size is below the signature's minimum, skip extraction.
        // This catches false-positive hits where a size-hint walker (e.g. MPEG-TS
        // stride check) finds very few valid structures and returns a tiny size.
        if sig.min_size > 0 && extraction_size < sig.min_size {
            trace!(
                sig = %sig.name,
                extraction_size,
                min_size = sig.min_size,
                "skipping extraction: resolved size below min_size"
            );
            return Ok(0);
        }

        let max_end = (hit.byte_offset + extraction_size).min(device_size);

        // When the size hint resolved successfully, stream the exact number
        // of bytes — bypass the footer search entirely.  Structure-walking
        // hints (PNG, GIF, OGG, EBML, …) already found the true end-of-file;
        // searching for the footer would risk a false match inside compressed
        // data (e.g. `00 3B` in GIF LZW data, IEND bytes in PNG IDAT) that
        // truncates the file prematurely.
        if hint_resolved || sig.footer.is_empty() {
            stream_bytes(self.device.as_ref(), hit.byte_offset, max_end, writer)
        } else if sig.footer_last {
            stream_until_last_footer(
                self.device.as_ref(),
                hit.byte_offset,
                max_end,
                &sig.footer,
                sig.footer_extra,
                writer,
            )
        } else {
            stream_until_footer(
                self.device.as_ref(),
                hit.byte_offset,
                max_end,
                &sig.footer,
                sig.footer_extra,
                writer,
            )
        }
    }
}

// â”€â”€ Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[cfg(test)]
mod tests {
    // Placeholder â€” scan tests below.
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;
    use crate::signature::{CarvingConfig, Signature};

    // â”€â”€ Helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn sig(header: &[u8], footer: &[u8], max_size: u64) -> Signature {
        Signature {
            name: "Test".into(),
            extension: "bin".into(),
            header: header.iter().map(|&b| Some(b)).collect(),
            footer: footer.to_vec(),
            footer_last: false,
            max_size,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
            suppress_group: None,
            footer_extra: 0,
        }
    }

    fn config_with(sigs: Vec<Signature>, chunk_size: usize) -> CarvingConfig {
        CarvingConfig {
            signatures: sigs,
            scan_chunk_size: chunk_size,
            start_byte: 0,
            end_byte: None,
        }
    }

    fn device_from(data: Vec<u8>) -> Arc<dyn BlockDevice> {
        Arc::new(MockBlockDevice::new(data, 512))
    }

    // â”€â”€ scan tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn scan_finds_single_hit() {
        let mut data = vec![0u8; 1024];
        data[0..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);

        let dev = device_from(data);
        let cfg = config_with(vec![sig(&[0xFF, 0xD8, 0xFF], &[], 1_000_000)], 512);
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].byte_offset, 0);
    }

    #[test]
    fn scan_finds_hit_mid_device() {
        let mut data = vec![0u8; 2048];
        data[768..771].copy_from_slice(&[0xAA, 0xBB, 0xCC]);

        let dev = device_from(data);
        let cfg = config_with(vec![sig(&[0xAA, 0xBB, 0xCC], &[], 1_000)], 1024);
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].byte_offset, 768);
    }

    #[test]
    fn scan_finds_multiple_hits_same_sig() {
        let mut data = vec![0u8; 2048];
        data[0..2].copy_from_slice(&[0xBE, 0xEF]);
        data[1024..1026].copy_from_slice(&[0xBE, 0xEF]);

        let dev = device_from(data);
        let cfg = config_with(vec![sig(&[0xBE, 0xEF], &[], 512)], 1024);
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].byte_offset, 0);
        assert_eq!(hits[1].byte_offset, 1024);
    }

    #[test]
    fn scan_detects_header_at_chunk_boundary() {
        // Header = [0xDE, 0xAD, 0xBE, 0xEF], placed starting at byte 2.
        // chunk_size = 4, so bytes 0-3 are chunk 0, bytes 4-7 are chunk 1.
        // The header starts at pos=2 inside chunk 0 (< chunk_size=4) but its
        // last byte falls at pos=5 inside the overlap window.
        let mut data = vec![0u8; 512];
        data[2..6].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let dev = device_from(data);
        let header = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let cfg = config_with(vec![sig(&header, &[], 256)], 4);
        let hits = Carver::new(dev, cfg).scan().unwrap();

        // Must find exactly one hit at offset 2.
        assert_eq!(hits.len(), 1, "expected exactly one hit, got: {hits:?}");
        assert_eq!(hits[0].byte_offset, 2);
    }

    #[test]
    fn scan_no_double_count_in_overlap() {
        // Header straddles the boundary (starts inside this chunk's non-overlap
        // window and extends into the overlap). It must only be reported once.
        let mut data = vec![0u8; 512];
        let header = [0x11u8, 0x22, 0x33];
        // Place at offset 3 â€” with chunk_size=4 this is pos=3 < 4, reported in chunk 0.
        // In chunk 1 (offset=4), pos would be -1 (not in buffer), so no duplicate.
        data[3..6].copy_from_slice(&header);

        let dev = device_from(data);
        let cfg = config_with(vec![sig(&header, &[], 256)], 4);
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].byte_offset, 3);
    }

    #[test]
    fn scan_multiple_signatures_parallel() {
        let mut data = vec![0u8; 2048];
        // Sig A at offset 0
        data[0..2].copy_from_slice(&[0xAA, 0x01]);
        // Sig B at offset 512
        data[512..514].copy_from_slice(&[0xBB, 0x02]);

        let dev = device_from(data);
        let cfg = config_with(
            vec![sig(&[0xAA, 0x01], &[], 100), sig(&[0xBB, 0x02], &[], 100)],
            1024,
        );
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 2);
        // Sorted by offset
        assert_eq!(hits[0].byte_offset, 0);
        assert_eq!(hits[1].byte_offset, 512);
    }

    #[test]
    fn scan_empty_device_returns_empty() {
        // A device that contains no matching header bytes returns no hits.
        let cfg = config_with(vec![sig(&[0xFF], &[], 100)], 512);
        let dev = device_from(vec![0u8; 512]);
        let hits = Carver::new(dev, cfg).scan().unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn scan_wildcard_header_matches_riff_subtypes() {
        // Simulate two RIFF files: an AVI and a WAV.
        // AVI: RIFF????AVI  (41 56 49 20)
        // WAV: RIFF????WAVE (57 41 56 45)
        let mut data = vec![0u8; 2048];
        // AVI at offset 0: RIFF + size(u32le=100) + "AVI "
        data[0..4].copy_from_slice(b"RIFF");
        data[4..8].copy_from_slice(&100u32.to_le_bytes());
        data[8..12].copy_from_slice(b"AVI ");
        // WAV at offset 512: RIFF + size(u32le=200) + "WAVE"
        data[512..516].copy_from_slice(b"RIFF");
        data[516..520].copy_from_slice(&200u32.to_le_bytes());
        data[520..524].copy_from_slice(b"WAVE");

        let dev = device_from(data);

        // AVI signature with wildcard bytes at positions 4-7.
        let avi_sig = Signature {
            name: "AVI".into(),
            extension: "avi".into(),
            header: vec![
                Some(0x52),
                Some(0x49),
                Some(0x46),
                Some(0x46), // RIFF
                None,
                None,
                None,
                None, // size (wildcard)
                Some(0x41),
                Some(0x56),
                Some(0x49),
                Some(0x20), // AVI<space>
            ],
            footer: vec![],
            footer_last: false,
            max_size: 2_147_483_648,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
            suppress_group: None,
            footer_extra: 0,
        };
        let wav_sig = Signature {
            name: "WAV".into(),
            extension: "wav".into(),
            header: vec![
                Some(0x52),
                Some(0x49),
                Some(0x46),
                Some(0x46), // RIFF
                None,
                None,
                None,
                None, // size (wildcard)
                Some(0x57),
                Some(0x41),
                Some(0x56),
                Some(0x45), // WAVE
            ],
            footer: vec![],
            footer_last: false,
            max_size: 2_147_483_648,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
            suppress_group: None,
            footer_extra: 0,
        };
        let cfg = CarvingConfig {
            signatures: vec![avi_sig, wav_sig],
            scan_chunk_size: 1024,
            start_byte: 0,
            end_byte: None,
        };
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 2, "expected AVI + WAV hits, got: {hits:?}");
        assert_eq!(hits[0].byte_offset, 0);
        assert_eq!(hits[0].signature.extension, "avi");
        assert_eq!(hits[1].byte_offset, 512);
        assert_eq!(hits[1].signature.extension, "wav");
    }

    #[test]
    fn min_size_filters_hit_near_device_end() {
        // Device is 512 bytes. Header at offset 500. min_size = 100.
        // 500 + 100 = 600 > 512, so the hit should be filtered out.
        let data = {
            let mut d = vec![0u8; 512];
            d[500] = 0xAA;
            d
        };
        let dev = device_from(data);
        let sig = Signature {
            name: "Test".into(),
            extension: "tst".into(),
            header: vec![Some(0xAA)],
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 100,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
            suppress_group: None,
            footer_extra: 0,
        };
        let cfg = CarvingConfig {
            signatures: vec![sig],
            ..Default::default()
        };
        let hits = Carver::new(dev, cfg).scan().unwrap();
        assert!(
            hits.is_empty(),
            "hit near device end should be filtered by min_size"
        );
    }

    #[test]
    fn min_hit_gap_suppresses_nearby_hits() {
        // Three occurrences of the magic at offsets 0, 100, and 2000.
        // With min_hit_gap = 512, the hit at 100 is suppressed (100 < 0+512)
        // but the hit at 2000 is kept (2000 >= 0+512).
        let mut data = vec![0u8; 4096];
        data[0] = 0xAB;
        data[100] = 0xAB;
        data[2000] = 0xAB;

        let dev = device_from(data);
        let gapped_sig = Signature {
            name: "GapTest".into(),
            extension: "tst".into(),
            header: vec![Some(0xAB)],
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 512,
            suppress_group: None,
            footer_extra: 0,
        };
        let cfg = CarvingConfig {
            signatures: vec![gapped_sig],
            scan_chunk_size: 4096,
            start_byte: 0,
            end_byte: None,
        };
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 2, "expected hits at 0 and 2000, got: {hits:?}");
        assert_eq!(hits[0].byte_offset, 0);
        assert_eq!(hits[1].byte_offset, 2000);
    }

    #[test]
    fn min_hit_gap_zero_does_not_suppress() {
        // With min_hit_gap = 0, all three hits should be reported.
        let mut data = vec![0u8; 512];
        data[0] = 0xAB;
        data[10] = 0xAB;
        data[20] = 0xAB;

        let dev = device_from(data);
        let cfg = config_with(vec![sig(&[0xAB], &[], 100)], 512);
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 3, "min_hit_gap=0 should not suppress any hits");
    }

    #[test]
    fn min_hit_gap_tracks_across_chunks() {
        // Two chunks of 512 bytes each. Hit at offset 0 (chunk 0) and offset 600 (chunk 1).
        // With min_hit_gap = 1024, the second hit at 600 is suppressed (600 < 0+1024).
        let mut data = vec![0u8; 1024];
        data[0] = 0xCC;
        data[600] = 0xCC;

        let dev = device_from(data);
        let gapped_sig = Signature {
            name: "CrossChunk".into(),
            extension: "bin".into(),
            header: vec![Some(0xCC)],
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 1024,
            suppress_group: None,
            footer_extra: 0,
        };
        let cfg = CarvingConfig {
            signatures: vec![gapped_sig],
            scan_chunk_size: 512,
            start_byte: 0,
            end_byte: None,
        };
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 1, "cross-chunk gap should suppress second hit");
        assert_eq!(hits[0].byte_offset, 0);
    }

    #[test]
    fn suppress_group_cross_sig_dedup() {
        // Simulate M2TS/TS co-detection: SigA fires at offset 0 (magic = 0xDD),
        // SigB fires at offset 4 (magic = 0xEE), mimicking M2TS+TS where every
        // M2TS packet places the TS sync byte 4 bytes later.
        //
        // Both sigs share suppress_group = "transport" with min_hit_gap = 512.
        // After SigA hits at 0, the group tracker is at 0.
        // SigB's hit at 4 is within the 512-byte gap → suppressed.
        let mut data = vec![0u8; 512];
        data[0] = 0xDD; // SigA
        data[4] = 0xEE; // SigB (would be suppressed by the group)

        let dev = device_from(data);
        let sig_a = Signature {
            name: "SigA".into(),
            extension: "a".into(),
            header: vec![Some(0xDD)],
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 512,
            suppress_group: Some("transport".into()),
            footer_extra: 0,
        };
        let sig_b = Signature {
            name: "SigB".into(),
            extension: "b".into(),
            header: vec![Some(0xEE)],
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 512,
            suppress_group: Some("transport".into()),
            footer_extra: 0,
        };
        let cfg = CarvingConfig {
            signatures: vec![sig_a, sig_b],
            scan_chunk_size: 512,
            start_byte: 0,
            end_byte: None,
        };
        let hits = Carver::new(dev, cfg).scan().unwrap();
        assert_eq!(
            hits.len(),
            1,
            "SigB hit at offset 4 should be suppressed by SigA's group gap"
        );
        assert_eq!(hits[0].byte_offset, 0);
        assert_eq!(hits[0].signature.name, "SigA");
    }

    #[test]
    fn min_size_keeps_hit_with_sufficient_space() {
        // Device is 512 bytes. Header at offset 0. min_size = 100.
        // 0 + 100 = 100 <= 512, so the hit should be kept.
        let data = {
            let mut d = vec![0u8; 512];
            d[0] = 0xAA;
            d
        };
        let dev = device_from(data);
        let sig = Signature {
            name: "Test".into(),
            extension: "tst".into(),
            header: vec![Some(0xAA)],
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 100,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
            suppress_group: None,
            footer_extra: 0,
        };
        let cfg = CarvingConfig {
            signatures: vec![sig],
            ..Default::default()
        };
        let hits = Carver::new(dev, cfg).scan().unwrap();
        assert_eq!(
            hits.len(),
            1,
            "hit with sufficient remaining space should be kept"
        );
    }
}
