//! TIFF IFD chain walker and Fujifilm RAF size-hint handlers.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Walk the TIFF IFD chain and return the maximum byte extent referenced by
/// any external data block, strip/tile offset+length pair, or SubIFD pointer.
///
/// Supports both little-endian (`II`) and big-endian (`MM`) byte orders, and
/// the Panasonic RW2 variant magic (`0x55`).  SubIFDs reached via tag `0x014A`
/// are queued for traversal so raw sensor IFDs are always visited.
pub(super) fn tiff_size_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
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
                max_extent = max_extent.max(ext_off_val + data_bytes);
            }

            match tag {
                // SubIFD (tag 0x014A)
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
                        continue;
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
pub(super) fn raf_size_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
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
