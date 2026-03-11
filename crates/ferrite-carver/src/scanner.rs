//! Parallel file-carving scanner and extractor.
//!
//! [`Carver`] reads the device in overlapping chunks, uses [`memchr`] for fast
//! first-byte matching, and [`rayon`] to search all signatures concurrently
//! within each chunk.  Footer detection uses [`memchr::memmem`] over a
//! sliding window to handle footers that span chunk boundaries.

use std::io::Write;
use std::sync::Arc;

use memchr::memmem;
use rayon::prelude::*;
use tracing::{trace, warn};

use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::error::{CarveError, Result};
use crate::signature::{CarvingConfig, Signature};

// ── Public types ──────────────────────────────────────────────────────────────

/// A single file-carving hit returned by [`Carver::scan`].
#[derive(Debug, Clone)]
pub struct CarveHit {
    /// Absolute byte offset of the file header on the device.
    pub byte_offset: u64,
    /// The signature that matched.
    pub signature: Signature,
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
    ///
    /// The device is read in `config.scan_chunk_size` increments.  An overlap
    /// region of `max(header_len) - 1` bytes is appended to each chunk so
    /// that headers straddling chunk boundaries are never missed.
    pub fn scan(&self) -> Result<Vec<CarveHit>> {
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

        let max_end = (hit.byte_offset + sig.max_size).min(device_size);

        if sig.footer.is_empty() {
            stream_bytes(self.device.as_ref(), hit.byte_offset, max_end, writer)
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
/// Uses [`memchr`] for fast single-byte scanning followed by a full-header
/// equality check.
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

    let report_end = report_end.min(data.len());
    let first_byte = header[0];
    let mut hits = Vec::new();
    let mut search_start = 0usize;

    loop {
        if search_start >= report_end {
            break;
        }
        let window = &data[search_start..report_end];
        let Some(rel) = memchr::memchr(first_byte, window) else {
            break;
        };
        let pos = search_start + rel;

        if data.get(pos..pos + header.len()) == Some(header.as_slice()) {
            hits.push(CarveHit {
                byte_offset: chunk_abs_offset + pos as u64,
                signature: sig.clone(),
            });
        }
        search_start = pos + 1;
    }

    hits
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
            header: header.to_vec(),
            footer: footer.to_vec(),
            max_size,
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
