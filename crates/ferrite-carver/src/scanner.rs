//! Parallel file-carving scanner and extractor.
//!
//! [`Carver`] reads the device in overlapping chunks, uses [`memchr`] for fast
//! first-byte matching, and [`rayon`] to search all signatures concurrently
//! within each chunk.  Footer detection uses [`memchr::memmem`] over a
//! sliding window to handle footers that span chunk boundaries.

use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use memchr::memmem;
use rayon::prelude::*;
use tracing::{trace, warn};

use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::error::{CarveError, Result};
use crate::signature::{CarvingConfig, Signature, SizeHint};

// ── Public types ──────────────────────────────────────────────────────────────

/// A single file-carving hit returned by [`Carver::scan`].
#[derive(Debug, Clone)]
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
}

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

    // ── Scanning ──────────────────────────────────────────────────────────────

    /// Scan the entire device and return all detected file-carving hits,
    /// sorted by byte offset.
    pub fn scan(&self) -> Result<Vec<CarveHit>> {
        self.scan_inner(None)
    }

    /// Same as [`scan`] but sends a [`ScanProgress`] update after each chunk
    /// and respects cancel/pause signals.
    ///
    /// - If `cancel` is set the scan stops between chunks and returns whatever
    ///   hits have been found so far (partial results, not an error).
    /// - If `pause` is set the scan spin-waits between chunks until cleared.
    ///
    /// Progress updates are best-effort (`try_send`) — a full channel does not
    /// stall the scan.
    pub fn scan_with_progress(
        &self,
        tx: &std::sync::mpsc::SyncSender<ScanProgress>,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        pause: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Vec<CarveHit>> {
        self.scan_inner(Some((tx, cancel, pause)))
    }

    fn scan_inner(
        &self,
        progress: Option<(
            &std::sync::mpsc::SyncSender<ScanProgress>,
            &std::sync::Arc<std::sync::atomic::AtomicBool>,
            &std::sync::Arc<std::sync::atomic::AtomicBool>,
        )>,
    ) -> Result<Vec<CarveHit>> {
        let device_size = self.device.size();
        if device_size == 0 || self.config.signatures.is_empty() {
            return Ok(vec![]);
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

        let mut all_hits: Vec<CarveHit> = Vec::new();
        let mut offset = 0u64;

        while offset < device_size {
            let remaining = (device_size - offset) as usize;
            let read_size = (chunk_size + overlap).min(remaining);

            // Only report hits whose header starts strictly before the
            // non-overlap boundary, preventing duplicates in the next chunk.
            let is_last = offset + chunk_size as u64 >= device_size;
            let report_end = if is_last {
                read_size
            } else {
                chunk_size.min(read_size)
            };

            let data = match read_bytes_clamped(self.device.as_ref(), offset, read_size) {
                Ok(d) => d,
                Err(e) => {
                    warn!(offset, error = %e, "read error during scan — skipping chunk");
                    offset += chunk_size as u64;
                    continue;
                }
            };

            // Search all signatures in parallel within this chunk.
            let chunk_hits: Vec<CarveHit> = self
                .config
                .signatures
                .par_iter()
                .flat_map(|sig| find_all(sig, &data, offset, report_end))
                .collect();

            trace!(
                offset,
                chunk_bytes = data.len(),
                hits = chunk_hits.len(),
                "scanned chunk"
            );

            all_hits.extend(chunk_hits);
            offset += chunk_size as u64;

            if let Some((tx, cancel, pause)) = &progress {
                let _ = tx.try_send(ScanProgress {
                    bytes_scanned: offset.min(device_size),
                    device_size,
                    hits_found: all_hits.len(),
                });
                // Spin-wait while paused; yield to avoid busy-looping.
                while pause.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::yield_now();
                }
                // Honour cancel — return partial hits, not an error.
                if cancel.load(Ordering::Relaxed) {
                    all_hits.sort_by_key(|h| h.byte_offset);
                    return Ok(all_hits);
                }
            }
        }

        all_hits.sort_by_key(|h| h.byte_offset);
        Ok(all_hits)
    }

    // ── Extraction ────────────────────────────────────────────────────────────

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
        let sig = &hit.signature;
        let device_size = self.device.size();

        if hit.byte_offset >= device_size {
            return Ok(0);
        }

        // If the signature carries a size hint, read the true file length from
        // the embedded field.  Fall back to max_size if the read fails or the
        // parsed value exceeds max_size (corrupt / stale data).
        let extraction_size = if let Some(hint) = &sig.size_hint {
            read_size_hint(self.device.as_ref(), hit.byte_offset, hint)
                .unwrap_or(sig.max_size)
                .min(sig.max_size)
        } else {
            sig.max_size
        };

        let max_end = (hit.byte_offset + extraction_size).min(device_size);

        if sig.footer.is_empty() {
            stream_bytes(self.device.as_ref(), hit.byte_offset, max_end, writer)
        } else if sig.footer_last {
            stream_until_last_footer(
                self.device.as_ref(),
                hit.byte_offset,
                max_end,
                &sig.footer,
                writer,
            )
        } else {
            stream_until_footer(
                self.device.as_ref(),
                hit.byte_offset,
                max_end,
                &sig.footer,
                writer,
            )
        }
    }
}

// ── Chunk search ──────────────────────────────────────────────────────────────

/// Return all positions within `data[..report_end]` where `sig.header` begins.
///
/// Uses [`memchr`] on the first fixed (non-wildcard) byte for fast scanning,
/// then verifies the full pattern including `??` wildcard positions.
fn find_all(
    sig: &Signature,
    data: &[u8],
    chunk_abs_offset: u64,
    report_end: usize,
) -> Vec<CarveHit> {
    let header = &sig.header;
    if header.is_empty() || data.is_empty() {
        return vec![];
    }

    // Find the first fixed byte to use as the memchr anchor.
    let Some((anchor_idx, anchor_byte)) = header
        .iter()
        .enumerate()
        .find_map(|(i, b)| b.map(|byte| (i, byte)))
    else {
        return vec![]; // all-wildcard header — refuse to match everything
    };

    let report_end = report_end.min(data.len());
    let mut hits = Vec::new();
    // Search window starts at anchor_idx so we can back-compute the header start.
    let mut search_start = anchor_idx;

    loop {
        if search_start >= report_end + anchor_idx {
            break;
        }
        let scan_end = (report_end + anchor_idx).min(data.len());
        let window = &data[search_start..scan_end];
        let Some(rel) = memchr::memchr(anchor_byte, window) else {
            break;
        };
        // Position of the anchor byte in data[].
        let anchor_pos = search_start + rel;
        // Position where the header would start.
        let pos = anchor_pos.saturating_sub(anchor_idx);

        if pos + header.len() <= data.len()
            && pos < report_end
            && header_matches(header, data, pos)
        {
            hits.push(CarveHit {
                byte_offset: chunk_abs_offset + pos as u64,
                signature: sig.clone(),
            });
        }
        search_start = anchor_pos + 1;
    }

    hits
}

/// Check whether `header` matches `data` starting at `pos`.
///
/// `None` entries in `header` are wildcards and match any byte.
#[inline]
fn header_matches(header: &[Option<u8>], data: &[u8], pos: usize) -> bool {
    header
        .iter()
        .enumerate()
        .all(|(i, opt)| match opt {
            None => true,
            Some(b) => data.get(pos + i) == Some(b),
        })
}

// ── Streaming helpers ─────────────────────────────────────────────────────────

const EXTRACT_CHUNK: usize = 256 * 1024; // 256 KiB per extraction chunk

/// Write bytes from `[start, end)` on the device to `writer`.
fn stream_bytes(
    device: &dyn BlockDevice,
    start: u64,
    end: u64,
    writer: &mut dyn Write,
) -> Result<u64> {
    let mut pos = start;
    let mut written = 0u64;

    while pos < end {
        let to_read = EXTRACT_CHUNK.min((end - pos) as usize);
        let data = read_bytes_clamped(device, pos, to_read)?;
        if data.is_empty() {
            break;
        }
        writer
            .write_all(&data)
            .map_err(|e| CarveError::Io(e.to_string()))?;
        written += data.len() as u64;
        pos += data.len() as u64;
    }

    Ok(written)
}

/// Write from `start` up to and including `footer` (first occurrence), capped
/// at `max_end`.
///
/// Uses a carry-over `tail` buffer of `footer.len() - 1` bytes between chunks
/// so that footers straddling chunk boundaries are correctly detected.
fn stream_until_footer(
    device: &dyn BlockDevice,
    start: u64,
    max_end: u64,
    footer: &[u8],
    writer: &mut dyn Write,
) -> Result<u64> {
    let overlap = footer.len().saturating_sub(1);
    let mut pos = start;
    let mut written = 0u64;
    // Carry-over bytes from the previous chunk for cross-boundary matching.
    let mut tail: Vec<u8> = Vec::new();

    while pos < max_end {
        let to_read = EXTRACT_CHUNK.min((max_end - pos) as usize);
        let new_data = read_bytes_clamped(device, pos, to_read)?;
        if new_data.is_empty() {
            break;
        }

        // Prepend tail to handle footers that span the boundary.
        let combined: Vec<u8> = tail.iter().chain(new_data.iter()).copied().collect();

        if let Some(footer_pos) = memmem::find(&combined, footer) {
            let end = footer_pos + footer.len();
            // The tail is unwritten — write all of combined[0..end] (tail + new bytes
            // up to and including footer).
            writer
                .write_all(&combined[..end])
                .map_err(|e| CarveError::Io(e.to_string()))?;
            written += end as u64;
            return Ok(written);
        }

        // Footer not found yet — flush all of combined except the new overlap tail.
        // The tail is also unwritten, so we start from index 0.
        let flush_end = combined.len().saturating_sub(overlap);

        if flush_end > 0 {
            writer
                .write_all(&combined[..flush_end])
                .map_err(|e| CarveError::Io(e.to_string()))?;
            written += flush_end as u64;
        }

        // Keep the last `overlap` bytes as the new tail.
        tail = if combined.len() > overlap {
            combined[combined.len() - overlap..].to_vec()
        } else {
            combined
        };

        pos += new_data.len() as u64;
    }

    // Footer never found — flush remaining tail bytes.
    if !tail.is_empty() {
        writer
            .write_all(&tail)
            .map_err(|e| CarveError::Io(e.to_string()))?;
        written += tail.len() as u64;
    }

    Ok(written)
}

/// Write from `start` up to and including the **last** occurrence of `footer`
/// within `[start, max_end)`, capped at `max_end`.
///
/// Reads the full extraction window into memory in chunks, tracking every
/// footer match.  At the end, writes exactly `last_footer_end` bytes.
/// If no footer is found, writes the entire window (same behaviour as
/// [`stream_bytes`]).
///
/// This is the correct strategy for formats like PDF where the footer byte
/// sequence (`%%EOF`) can appear inside binary streams, and for incrementally
/// updated files that accumulate multiple EOF markers.
fn stream_until_last_footer(
    device: &dyn BlockDevice,
    start: u64,
    max_end: u64,
    footer: &[u8],
    writer: &mut dyn Write,
) -> Result<u64> {
    // We need to see all data before we can identify the last footer position.
    // Read in chunks and maintain a running candidate for the last match.
    let overlap = footer.len().saturating_sub(1);
    let mut pos = start;
    let mut buf: Vec<u8> = Vec::new();

    // Accumulate all bytes (respecting EXTRACT_CHUNK to avoid huge allocations
    // on single reads while still streaming in manageable pieces).
    while pos < max_end {
        let to_read = EXTRACT_CHUNK.min((max_end - pos) as usize);
        let chunk = read_bytes_clamped(device, pos, to_read)?;
        if chunk.is_empty() {
            break;
        }
        pos += chunk.len() as u64;
        buf.extend_from_slice(&chunk);
    }

    // Find the last occurrence of footer in the accumulated buffer.
    // `memmem::rfind` scans from the right, which is O(n) and avoids
    // iterating over every match manually.
    let write_end = if let Some(last_pos) = memmem::rfind(&buf, footer) {
        last_pos + footer.len()
    } else {
        // No footer found — write everything (same as stream_bytes).
        buf.len()
    };

    let _ = overlap; // overlap concept not needed here (full buffer in hand)
    writer
        .write_all(&buf[..write_end])
        .map_err(|e| CarveError::Io(e.to_string()))?;

    Ok(write_end as u64)
}

// ── Size hint reader ──────────────────────────────────────────────────────────

/// Read the embedded size hint from the device and return the implied total
/// file size.  Returns `None` if any read fails or the header is malformed.
fn read_size_hint(device: &dyn BlockDevice, file_offset: u64, hint: &SizeHint) -> Option<u64> {
    match hint {
        SizeHint::Linear { offset, len, little_endian, add } => {
            let field_offset = file_offset + *offset as u64;
            let bytes = read_bytes_clamped(device, field_offset, *len as usize).ok()?;
            if bytes.len() < *len as usize {
                return None;
            }
            let value: u64 = match len {
                2 => {
                    let arr: [u8; 2] = bytes[..2].try_into().ok()?;
                    if *little_endian { u16::from_le_bytes(arr) as u64 }
                    else              { u16::from_be_bytes(arr) as u64 }
                }
                4 => {
                    let arr: [u8; 4] = bytes[..4].try_into().ok()?;
                    if *little_endian { u32::from_le_bytes(arr) as u64 }
                    else              { u32::from_be_bytes(arr) as u64 }
                }
                8 => {
                    let arr: [u8; 8] = bytes[..8].try_into().ok()?;
                    if *little_endian { u64::from_le_bytes(arr) }
                    else              { u64::from_be_bytes(arr) }
                }
                _ => return None,
            };
            Some(value.saturating_add(*add))
        }

        SizeHint::Ole2 => {
            // uSectorShift (u16 LE) at offset 30: sector_size = 1 << uSectorShift
            // Valid values: 9 (512-byte sectors, version 3) or 12 (4096-byte, v4).
            let shift_bytes = read_bytes_clamped(device, file_offset + 30, 2).ok()?;
            let shift_arr: [u8; 2] = shift_bytes[..2].try_into().ok()?;
            let sector_shift = u16::from_le_bytes(shift_arr) as u32;
            if !(7..=16).contains(&sector_shift) {
                return None; // sanity-check: reject implausible values
            }
            let sector_size = 1u64 << sector_shift;

            // csectFat (u32 LE) at offset 44: number of FAT sectors.
            // Each FAT sector can reference (sector_size / 4) data sectors.
            let fat_bytes = read_bytes_clamped(device, file_offset + 44, 4).ok()?;
            let fat_arr: [u8; 4] = fat_bytes[..4].try_into().ok()?;
            let csect_fat = u32::from_le_bytes(fat_arr) as u64;

            // Upper bound: all addressable sectors occupied + 1 header sector.
            let addressable = csect_fat.saturating_mul(sector_size / 4);
            Some(addressable.saturating_add(1).saturating_mul(sector_size))
        }

        SizeHint::LinearScaled { offset, len, little_endian, scale, add } => {
            let field_offset = file_offset + *offset as u64;
            let bytes = read_bytes_clamped(device, field_offset, *len as usize).ok()?;
            if bytes.len() < *len as usize {
                return None;
            }
            let value: u64 = match len {
                2 => {
                    let arr: [u8; 2] = bytes[..2].try_into().ok()?;
                    if *little_endian { u16::from_le_bytes(arr) as u64 }
                    else              { u16::from_be_bytes(arr) as u64 }
                }
                4 => {
                    let arr: [u8; 4] = bytes[..4].try_into().ok()?;
                    if *little_endian { u32::from_le_bytes(arr) as u64 }
                    else              { u32::from_be_bytes(arr) as u64 }
                }
                8 => {
                    let arr: [u8; 8] = bytes[..8].try_into().ok()?;
                    if *little_endian { u64::from_le_bytes(arr) }
                    else              { u64::from_be_bytes(arr) }
                }
                _ => return None,
            };
            Some(value.saturating_mul(*scale).saturating_add(*add))
        }

        SizeHint::Sqlite => {
            // page_size: u16 BE at offset 16; value 1 encodes 65536.
            let ps_bytes = read_bytes_clamped(device, file_offset + 16, 2).ok()?;
            let ps_arr: [u8; 2] = ps_bytes[..2].try_into().ok()?;
            let raw_page_size = u16::from_be_bytes(ps_arr);
            let page_size: u64 = if raw_page_size == 1 { 65536 } else { raw_page_size as u64 };
            if page_size < 512 {
                return None; // corrupt or not an SQLite file
            }

            // db_pages: u32 BE at offset 28; 0 means not written (pre-3.7.0).
            let dp_bytes = read_bytes_clamped(device, file_offset + 28, 4).ok()?;
            let dp_arr: [u8; 4] = dp_bytes[..4].try_into().ok()?;
            let db_pages = u32::from_be_bytes(dp_arr) as u64;
            if db_pages == 0 {
                return None; // field not set — caller will fall back to max_size
            }

            Some(page_size.saturating_mul(db_pages))
        }

        SizeHint::SevenZip => {
            // Start header (32 bytes total):
            //   offset 12: NextHeaderOffset (u64 LE) — bytes from end of start header
            //   offset 20: NextHeaderSize   (u64 LE) — byte length of encoded header
            // total = 32 + NextHeaderOffset + NextHeaderSize
            let off_bytes = read_bytes_clamped(device, file_offset + 12, 8).ok()?;
            let off_arr: [u8; 8] = off_bytes[..8].try_into().ok()?;
            let next_offset = u64::from_le_bytes(off_arr);

            let sz_bytes = read_bytes_clamped(device, file_offset + 20, 8).ok()?;
            let sz_arr: [u8; 8] = sz_bytes[..8].try_into().ok()?;
            let next_size = u64::from_le_bytes(sz_arr);

            Some(32u64.saturating_add(next_offset).saturating_add(next_size))
        }
    }
}

// ── I/O helper ────────────────────────────────────────────────────────────────

/// Read up to `len` bytes starting at `offset`, clamped to device bounds.
///
/// Returns fewer bytes than `len` if near the device end.  Handles sector
/// alignment internally.
fn read_bytes_clamped(device: &dyn BlockDevice, offset: u64, len: usize) -> Result<Vec<u8>> {
    if len == 0 || offset >= device.size() {
        return Ok(Vec::new());
    }

    let available = (device.size() - offset) as usize;
    let len = len.min(available);

    let sector_size = device.sector_size() as u64;
    let start_sector = offset / sector_size;
    let end_sector = (offset + len as u64).div_ceil(sector_size);
    let sectors = (end_sector - start_sector) as usize;
    let buf_size = sectors * sector_size as usize;

    let mut buf = AlignedBuffer::new(buf_size, sector_size as usize);
    let bytes_read = device
        .read_at(start_sector * sector_size, &mut buf)
        .map_err(CarveError::BlockDevice)?;

    let start_in_buf = (offset % sector_size) as usize;
    let end_in_buf = (start_in_buf + len).min(bytes_read);

    if end_in_buf <= start_in_buf {
        return Ok(Vec::new());
    }

    Ok(buf.as_slice()[start_in_buf..end_in_buf].to_vec())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;
    use crate::signature::{CarvingConfig, Signature};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn sig(header: &[u8], footer: &[u8], max_size: u64) -> Signature {
        Signature {
            name: "Test".into(),
            extension: "bin".into(),
            header: header.iter().map(|&b| Some(b)).collect(),
            footer: footer.to_vec(),
            footer_last: false,
            max_size,
            size_hint: None,
        }
    }

    fn config_with(sigs: Vec<Signature>, chunk_size: usize) -> CarvingConfig {
        CarvingConfig {
            signatures: sigs,
            scan_chunk_size: chunk_size,
        }
    }

    fn device_from(data: Vec<u8>) -> Arc<dyn BlockDevice> {
        Arc::new(MockBlockDevice::new(data, 512))
    }

    // ── scan tests ────────────────────────────────────────────────────────────

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
        // Place at offset 3 — with chunk_size=4 this is pos=3 < 4, reported in chunk 0.
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

    // ── extract tests ─────────────────────────────────────────────────────────

    #[test]
    fn extract_no_footer_writes_max_size() {
        let data: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        let dev = device_from(data.clone());

        let hit = CarveHit {
            byte_offset: 0,
            signature: sig(&[data[0]], &[], 100),
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 100);
        assert_eq!(&out, &data[..100]);
    }

    #[test]
    fn extract_no_footer_capped_at_device_end() {
        let data = vec![0xAAu8; 200];
        let dev = device_from(data.clone());

        let hit = CarveHit {
            byte_offset: 150,
            signature: sig(&[0xAA], &[], 1_000_000), // max_size > device
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 50); // 200 - 150
        assert_eq!(out.len(), 50);
    }

    #[test]
    fn extract_with_footer_stops_after_footer() {
        // Data: [header ... data ... footer ... more data]
        // Should write exactly header + data + footer.
        let header = [0xFF, 0xD8, 0xFF];
        let footer = [0xFF, 0xD9];
        let mut data = vec![0u8; 1024];
        data[0..3].copy_from_slice(&header);
        data[10..12].copy_from_slice(&footer); // footer at byte 10
                                               // bytes 12-1023 should NOT be written

        let dev = device_from(data);
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig(&header, &footer, 1_000_000),
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        // Should have written bytes 0..12 (10 data bytes + 2 footer bytes)
        assert_eq!(written, 12, "expected 12 bytes written, got {written}");
        assert_eq!(out[10..12], footer);
    }

    #[test]
    fn extract_footer_spanning_extract_chunk_boundary() {
        // Place footer bytes so they span the EXTRACT_CHUNK (256 KiB) boundary.
        let footer = [0xDE, 0xAD];
        let boundary = EXTRACT_CHUNK; // 262144
        let mut data = vec![0x00u8; boundary + 512];
        // Footer: last byte in first extract chunk, first byte in second chunk.
        data[boundary - 1] = footer[0];
        data[boundary] = footer[1];

        let dev = device_from(data.clone());
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig(&[0x00], &footer, data.len() as u64),
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        let expected_len = boundary + 1; // up to and including second footer byte
        assert_eq!(written as usize, expected_len);
        assert_eq!(out[boundary - 1..=boundary], footer);
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
                Some(0x52), Some(0x49), Some(0x46), Some(0x46), // RIFF
                None, None, None, None,                          // size (wildcard)
                Some(0x41), Some(0x56), Some(0x49), Some(0x20), // AVI<space>
            ],
            footer: vec![],
            footer_last: false,
            max_size: 2_147_483_648,
            size_hint: None,
        };
        let wav_sig = Signature {
            name: "WAV".into(),
            extension: "wav".into(),
            header: vec![
                Some(0x52), Some(0x49), Some(0x46), Some(0x46), // RIFF
                None, None, None, None,                          // size (wildcard)
                Some(0x57), Some(0x41), Some(0x56), Some(0x45), // WAVE
            ],
            footer: vec![],
            footer_last: false,
            max_size: 2_147_483_648,
            size_hint: None,
        };
        let cfg = CarvingConfig { signatures: vec![avi_sig, wav_sig], scan_chunk_size: 1024 };
        let hits = Carver::new(dev, cfg).scan().unwrap();

        assert_eq!(hits.len(), 2, "expected AVI + WAV hits, got: {hits:?}");
        assert_eq!(hits[0].byte_offset, 0);
        assert_eq!(hits[0].signature.extension, "avi");
        assert_eq!(hits[1].byte_offset, 512);
        assert_eq!(hits[1].signature.extension, "wav");
    }

    #[test]
    fn extract_size_hint_limits_output() {
        // Build a fake RIFF-like file: header says 100 bytes of content.
        // Total file size = 100 + 8 = 108 bytes.
        // Device is padded to 4096; extractor must stop at 108, not max_size.
        let mut data = vec![0xAAu8; 4096];
        data[0..4].copy_from_slice(b"RIFF");
        data[4..8].copy_from_slice(&100u32.to_le_bytes()); // payload = 100 bytes
        data[8..12].copy_from_slice(b"AVI ");

        let dev = device_from(data);
        let sig = Signature {
            name: "AVI".into(),
            extension: "avi".into(),
            header: vec![Some(0x52), Some(0x49), Some(0x46), Some(0x46)],
            footer: vec![],
            footer_last: false,
            max_size: 2_147_483_648,
            size_hint: Some(crate::signature::SizeHint::Linear {
                offset: 4,
                len: 4,
                little_endian: true,
                add: 8,
            }),
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 108, "size_hint should limit extraction to 108 bytes, got {written}");
    }

    #[test]
    fn extract_ole2_size_hint_limits_output() {
        // Build a minimal OLE2 header:
        //   uSectorShift = 9  (sector_size = 512)
        //   csectFat     = 2  → 2 × (512/4) = 256 addressable sectors
        //   expected max = (256 + 1) × 512 = 131,584 bytes
        let mut data = vec![0u8; 512 * 1024]; // 512 KiB device
        // Magic bytes
        data[0..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
        // uSectorShift = 9 (little-endian u16 at offset 30)
        data[30..32].copy_from_slice(&9u16.to_le_bytes());
        // csectFat = 2 (little-endian u32 at offset 44)
        data[44..48].copy_from_slice(&2u32.to_le_bytes());

        let dev = device_from(data);
        let sig = Signature {
            name: "OLE2".into(),
            extension: "ole".into(),
            header: vec![
                Some(0xD0), Some(0xCF), Some(0x11), Some(0xE0),
                Some(0xA1), Some(0xB1), Some(0x1A), Some(0xE1),
            ],
            footer: vec![],
            footer_last: false,
            max_size: 524_288_000,
            size_hint: Some(crate::signature::SizeHint::Ole2),
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        // (2 × 128 + 1) × 512 = 131,584 — capped by device size (512 KiB = 524,288)
        assert_eq!(written, 131_584, "OLE2 size_hint gave {written}, expected 131584");
    }

    #[test]
    fn extract_linear_scaled_size_hint_limits_output() {
        // Simulate an EVTX header: chunk_count = 3 at offset 42 (u16 LE).
        // expected = 3 × 65536 + 4096 = 200,704 bytes
        let mut data = vec![0u8; 512 * 1024]; // 512 KiB device
        data[0..7].copy_from_slice(&[0x45, 0x4C, 0x46, 0x49, 0x4C, 0x45, 0x00]);
        data[42..44].copy_from_slice(&3u16.to_le_bytes());

        let dev = device_from(data);
        let sig = Signature {
            name: "EVTX".into(),
            extension: "evtx".into(),
            header: vec![
                Some(0x45), Some(0x4C), Some(0x46), Some(0x49),
                Some(0x4C), Some(0x45), Some(0x00),
            ],
            footer: vec![],
            footer_last: false,
            max_size: 104_857_600,
            size_hint: Some(crate::signature::SizeHint::LinearScaled {
                offset: 42,
                len: 2,
                little_endian: true,
                scale: 65536,
                add: 4096,
            }),
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 200_704, "EVTX size_hint gave {written}, expected 200704");
    }

    #[test]
    fn extract_sqlite_size_hint_limits_output() {
        // page_size = 4096 (u16 BE = 0x10_00 at offset 16)
        // db_pages  = 5    (u32 BE at offset 28)
        // expected  = 4096 × 5 = 20480 bytes
        let mut data = vec![0u8; 512 * 1024];
        data[0..16].copy_from_slice(b"SQLite format 3\0");
        data[16..18].copy_from_slice(&4096u16.to_be_bytes());
        data[28..32].copy_from_slice(&5u32.to_be_bytes());

        let dev = device_from(data);
        let sig = Signature {
            name: "SQLite".into(),
            extension: "db".into(),
            header: vec![
                Some(0x53), Some(0x51), Some(0x4C), Some(0x69),
                Some(0x74), Some(0x65), Some(0x20), Some(0x66),
                Some(0x6F), Some(0x72), Some(0x6D), Some(0x61),
                Some(0x74), Some(0x20), Some(0x33), Some(0x00),
            ],
            footer: vec![],
            footer_last: false,
            max_size: 10_737_418_240,
            size_hint: Some(crate::signature::SizeHint::Sqlite),
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 20_480, "SQLite size_hint gave {written}, expected 20480");
    }

    #[test]
    fn extract_seven_zip_size_hint_limits_output() {
        // NextHeaderOffset = 1000 (u64 LE at offset 12)
        // NextHeaderSize   = 200  (u64 LE at offset 20)
        // expected = 32 + 1000 + 200 = 1232 bytes
        let mut data = vec![0u8; 512 * 1024];
        data[0..6].copy_from_slice(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]);
        data[12..20].copy_from_slice(&1000u64.to_le_bytes());
        data[20..28].copy_from_slice(&200u64.to_le_bytes());

        let dev = device_from(data);
        let sig = Signature {
            name: "7-Zip".into(),
            extension: "7z".into(),
            header: vec![
                Some(0x37), Some(0x7A), Some(0xBC), Some(0xAF),
                Some(0x27), Some(0x1C),
            ],
            footer: vec![],
            footer_last: false,
            max_size: 524_288_000,
            size_hint: Some(crate::signature::SizeHint::SevenZip),
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 1_232, "7-Zip size_hint gave {written}, expected 1232");
    }

    #[test]
    fn extract_footer_last_stops_at_last_occurrence() {
        // Build data with footer appearing three times — extractor must stop
        // at the LAST one, not the first.
        let footer = [0xFF, 0xD9u8];
        let mut data = vec![0xAAu8; 1024];
        data[10..12].copy_from_slice(&footer); // first  (should NOT stop here)
        data[50..52].copy_from_slice(&footer); // second (should NOT stop here)
        data[200..202].copy_from_slice(&footer); // last  (should stop here → 202 bytes)

        let dev = device_from(data);
        let sig = Signature {
            name: "Test".into(),
            extension: "tst".into(),
            header: vec![Some(0xAA)],
            footer: footer.to_vec(),
            footer_last: true,
            max_size: 1024,
            size_hint: None,
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 202, "footer_last should stop after the last footer at offset 200, got {written}");
        assert_eq!(&out[200..202], &footer);
    }

    #[test]
    fn extract_footer_last_no_footer_writes_all() {
        // With footer_last and no footer present, writes the full max_size.
        let data = vec![0xBBu8; 500];
        let dev = device_from(data);
        let sig = Signature {
            name: "Test".into(),
            extension: "tst".into(),
            header: vec![Some(0xBB)],
            footer: vec![0xFF, 0xFE],
            footer_last: true,
            max_size: 300,
            size_hint: None,
        };
        let hit = CarveHit { byte_offset: 0, signature: sig };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();

        assert_eq!(written, 300);
    }

    #[test]
    fn extract_footer_not_found_writes_max_size() {
        // Footer never appears — should write exactly max_size bytes.
        let data = vec![0xAAu8; 1024];
        let dev = device_from(data);
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig(&[0xAA], &[0xFF, 0xFE], 300), // footer absent
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();
        assert_eq!(written, 300);
    }
}
