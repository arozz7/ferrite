//! Gap-tolerant sliding-window text block scanner.
//!
//! Reads the raw device stream in aligned chunks and identifies contiguous
//! regions of valid text content.  Each region is classified, quality-gated,
//! and deduplicated before being sent to the TUI via `tx`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::classifier::classify;
use crate::scanner::{TextBlock, TextScanConfig, TextScanMsg, TextScanProgress};

// ── Public entry point ────────────────────────────────────────────────────────

/// Run the text scan in the **calling thread**.
///
/// Spawn a thread around this call site for non-blocking behaviour:
/// ```ignore
/// std::thread::spawn(move || ferrite_textcarver::run_scan(device, config, tx, cancel));
/// ```
pub fn run_scan(
    device: Arc<dyn BlockDevice>,
    config: TextScanConfig,
    tx: Sender<TextScanMsg>,
    cancel: Arc<AtomicBool>,
) {
    let total_size = device.size();
    if total_size == 0 {
        let _ = tx.send(TextScanMsg::Done { total_blocks: 0 });
        return;
    }

    let chunk_size = config.chunk_bytes as usize;
    let overlap = config.overlap_bytes;

    let mut scanner = BlockScanner {
        config: &config,
        device: device.as_ref(),
        tx: &tx,
        seen_hashes: HashSet::new(),
        total_blocks: 0,
        // Sliding-window state.
        in_block: false,
        block_start_abs: 0,
        gap_run: 0,
        printable_count: 0,
        total_count: 0,
        block_buf: Vec::new(),
        // Batch accumulation.
        batch: Vec::new(),
        last_batch_time: Instant::now(),
    };

    let mut overlap_tail: Vec<u8> = Vec::new();
    let mut chunk_offset: u64 = 0;

    while chunk_offset < total_size {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let chunk_end = (chunk_offset + chunk_size as u64).min(total_size);
        let chunk_len = (chunk_end - chunk_offset) as usize;

        let raw = read_chunk(device.as_ref(), chunk_offset, chunk_len);
        if raw.is_empty() {
            break;
        }

        // Prepend overlap tail from the previous chunk.
        let window: Vec<u8> = if overlap_tail.is_empty() {
            raw.clone()
        } else {
            let mut w = overlap_tail.clone();
            w.extend_from_slice(&raw);
            w
        };

        // Absolute offset of window[0].
        let window_abs_base = chunk_offset.saturating_sub(overlap_tail.len() as u64);

        scanner.process_window(&window, window_abs_base, chunk_offset == chunk_end);

        // Save tail for next iteration.
        let tail_start = raw.len().saturating_sub(overlap);
        overlap_tail = raw[tail_start..].to_vec();

        // Emit progress every chunk.
        let _ = tx.send(TextScanMsg::Progress(TextScanProgress {
            bytes_done: chunk_end,
            bytes_total: total_size,
            blocks_found: scanner.total_blocks,
        }));

        chunk_offset = chunk_end;
    }

    // End of device — flush any open block.
    scanner.flush_block();
    scanner.flush_batch();

    let _ = tx.send(TextScanMsg::Done {
        total_blocks: scanner.total_blocks,
    });
}

// ── Internal scanner state ────────────────────────────────────────────────────

struct BlockScanner<'a> {
    config: &'a TextScanConfig,
    device: &'a dyn BlockDevice,
    tx: &'a Sender<TextScanMsg>,
    seen_hashes: HashSet<u64>,
    total_blocks: usize,

    // Sliding-window state.
    in_block: bool,
    block_start_abs: u64,
    gap_run: usize,
    printable_count: u64,
    total_count: u64,
    block_buf: Vec<u8>,

    // Output batch.
    batch: Vec<TextBlock>,
    last_batch_time: Instant,
}

impl<'a> BlockScanner<'a> {
    /// Process one window (overlap_tail + raw_chunk).
    fn process_window(&mut self, window: &[u8], window_abs_base: u64, _last: bool) {
        for (i, &byte) in window.iter().enumerate() {
            let abs = window_abs_base + i as u64;
            let text_like = is_text_byte(byte);
            let printable = is_printable_ascii(byte);

            if self.in_block {
                self.total_count += 1;
                if printable {
                    self.printable_count += 1;
                }

                if text_like {
                    self.gap_run = 0;
                    self.block_buf.push(byte);
                } else {
                    self.gap_run += 1;
                    self.block_buf.push(byte);

                    if self.gap_run > self.config.gap_tolerance_bytes {
                        // End the block at the last text byte (trim the gap).
                        let trim = self.gap_run;
                        let buf_len = self.block_buf.len();
                        self.block_buf.truncate(buf_len.saturating_sub(trim));
                        let trimmed_total = self.total_count.saturating_sub(trim as u64);
                        let trimmed_printable = self.printable_count;
                        self.emit_block(trimmed_printable, trimmed_total);
                        self.in_block = false;
                        self.gap_run = 0;
                        self.printable_count = 0;
                        self.total_count = 0;
                        self.block_buf.clear();
                    }
                }

                // Max block size reached — emit and immediately restart.
                let block_len = abs - self.block_start_abs + 1;
                if block_len >= self.config.max_block_bytes {
                    let pc = self.printable_count;
                    let tc = self.total_count;
                    self.emit_block(pc, tc);
                    self.in_block = false;
                    self.gap_run = 0;
                    self.printable_count = 0;
                    self.total_count = 0;
                    self.block_buf.clear();
                }
            } else {
                // Look for 3 consecutive text-like bytes to start a new block.
                if text_like {
                    self.block_buf.push(byte);
                    if self.block_buf.len() >= 3 {
                        self.in_block = true;
                        self.block_start_abs = abs - (self.block_buf.len() as u64 - 1);
                        self.gap_run = 0;
                        self.printable_count = self
                            .block_buf
                            .iter()
                            .filter(|&&b| is_printable_ascii(b))
                            .count() as u64;
                        self.total_count = self.block_buf.len() as u64;
                    }
                } else {
                    self.block_buf.clear();
                }
            }

            // Periodically flush batch (every ~50 blocks or 5 s).
            if self.batch.len() >= 50
                || (self.last_batch_time.elapsed().as_secs() >= 5 && !self.batch.is_empty())
            {
                self.flush_batch();
            }
        }
    }

    fn flush_block(&mut self) {
        if self.in_block {
            let pc = self.printable_count;
            let tc = self.total_count;
            self.emit_block(pc, tc);
            self.in_block = false;
            self.gap_run = 0;
            self.printable_count = 0;
            self.total_count = 0;
            self.block_buf.clear();
        }
    }

    fn emit_block(&mut self, printable_count: u64, total_count: u64) {
        let buf = &self.block_buf;
        let length = buf.len() as u64;

        if length < self.config.min_block_bytes {
            return;
        }

        // Quality gate.
        let quality_pct = if total_count == 0 {
            0u8
        } else {
            ((printable_count * 100) / total_count).min(100) as u8
        };
        if quality_pct < self.config.min_printable_pct {
            return;
        }

        // Dedup via content hash.
        let hash = hash_bytes(buf);
        if !self.seen_hashes.insert(hash) {
            return;
        }

        // Read the block content for classification and preview.
        let content = read_chunk(self.device, self.block_start_abs, length as usize);
        if content.is_empty() {
            return;
        }

        let (kind, confidence, extension) = classify(&content);
        let preview = make_preview(&content, 80);

        let block = TextBlock {
            byte_offset: self.block_start_abs,
            length,
            kind,
            extension,
            confidence,
            quality: quality_pct,
            preview,
        };

        self.batch.push(block);
        self.total_blocks += 1;
    }

    fn flush_batch(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.batch);
        let _ = self.tx.send(TextScanMsg::BlockBatch(batch));
        self.last_batch_time = Instant::now();
    }
}

// ── Byte classification ───────────────────────────────────────────────────────

/// Returns `true` for bytes that are considered "text-like" in the scanner.
///
/// Includes ASCII printable/whitespace and valid UTF-8 lead/continuation bytes.
/// Binary sentinel bytes (0x00–0x08, 0x0B–0x0C, 0x0E–0x1F, 0x7F, 0xC0–0xC1,
/// 0xF5–0xFF) return `false`.
#[inline]
fn is_text_byte(b: u8) -> bool {
    matches!(b,
        0x09 | 0x0A | 0x0D       // tab, LF, CR
        | 0x20..=0x7E             // ASCII printable
        | 0x80..=0xBF             // UTF-8 continuation bytes
        | 0xC2..=0xF4             // UTF-8 lead bytes (2–4 byte sequences)
    )
}

/// Returns `true` for bytes in the printable ASCII range (used for quality scoring).
#[inline]
fn is_printable_ascii(b: u8) -> bool {
    matches!(b, 0x09 | 0x0A | 0x0D | 0x20..=0x7E)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn hash_bytes(data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    data.hash(&mut h);
    h.finish()
}

/// Build a preview string: first `max_chars` chars, newlines → `↵`.
fn make_preview(data: &[u8], max_chars: usize) -> String {
    let s = String::from_utf8_lossy(data);
    s.chars()
        .take(max_chars)
        .map(|c| if c == '\n' || c == '\r' { '↵' } else { c })
        .collect()
}

/// Read `len` bytes from `device` at `offset`, handling sector alignment.
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
            tracing::warn!(?e, offset, "textcarver: read error");
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_ascii_classified_correctly() {
        assert!(is_text_byte(b'A'));
        assert!(is_text_byte(b' '));
        assert!(is_text_byte(b'\n'));
        assert!(is_text_byte(b'\t'));
    }

    #[test]
    fn binary_bytes_not_text_like() {
        assert!(!is_text_byte(0x00)); // NUL
        assert!(!is_text_byte(0x01));
        assert!(!is_text_byte(0x7F)); // DEL
        assert!(!is_text_byte(0xFF));
    }

    #[test]
    fn utf8_continuation_is_text_like() {
        assert!(is_text_byte(0x80));
        assert!(is_text_byte(0xBF));
        assert!(is_text_byte(0xC2));
        assert!(is_text_byte(0xF4));
    }

    #[test]
    fn make_preview_truncates_and_replaces_newlines() {
        let data = b"hello\nworld";
        let preview = make_preview(data, 8);
        assert_eq!(preview, "hello↵wo");
    }

    #[test]
    fn make_preview_short_input() {
        let data = b"hi";
        let preview = make_preview(data, 80);
        assert_eq!(preview, "hi");
    }

    #[test]
    fn hash_bytes_is_deterministic() {
        let a = hash_bytes(b"hello");
        let b = hash_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_bytes_differs_for_different_inputs() {
        assert_ne!(hash_bytes(b"hello"), hash_bytes(b"world"));
    }
}
