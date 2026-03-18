//! Size-hint readers: derive the true file size from embedded header fields.
//!
//! Each [`SizeHint`] variant has a corresponding arm here that reads the
//! minimum number of bytes from the device and returns the implied total file
//! size, or `None` when the header is malformed or a read fails (the caller
//! falls back to `max_size`).

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;
use crate::signature::SizeHint;

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

        SizeHint::Tiff => tiff_size_hint(device, file_offset),

        SizeHint::Raf => raf_size_hint(device, file_offset),

        SizeHint::Isobmff => {
            // Walk ISO Base Media File Format top-level boxes (MP4, MOV, M4A, …).
            //
            // Box layout (ISO 14496-12 §4.2):
            //   [0..4]  box_size  — u32 BE; 0 = extends to EOF, 1 = largesize follows
            //   [4..8]  box_type  — 4 bytes; printable ASCII (0x20–0x7E) for valid boxes
            //   [8..16] largesize — u64 BE (only present when box_size == 1)
            //
            // Walk sequential top-level boxes, summing their sizes.  Stop when:
            //   - box_type has non-printable bytes (sync lost / end of file data)
            //   - box_size == 0 (EOF-extending; true size unknowable)
            //   - box_size is invalid (< 8)
            //   - safety cap of 2 000 boxes is reached
            //
            // Returns None when no valid boxes are found or on any read error.

            const MAX_BOXES: u32 = 2_000;
            let device_size = device.size();
            let mut pos = file_offset;
            let mut total: u64 = 0;

            for _ in 0..MAX_BOXES {
                if pos + 8 > device_size {
                    break;
                }
                let hdr = read_bytes_clamped(device, pos, 8).ok()?;
                if hdr.len() < 8 {
                    break;
                }
                let box_size_raw = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
                let box_type = [hdr[4], hdr[5], hdr[6], hdr[7]];

                // All 4 type bytes must be printable ASCII.
                if !box_type.iter().all(|b| (0x20..=0x7E).contains(b)) {
                    break;
                }

                let actual_size: u64 = match box_size_raw {
                    // 0 = extends to EOF — size unknowable without an EOF marker.
                    0 => break,
                    // 1 = largesize: real size is a u64 BE at offset +8 in the box.
                    1 => {
                        if pos + 16 > device_size {
                            break;
                        }
                        let ls = read_bytes_clamped(device, pos + 8, 8).ok()?;
                        if ls.len() < 8 {
                            break;
                        }
                        let sz = u64::from_be_bytes(ls[..8].try_into().ok()?);
                        // largesize must cover at least size(4)+type(4)+largesize(8).
                        if sz < 16 {
                            break;
                        }
                        sz
                    }
                    // Minimum valid box is 8 bytes (size + type fields).
                    n if n < 8 => break,
                    n => n as u64,
                };

                total = total.saturating_add(actual_size);
                pos = pos.saturating_add(actual_size);
            }

            if total > 0 {
                Some(total)
            } else {
                None
            }
        }
    }
}

// ── TIFF IFD walker ───────────────────────────────────────────────────────────

/// Walk the TIFF IFD chain and return the maximum byte extent referenced by
/// any external data block, strip/tile offset+length pair, or SubIFD pointer.
///
/// Supports both little-endian (`II`) and big-endian (`MM`) byte orders, and
/// the Panasonic RW2 variant magic (`0x55`).  SubIFDs reached via tag `0x014A`
/// are queued for traversal so raw sensor IFDs are always visited.
fn tiff_size_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    use std::collections::HashSet;

    /// Bytes-per-element for each TIFF type code (1–12; 0 is invalid).
    fn type_bytes(t: u16) -> Option<u64> {
        match t {
            1 | 2 | 6 | 7 => Some(1), // BYTE, ASCII, SBYTE, UNDEFINED
            3 | 8 => Some(2),         // SHORT, SSHORT
            4 | 9 | 11 => Some(4),    // LONG, SLONG, FLOAT
            5 | 10 | 12 => Some(8),   // RATIONAL, SRATIONAL, DOUBLE
            _ => None,
        }
    }

    let hdr = read_bytes_clamped(device, file_offset, 8).ok()?;
    if hdr.len() < 8 {
        return None;
    }
    let le = match [hdr[0], hdr[1]] {
        [0x49, 0x49] => true,  // "II" little-endian
        [0x4D, 0x4D] => false, // "MM" big-endian
        _ => return None,
    };
    let ru16 = |b: &[u8]| -> u16 {
        if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    };
    let ru32 = |b: &[u8]| -> u32 {
        if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
    };

    let ifd0_off = ru32(&hdr[4..8]) as u64;
    if ifd0_off == 0 {
        return None;
    }

    let device_size = device.size();
    let mut max_extent: u64 = 8;
    let mut ifd_queue: Vec<u64> = vec![ifd0_off];
    let mut visited: HashSet<u64> = HashSet::new();

    while let Some(ifd_off) = ifd_queue.pop() {
        if ifd_off == 0 || visited.contains(&ifd_off) || visited.len() >= 64 {
            continue;
        }
        visited.insert(ifd_off);

        let abs_ifd = file_offset.saturating_add(ifd_off);
        if abs_ifd + 2 > device_size {
            continue;
        }
        let cnt_b = match read_bytes_clamped(device, abs_ifd, 2) {
            Ok(b) if b.len() >= 2 => b,
            _ => continue,
        };
        let entry_count = ru16(&cnt_b) as u64;
        if entry_count == 0 || entry_count > 1000 {
            continue;
        }

        // Account for the IFD table itself (2-byte count + entries + 4-byte next link).
        max_extent = max_extent.max(ifd_off + 2 + entry_count * 12 + 4);

        // Paired strip/tile/JPEG data-pointer tags collected during entry scan.
        let mut strip_offs: Vec<u64> = Vec::new();
        let mut strip_lens: Vec<u64> = Vec::new();
        let mut tile_offs: Vec<u64> = Vec::new();
        let mut tile_lens: Vec<u64> = Vec::new();
        let mut jpeg_off_tag: Option<u64> = None;
        let mut jpeg_len_tag: Option<u64> = None;

        for i in 0..entry_count {
            let ep = abs_ifd + 2 + i * 12;
            if ep + 12 > device_size {
                break;
            }
            let entry = match read_bytes_clamped(device, ep, 12) {
                Ok(b) if b.len() == 12 => b,
                _ => break,
            };
            let tag = ru16(&entry[0..2]);
            let type_id = ru16(&entry[2..4]);
            let count = ru32(&entry[4..8]) as u64;
            let val4 = &entry[8..12];

            let tsz = match type_bytes(type_id) {
                Some(s) => s,
                None => continue,
            };
            let data_bytes = count.saturating_mul(tsz);
            let is_ext = data_bytes > 4;
            let ext_off_val = ru32(val4) as u64; // meaningful only when is_ext

            if is_ext {
                // External data block: its bytes are inside the file.
                max_extent = max_extent.max(ext_off_val + data_bytes);
            }

            match tag {
                // SubIFD (tag 0x014A) — Sony, Canon, and others store the RAW data IFD here.
                0x014A => {
                    if is_ext {
                        let abs_ext = file_offset + ext_off_val;
                        let read_len = (count * 4).min(256) as usize;
                        if let Ok(data) = read_bytes_clamped(device, abs_ext, read_len) {
                            for j in 0..(count.min(64)) as usize {
                                if j * 4 + 4 > data.len() {
                                    break;
                                }
                                let sub = ru32(&data[j * 4..j * 4 + 4]) as u64;
                                if sub > 0 {
                                    ifd_queue.push(sub);
                                }
                            }
                        }
                    } else if ext_off_val > 0 {
                        ifd_queue.push(ext_off_val);
                    }
                }

                // Strip / tile / JPEG thumbnail pointer tags.
                0x0111 | 0x0117 | 0x0144 | 0x0145 | 0x0201 | 0x0202 => {
                    if type_id != 3 && type_id != 4 {
                        continue; // only SHORT and LONG are used for offsets/counts
                    }
                    let elem_sz = if type_id == 3 { 2usize } else { 4usize };
                    let vals: Vec<u64> = if is_ext {
                        let abs_ext = file_offset + ext_off_val;
                        let read_len = (count as usize).saturating_mul(elem_sz).min(512 * 1024);
                        match read_bytes_clamped(device, abs_ext, read_len) {
                            Ok(data) => {
                                let actual = (data.len() / elem_sz).min(count as usize);
                                (0..actual)
                                    .map(|j| {
                                        if type_id == 3 {
                                            ru16(&data[j * 2..j * 2 + 2]) as u64
                                        } else {
                                            ru32(&data[j * 4..j * 4 + 4]) as u64
                                        }
                                    })
                                    .collect()
                            }
                            Err(_) => vec![],
                        }
                    } else {
                        match (type_id, count) {
                            (4, 1) => vec![ru32(val4) as u64],
                            (3, 1) => vec![ru16(val4) as u64],
                            (3, 2) => {
                                vec![ru16(&val4[0..2]) as u64, ru16(&val4[2..4]) as u64]
                            }
                            _ => vec![],
                        }
                    };

                    match tag {
                        0x0111 => strip_offs = vals,
                        0x0117 => strip_lens = vals,
                        0x0144 => tile_offs = vals,
                        0x0145 => tile_lens = vals,
                        0x0201 => jpeg_off_tag = vals.first().copied(),
                        0x0202 => jpeg_len_tag = vals.first().copied(),
                        _ => {}
                    }
                }

                _ => {}
            }
        }

        // Pair strip/tile/JPEG offset+length tags to get data extents.
        for (off, len) in strip_offs.iter().zip(strip_lens.iter()) {
            max_extent = max_extent.max(off + len);
        }
        for (off, len) in tile_offs.iter().zip(tile_lens.iter()) {
            max_extent = max_extent.max(off + len);
        }
        if let (Some(off), Some(len)) = (jpeg_off_tag, jpeg_len_tag) {
            max_extent = max_extent.max(off + len);
        }

        // Follow the next-IFD link (4 bytes after the last entry).
        let next_pos = abs_ifd + 2 + entry_count * 12;
        if next_pos + 4 <= device_size {
            if let Ok(nb) = read_bytes_clamped(device, next_pos, 4) {
                if nb.len() >= 4 {
                    let next = ru32(&nb) as u64;
                    if next > 0 {
                        ifd_queue.push(next);
                    }
                }
            }
        }
    }

    if max_extent > 8 {
        Some(max_extent)
    } else {
        None
    }
}

/// Derive the true file size from a Fujifilm RAF header.
///
/// The RAF header stores two data extents at fixed big-endian u32 offsets:
/// - offset  84: JPEG preview offset
/// - offset  88: JPEG preview length
/// - offset  92: CFA (raw sensor) data offset
/// - offset  96: CFA length
///
/// Returns the end of whichever extent is larger.
fn raf_size_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let hdr = read_bytes_clamped(device, file_offset + 84, 16).ok()?;
    if hdr.len() < 16 {
        return None;
    }
    let jpeg_off = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
    let jpeg_len = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
    let cfa_off = u32::from_be_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]) as u64;
    let cfa_len = u32::from_be_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]) as u64;

    let max_end = jpeg_off
        .saturating_add(jpeg_len)
        .max(cfa_off.saturating_add(cfa_len));
    if max_end == 0 {
        None
    } else {
        Some(max_end)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;
    use crate::signature::Signature;

    fn device_from(data: Vec<u8>) -> Arc<dyn BlockDevice> {
        Arc::new(MockBlockDevice::new(data, 512))
    }

    fn dummy_sig() -> Signature {
        Signature {
            name: "Test".into(),
            extension: "bin".into(),
            header: vec![Some(0xFF)],
            footer: vec![],
            footer_last: false,
            max_size: 1024,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
        }
    }

    #[test]
    fn isobmff_two_box_file() {
        // Craft a minimal ISOBMFF byte stream:
        //   ftyp box: size=24 (bytes 0-23)
        //   moov box: size=16 (bytes 24-39)
        // Expected total = 40 bytes.
        let mut data = vec![0u8; 512];
        // ftyp box
        data[0..4].copy_from_slice(&24u32.to_be_bytes()); // size
        data[4..8].copy_from_slice(b"ftyp"); // type
                                             // moov box
        data[24..28].copy_from_slice(&16u32.to_be_bytes()); // size
        data[28..32].copy_from_slice(b"moov"); // type
                                               // byte 40 onward: zeroes — non-printable stops the walker

        let dev = device_from(data);
        let _ = dummy_sig(); // just to use the import
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff);
        assert_eq!(
            result,
            Some(40),
            "expected 40 bytes from two-box ISOBMFF file"
        );
    }

    #[test]
    fn isobmff_invalid_type_stops_walk() {
        // A box whose type contains 0x00 bytes should stop the walker.
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(&16u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        // Second box has a non-printable type byte.
        data[16..20].copy_from_slice(&16u32.to_be_bytes());
        data[20] = 0x01; // not printable ASCII
        data[21..24].copy_from_slice(b"bad");

        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff);
        // Walker should stop after the first valid box.
        assert_eq!(result, Some(16));
    }

    #[test]
    fn isobmff_size_zero_stops_walk() {
        // A box with size=0 (EOF-extending) stops the walk; accumulated total returned.
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(&16u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[16..20].copy_from_slice(&0u32.to_be_bytes()); // size=0 → EOF-extending
        data[20..24].copy_from_slice(b"mdat");

        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff);
        // Should stop at the size-0 box and return the 16 bytes accumulated so far.
        assert_eq!(result, Some(16));
    }

    #[test]
    fn isobmff_no_valid_boxes_returns_none() {
        // All-zero data has no valid ISOBMFF boxes.
        let data = vec![0u8; 512];
        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff);
        assert_eq!(result, None);
    }

    // ── TIFF IFD walker tests ──────────────────────────────────────────────────

    /// Build a minimal little-endian TIFF header + IFD0 at offset 8.
    ///
    /// IFD0 has two entries:
    ///   0x0111 StripOffsets  (LONG, count=1, value=strip_off)
    ///   0x0117 StripByteCounts (LONG, count=1, value=strip_len)
    fn make_tiff_le_single_strip(strip_off: u32, strip_len: u32) -> Vec<u8> {
        let mut data = vec![0u8; 512];
        // TIFF header
        data[0..2].copy_from_slice(b"II"); // little-endian
        data[2..4].copy_from_slice(&42u16.to_le_bytes()); // TIFF magic
        data[4..8].copy_from_slice(&8u32.to_le_bytes()); // IFD0 at offset 8

        // IFD0: 2 entries
        data[8..10].copy_from_slice(&2u16.to_le_bytes());
        // Entry 0: StripOffsets
        data[10..12].copy_from_slice(&0x0111u16.to_le_bytes()); // tag
        data[12..14].copy_from_slice(&4u16.to_le_bytes()); // type LONG
        data[14..18].copy_from_slice(&1u32.to_le_bytes()); // count
        data[18..22].copy_from_slice(&strip_off.to_le_bytes()); // value
                                                                // Entry 1: StripByteCounts
        data[22..24].copy_from_slice(&0x0117u16.to_le_bytes());
        data[24..26].copy_from_slice(&4u16.to_le_bytes());
        data[26..30].copy_from_slice(&1u32.to_le_bytes());
        data[30..34].copy_from_slice(&strip_len.to_le_bytes());
        // Next IFD offset = 0
        data[34..38].copy_from_slice(&0u32.to_le_bytes());
        data
    }

    #[test]
    fn tiff_single_strip_extent() {
        // strip at offset 100, length 1000 → file size = 1100
        let data = make_tiff_le_single_strip(100, 1000);
        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Tiff);
        assert_eq!(result, Some(1100), "single-strip TIFF extent");
    }

    #[test]
    fn tiff_subifd_extent() {
        // IFD0 → SubIFD entry → sub-IFD with large strip.
        // Layout:
        //   0..8   TIFF header (IFD0 at 8)
        //   8..26  IFD0 (1 entry: SubIFD at offset 32)
        //   32..62 Sub-IFD (2 entries: StripOffsets + StripByteCounts)
        // Strip at offset 1024, length 20000 → expected = 21024
        let mut data = vec![0u8; 4096];
        // TIFF header
        data[0..2].copy_from_slice(b"II");
        data[2..4].copy_from_slice(&42u16.to_le_bytes());
        data[4..8].copy_from_slice(&8u32.to_le_bytes()); // IFD0 at 8

        // IFD0: 1 entry (SubIFD, tag 0x014A)
        data[8..10].copy_from_slice(&1u16.to_le_bytes());
        data[10..12].copy_from_slice(&0x014Au16.to_le_bytes()); // tag
        data[12..14].copy_from_slice(&4u16.to_le_bytes()); // LONG
        data[14..18].copy_from_slice(&1u32.to_le_bytes()); // count = 1
        data[18..22].copy_from_slice(&32u32.to_le_bytes()); // sub-IFD at offset 32
        data[22..26].copy_from_slice(&0u32.to_le_bytes()); // next IFD = 0

        // Sub-IFD at offset 32: 2 entries
        data[32..34].copy_from_slice(&2u16.to_le_bytes());
        // StripOffsets
        data[34..36].copy_from_slice(&0x0111u16.to_le_bytes());
        data[36..38].copy_from_slice(&4u16.to_le_bytes());
        data[38..42].copy_from_slice(&1u32.to_le_bytes());
        data[42..46].copy_from_slice(&1024u32.to_le_bytes()); // strip at 1024
                                                              // StripByteCounts
        data[46..48].copy_from_slice(&0x0117u16.to_le_bytes());
        data[48..50].copy_from_slice(&4u16.to_le_bytes());
        data[50..54].copy_from_slice(&1u32.to_le_bytes());
        data[54..58].copy_from_slice(&20000u32.to_le_bytes()); // length 20000
        data[58..62].copy_from_slice(&0u32.to_le_bytes()); // next IFD = 0

        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Tiff);
        assert_eq!(result, Some(21024), "SubIFD strip extent");
    }

    #[test]
    fn tiff_invalid_byte_order_returns_none() {
        let mut data = vec![0u8; 64];
        data[0..2].copy_from_slice(b"XX"); // invalid byte order marker
        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Tiff);
        assert_eq!(result, None);
    }

    // ── RAF size hint tests ────────────────────────────────────────────────────

    #[test]
    fn raf_cfa_dominates() {
        // JPEG: offset=200, len=500 (end=700)
        // CFA:  offset=1000, len=5000 (end=6000) ← dominant
        let mut data = vec![0u8; 512];
        data[84..88].copy_from_slice(&200u32.to_be_bytes());
        data[88..92].copy_from_slice(&500u32.to_be_bytes());
        data[92..96].copy_from_slice(&1000u32.to_be_bytes());
        data[96..100].copy_from_slice(&5000u32.to_be_bytes());
        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Raf);
        assert_eq!(result, Some(6000));
    }

    #[test]
    fn raf_jpeg_dominates() {
        // JPEG: offset=5000, len=15000 (end=20000) ← dominant
        // CFA:  offset=100, len=1000 (end=1100)
        let mut data = vec![0u8; 512];
        data[84..88].copy_from_slice(&5000u32.to_be_bytes());
        data[88..92].copy_from_slice(&15000u32.to_be_bytes());
        data[92..96].copy_from_slice(&100u32.to_be_bytes());
        data[96..100].copy_from_slice(&1000u32.to_be_bytes());
        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Raf);
        assert_eq!(result, Some(20000));
    }

    #[test]
    fn raf_all_zero_returns_none() {
        let data = vec![0u8; 512];
        let dev = device_from(data);
        let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Raf);
        assert_eq!(result, None);
    }
}
