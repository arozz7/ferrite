//! Extraction I/O helpers: streaming, size-hint reading, and clamped device reads.
//!
//! Split from `scanner.rs` to keep both files under the 600-line limit.

use std::io::Write;

use memchr::memmem;

use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::error::{CarveError, Result};
use crate::signature::SizeHint;

// ── Streaming helpers ─────────────────────────────────────────────────────────

pub(crate) const EXTRACT_CHUNK: usize = 256 * 1024; // 256 KiB per extraction chunk

/// Write bytes from `[start, end)` on the device to `writer`.
pub(crate) fn stream_bytes(
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
pub(crate) fn stream_until_footer(
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
/// Reads the full window in chunks, then writes up to the last footer match.
/// If no footer is found, writes the entire window (same as [`stream_bytes`]).
pub(crate) fn stream_until_last_footer(
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
pub(crate) fn read_size_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    hint: &SizeHint,
) -> Option<u64> {
    match hint {
        SizeHint::Linear {
            offset,
            len,
            little_endian,
            add,
        } => {
            let field_offset = file_offset + *offset as u64;
            let bytes = read_bytes_clamped(device, field_offset, *len as usize).ok()?;
            if bytes.len() < *len as usize {
                return None;
            }
            let value: u64 = match len {
                2 => {
                    let arr: [u8; 2] = bytes[..2].try_into().ok()?;
                    if *little_endian {
                        u16::from_le_bytes(arr) as u64
                    } else {
                        u16::from_be_bytes(arr) as u64
                    }
                }
                4 => {
                    let arr: [u8; 4] = bytes[..4].try_into().ok()?;
                    if *little_endian {
                        u32::from_le_bytes(arr) as u64
                    } else {
                        u32::from_be_bytes(arr) as u64
                    }
                }
                8 => {
                    let arr: [u8; 8] = bytes[..8].try_into().ok()?;
                    if *little_endian {
                        u64::from_le_bytes(arr)
                    } else {
                        u64::from_be_bytes(arr)
                    }
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

        SizeHint::LinearScaled {
            offset,
            len,
            little_endian,
            scale,
            add,
        } => {
            let field_offset = file_offset + *offset as u64;
            let bytes = read_bytes_clamped(device, field_offset, *len as usize).ok()?;
            if bytes.len() < *len as usize {
                return None;
            }
            let value: u64 = match len {
                2 => {
                    let arr: [u8; 2] = bytes[..2].try_into().ok()?;
                    if *little_endian {
                        u16::from_le_bytes(arr) as u64
                    } else {
                        u16::from_be_bytes(arr) as u64
                    }
                }
                4 => {
                    let arr: [u8; 4] = bytes[..4].try_into().ok()?;
                    if *little_endian {
                        u32::from_le_bytes(arr) as u64
                    } else {
                        u32::from_be_bytes(arr) as u64
                    }
                }
                8 => {
                    let arr: [u8; 8] = bytes[..8].try_into().ok()?;
                    if *little_endian {
                        u64::from_le_bytes(arr)
                    } else {
                        u64::from_be_bytes(arr)
                    }
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
            let page_size: u64 = if raw_page_size == 1 {
                65536
            } else {
                raw_page_size as u64
            };
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

        SizeHint::OggStream => {
            // Walk Ogg pages from `file_offset` until the end-of-stream (EOS) page.
            //
            // Ogg page layout (all offsets relative to the start of this page):
            //   [0..4]   capture_pattern "OggS" (4F 67 67 53)
            //   [4]      stream_structure_version (must be 0)
            //   [5]      header_type_flag — bit 2 (0x04) = EOS (last page)
            //   [6..14]  granule_position (u64 LE)
            //   [14..18] bitstream_serial_number (u32 LE)
            //   [18..22] page_sequence_number (u32 LE)
            //   [22..26] CRC checksum (u32 LE)
            //   [26]     number_page_segments (u8)
            //   [27..27+N] segment_table[N] — each byte is a lace value
            //   data: sum(segment_table) bytes
            //
            // page_size = 27 + num_segments + sum(segment_table)
            //
            // Returns the total file size when EOS page is found, None otherwise.

            const OGG_MAGIC: &[u8; 4] = b"OggS";
            // Safety cap: abort if no EOS found within 100 000 pages (~typical
            // for a multi-hour audio file at 44 Hz page rate) to prevent hangs
            // on corrupt data.
            const MAX_PAGES: u32 = 100_000;

            let device_size = device.size();
            let mut pos = file_offset;

            for _ in 0..MAX_PAGES {
                if pos >= device_size {
                    break;
                }
                // Read the fixed part of the page header (27 bytes).
                let hdr = read_bytes_clamped(device, pos, 27).ok()?;
                if hdr.len() < 27 {
                    break;
                }
                if &hdr[0..4] != OGG_MAGIC {
                    break; // lost sync — corrupt or overlapping hit
                }
                let header_type = hdr[5];
                let num_segments = hdr[26] as u64;

                // Read segment table to determine data payload length.
                let seg_table = read_bytes_clamped(device, pos + 27, num_segments as usize).ok()?;
                if seg_table.len() < num_segments as usize {
                    break;
                }
                let data_size: u64 = seg_table.iter().map(|&b| b as u64).sum();
                let page_size = 27 + num_segments + data_size;

                if header_type & 0x04 != 0 {
                    // EOS page found — total file size = distance from start to end of this page.
                    return Some(pos - file_offset + page_size);
                }

                pos = pos.saturating_add(page_size);
            }

            None // EOS not found within limit; fall back to max_size
        }
    }
}

// ── I/O helper ────────────────────────────────────────────────────────────────

/// Read up to `len` bytes starting at `offset`, clamped to device bounds.
///
/// Returns fewer bytes than `len` if near the device end.  Handles sector
/// alignment internally.
pub(crate) fn read_bytes_clamped(
    device: &dyn BlockDevice,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>> {
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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::{BlockDevice, MockBlockDevice};

    use super::*;
    use crate::scanner::{CarveHit, Carver};
    use crate::signature::{CarvingConfig, Signature};

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
            pre_validate_zip: false,
        }
    }

    fn device_from(data: Vec<u8>) -> Arc<dyn BlockDevice> {
        Arc::new(MockBlockDevice::new(data, 512))
    }

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
            signature: sig(&[0xAA], &[], 1_000_000),
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();
        assert_eq!(written, 50);
        assert_eq!(out.len(), 50);
    }

    #[test]
    fn extract_with_footer_stops_after_footer() {
        let header = [0xFF, 0xD8, 0xFF];
        let footer = [0xFF, 0xD9];
        let mut data = vec![0u8; 1024];
        data[0..3].copy_from_slice(&header);
        data[10..12].copy_from_slice(&footer);

        let dev = device_from(data);
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig(&header, &footer, 1_000_000),
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();
        assert_eq!(written, 12, "expected 12 bytes written, got {written}");
        assert_eq!(out[10..12], footer);
    }

    #[test]
    fn extract_footer_spanning_extract_chunk_boundary() {
        let footer = [0xDE, 0xAD];
        let boundary = EXTRACT_CHUNK;
        let mut data = vec![0x00u8; boundary + 512];
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
        let expected_len = boundary + 1;
        assert_eq!(written as usize, expected_len);
        assert_eq!(out[boundary - 1..=boundary], footer);
    }

    #[test]
    fn extract_footer_last_stops_at_last_occurrence() {
        let footer = [0xFF, 0xD9u8];
        let mut data = vec![0xAAu8; 1024];
        data[10..12].copy_from_slice(&footer);
        data[50..52].copy_from_slice(&footer);
        data[200..202].copy_from_slice(&footer);

        let dev = device_from(data);
        let sig = Signature {
            name: "Test".into(),
            extension: "tst".into(),
            header: vec![Some(0xAA)],
            footer: footer.to_vec(),
            footer_last: true,
            max_size: 1024,
            size_hint: None,
            min_size: 0,
            pre_validate_zip: false,
        };
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig,
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();
        assert_eq!(written, 202, "footer_last should stop after last footer");
        assert_eq!(&out[200..202], &footer);
    }

    #[test]
    fn extract_footer_last_no_footer_writes_all() {
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
            min_size: 0,
            pre_validate_zip: false,
        };
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig,
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();
        assert_eq!(written, 300);
    }

    #[test]
    fn extract_footer_not_found_writes_max_size() {
        let data = vec![0xAAu8; 1024];
        let dev = device_from(data);
        let sig_val = sig(&[0xAA], &[0xFF, 0xFE], 300);
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig_val,
        };
        let mut out = Vec::new();
        let written = Carver::new(dev, CarvingConfig::default())
            .extract(&hit, &mut out)
            .unwrap();
        assert_eq!(written, 300);
    }
}
