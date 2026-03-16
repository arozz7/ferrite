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
                let seg_table =
                    read_bytes_clamped(device, pos + 27, num_segments as usize).ok()?;
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
            pre_validate_zip: false,
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
        assert_eq!(result, Some(40), "expected 40 bytes from two-box ISOBMFF file");
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
}
