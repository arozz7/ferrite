//! Scan engine: orchestrates all artifact scanners over a block device.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::scanner::{ArtifactHit, ArtifactKind, ArtifactScanner};
use crate::scanners::{
    CreditCardScanner, EmailScanner, IbanScanner, SsnScanner, UrlScanner, WinPathScanner,
};

// ── Config & progress types ───────────────────────────────────────────────────

/// Configuration for an artifact scan run.
pub struct ArtifactScanConfig {
    /// Bytes per scan chunk (default: 4 MiB).
    pub chunk_size: usize,
    /// Bytes of overlap between consecutive chunks to catch cross-boundary
    /// hits (default: 512 B — covers all built-in pattern lengths).
    pub overlap_bytes: usize,
    /// Which artifact kinds to scan for.  Empty = all built-ins enabled.
    pub enabled_kinds: Vec<ArtifactKind>,
    /// First byte to scan from (0 = start of device).
    pub start_byte: u64,
    /// Last byte to scan up to, exclusive (None = full device).
    pub end_byte: Option<u64>,
}

impl Default for ArtifactScanConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024,
            overlap_bytes: 512,
            enabled_kinds: Vec::new(),
            start_byte: 0,
            end_byte: None,
        }
    }
}

/// Snapshot of scan progress sent to the TUI on every chunk.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub hits_found: usize,
}

// ── Channel messages ──────────────────────────────────────────────────────────

/// Messages streamed from the background scan thread to the TUI.
pub enum ScanMsg {
    Progress(ScanProgress),
    HitBatch(Vec<ArtifactHit>),
    Done { total_hits: usize },
    Error(String),
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Build the enabled scanner list from the config.
fn build_scanners(enabled: &[ArtifactKind]) -> Vec<Box<dyn ArtifactScanner>> {
    let all: Vec<Box<dyn ArtifactScanner>> = vec![
        Box::new(EmailScanner),
        Box::new(UrlScanner),
        Box::new(CreditCardScanner),
        Box::new(IbanScanner),
        Box::new(WinPathScanner),
        Box::new(SsnScanner),
    ];
    if enabled.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|s| enabled.contains(&s.kind()))
            .collect()
    }
}

/// Read `len` bytes from `device` starting at `offset`, handling sector
/// alignment.  Returns fewer bytes at end-of-device.
fn read_chunk(device: &dyn BlockDevice, offset: u64, len: usize) -> Vec<u8> {
    if len == 0 || offset >= device.size() {
        return Vec::new();
    }
    let available = (device.size() - offset) as usize;
    let len = len.min(available);

    let sector_size = device.sector_size() as u64;
    let start_sector = offset / sector_size;
    let end_sector = (offset + len as u64).div_ceil(sector_size);
    let sectors = (end_sector - start_sector) as usize;
    let buf_size = sectors * sector_size as usize;

    let mut buf = AlignedBuffer::new(buf_size, sector_size as usize);
    let bytes_read = match device.read_at(start_sector * sector_size, &mut buf) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(?e, offset, "artifact engine: read error");
            return Vec::new();
        }
    };

    let start_in_buf = (offset % sector_size) as usize;
    let end_in_buf = (start_in_buf + len).min(bytes_read);
    if end_in_buf <= start_in_buf {
        return Vec::new();
    }
    buf.as_slice()[start_in_buf..end_in_buf].to_vec()
}

/// Run the artifact scan in the **calling thread** (spawn a thread around this
/// call site if you want non-blocking behaviour).
///
/// Results are streamed via `tx`.  Hitting the cancel flag stops the scan
/// cleanly after the current chunk.
pub fn run_scan(
    device: Arc<dyn BlockDevice>,
    config: ArtifactScanConfig,
    tx: Sender<ScanMsg>,
    cancel: Arc<AtomicBool>,
) {
    let scanners = build_scanners(&config.enabled_kinds);
    // Per-kind dedup set — same value at different offsets counts once.
    let mut seen: HashMap<ArtifactKind, HashSet<String>> = HashMap::new();

    let total_size = device.size();
    let scan_start = config.start_byte;
    let scan_end = config.end_byte.unwrap_or(total_size).min(total_size);
    let total_bytes = scan_end.saturating_sub(scan_start);

    if total_bytes == 0 {
        let _ = tx.send(ScanMsg::Done { total_hits: 0 });
        return;
    }

    let chunk_size = config.chunk_size;
    let overlap = config.overlap_bytes;

    let mut bytes_scanned: u64 = 0;
    let mut total_hits: usize = 0;
    // Tail buffer from the previous chunk for cross-boundary matching.
    let mut tail: Vec<u8> = Vec::new();

    let mut chunk_start = scan_start;

    while chunk_start < scan_end {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let chunk_end = (chunk_start + chunk_size as u64).min(scan_end);
        let chunk_len = (chunk_end - chunk_start) as usize;

        let chunk = read_chunk(device.as_ref(), chunk_start, chunk_len);
        if chunk.is_empty() {
            break;
        }

        // Build scan buffer: overlap tail from previous chunk + this chunk.
        let mut scan_buf = Vec::with_capacity(tail.len() + chunk.len());
        scan_buf.extend_from_slice(&tail);
        scan_buf.extend_from_slice(&chunk);

        // The byte offset of the first byte in `scan_buf`.
        let scan_buf_offset = chunk_start.saturating_sub(tail.len() as u64);

        let mut hit_batch: Vec<ArtifactHit> = Vec::new();
        for scanner in &scanners {
            let hits = scanner.scan_block(&scan_buf, scan_buf_offset);
            for hit in hits {
                // Only emit hits whose offset falls within this chunk window
                // (not in the overlap region belonging to the previous chunk).
                if hit.byte_offset < chunk_start {
                    continue;
                }
                let entry = seen.entry(hit.kind).or_default();
                if entry.insert(hit.value.clone()) {
                    total_hits += 1;
                    hit_batch.push(hit);
                }
            }
        }

        if !hit_batch.is_empty() && tx.send(ScanMsg::HitBatch(hit_batch)).is_err() {
            return; // receiver dropped — TUI gone
        }

        // Update tail for the next iteration.
        if chunk.len() >= overlap {
            tail = chunk[chunk.len() - overlap..].to_vec();
        } else {
            tail = chunk;
        }

        bytes_scanned += chunk_len as u64;
        let _ = tx.send(ScanMsg::Progress(ScanProgress {
            bytes_done: bytes_scanned,
            bytes_total: total_bytes,
            hits_found: total_hits,
        }));

        chunk_start = chunk_end;
    }

    let _ = tx.send(ScanMsg::Done { total_hits });
}
