//! RAR archive size-hint handler — block walker for RAR4 and RAR5.

use ferrite_blockdev::BlockDevice;

use super::helpers::{read_u16_le, read_u32_le};
use crate::carver_io::read_bytes_clamped;

/// Safety cap: maximum number of blocks to walk before giving up.
const MAX_BLOCKS: u32 = 100_000;

/// Derive the true file size of a RAR archive by walking its block structure.
///
/// Detects RAR4 (signature `Rar!\x1A\x07\x00`) and RAR5 (`Rar!\x1A\x07\x01\x00`)
/// and dispatches to the appropriate walker.
pub(super) fn rar_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // Read enough to distinguish RAR4 from RAR5.
    let sig = read_bytes_clamped(device, file_offset, 8).ok()?;
    if sig.len() < 7 {
        return None;
    }
    // Both start with Rar!\x1A\x07; byte 6 distinguishes version.
    if &sig[..6] != b"Rar!\x1A\x07" {
        return None;
    }
    if sig[6] == 0x00 {
        rar4_walk(device, file_offset)
    } else if sig.len() >= 8 && sig[6] == 0x01 && sig[7] == 0x00 {
        rar5_walk(device, file_offset)
    } else {
        None
    }
}

/// RAR4 block walker.
///
/// Block layout:
/// - HEAD_CRC: 2 bytes
/// - HEAD_TYPE: 1 byte
/// - HEAD_FLAGS: 2 bytes (u16 LE)
/// - HEAD_SIZE: 2 bytes (u16 LE) — total size of this header (including these 7 bytes)
/// - If HEAD_FLAGS bit 15 is set: ADD_SIZE (u32 LE) follows — data payload size.
///
/// Block total = HEAD_SIZE + (ADD_SIZE if bit 15).
/// Stop at HEAD_TYPE == 0x7B (end-of-archive) or invalid block.
fn rar4_walk(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let device_size = device.size();
    // RAR4 signature is 7 bytes.
    let mut pos = file_offset + 7;

    for _ in 0..MAX_BLOCKS {
        if pos + 7 > device_size {
            break;
        }
        let hdr = read_bytes_clamped(device, pos, 7).ok()?;
        if hdr.len() < 7 {
            break;
        }
        let head_type = hdr[2];
        let head_flags = read_u16_le(&hdr[3..5]);
        let head_size = read_u16_le(&hdr[5..7]) as u64;

        if head_size < 7 {
            break; // corrupt block
        }

        let add_size: u64 = if head_flags & 0x8000 != 0 {
            // ADD_SIZE follows immediately after the 7-byte fixed header.
            if pos + 7 + 4 > device_size {
                break;
            }
            let add_bytes = read_bytes_clamped(device, pos + 7, 4).ok()?;
            if add_bytes.len() < 4 {
                break;
            }
            read_u32_le(&add_bytes) as u64
        } else {
            0
        };

        let block_total = head_size.saturating_add(add_size);
        pos = pos.saturating_add(block_total);

        // End-of-archive marker.
        if head_type == 0x7B {
            return Some(pos - file_offset);
        }
    }

    // If we walked past the signature, return accumulated size even without
    // an end-of-archive marker (best effort for truncated archives).
    let size = pos - file_offset;
    if size > 7 {
        Some(size)
    } else {
        None
    }
}

/// Read a RAR5 variable-length integer (vint).
///
/// Each byte: lower 7 bits are data, bit 7 is continuation flag.
/// Returns `(value, bytes_consumed)` or `None` on overflow/malformed input.
fn read_rar5_vint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for i in 0..10 {
        // vint is at most 10 bytes for u64
        let pos = offset + i;
        if pos >= data.len() {
            return None;
        }
        let b = data[pos];
        let val = (b & 0x7F) as u64;
        result |= val.checked_shl(shift)?;
        shift += 7;
        if b & 0x80 == 0 {
            return Some((result, i + 1));
        }
    }
    None
}

/// RAR5 block walker.
///
/// Block layout:
/// - Header CRC32: 4 bytes
/// - Header size: vint (size of header after this field)
/// - Header type: vint
/// - Header flags: vint
/// - If flags bit 1: Extra area size (vint)
/// - If flags bit 2: Data size (vint) — payload after header
///
/// Stop at Header type == 5 (end-of-archive) or invalid block.
fn rar5_walk(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let device_size = device.size();
    // RAR5 signature is 8 bytes.
    let mut pos = file_offset + 8;

    for _ in 0..MAX_BLOCKS {
        if pos + 4 >= device_size {
            break;
        }
        // Read a generous chunk for vint parsing.
        let max_read = ((device_size - pos) as usize).min(1024);
        let data = read_bytes_clamped(device, pos, max_read).ok()?;
        if data.len() < 11 {
            break; // too small for any valid header
        }

        // Skip CRC32 (4 bytes).
        let mut off = 4;

        // Header size (vint).
        let (header_size, n) = read_rar5_vint(&data, off)?;
        off += n;
        let header_end = 4 + n + header_size as usize; // total header bytes from block start

        // Header type (vint).
        let (header_type, n) = read_rar5_vint(&data, off)?;
        off += n;

        // Header flags (vint).
        let (header_flags, n) = read_rar5_vint(&data, off)?;
        off += n;

        // Extra area size (if flags bit 0).
        if header_flags & 0x01 != 0 {
            let (_extra_size, n) = read_rar5_vint(&data, off)?;
            off += n;
        }
        let _ = off; // suppress unused warning

        // Data size (if flags bit 1).
        let data_size: u64 = if header_flags & 0x02 != 0 {
            // Data size vint is at the end of the header area — re-parse from
            // the correct offset. For simplicity, skip the extra area size too
            // and just read the last vint before the end of header.
            // Actually, let's parse properly.
            let (ds, _) = read_rar5_vint(&data, off)?;
            ds
        } else {
            0
        };

        let block_total = (header_end as u64).saturating_add(data_size);
        pos = pos.saturating_add(block_total);

        // End-of-archive marker (type 5).
        if header_type == 5 {
            return Some(pos - file_offset);
        }
    }

    let size = pos - file_offset;
    if size > 8 {
        Some(size)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_blockdev::MockBlockDevice;
    use std::sync::Arc;

    fn device_from(data: Vec<u8>) -> Arc<dyn ferrite_blockdev::BlockDevice> {
        Arc::new(MockBlockDevice::new(data, 512))
    }

    #[test]
    fn rar4_end_of_archive() {
        // RAR4 signature (7 bytes) + one end-of-archive block.
        let mut data = vec![0u8; 512];
        // RAR4 magic
        data[0..7].copy_from_slice(b"Rar!\x1A\x07\x00");
        // End-of-archive block at offset 7:
        // CRC(2) + TYPE(1) + FLAGS(2) + SIZE(2) = 7 bytes
        // HEAD_TYPE = 0x7B, HEAD_FLAGS = 0x4000 (fixed per spec), HEAD_SIZE = 7
        data[9] = 0x7B; // HEAD_TYPE
        data[10..12].copy_from_slice(&0x4000u16.to_le_bytes()); // HEAD_FLAGS
        data[12..14].copy_from_slice(&7u16.to_le_bytes()); // HEAD_SIZE

        let dev = device_from(data);
        let result = rar_hint(dev.as_ref(), 0);
        // 7 (sig) + 7 (block) = 14
        assert_eq!(result, Some(14));
    }

    #[test]
    fn rar4_with_data_block() {
        let mut data = vec![0u8; 2048];
        data[0..7].copy_from_slice(b"Rar!\x1A\x07\x00");
        // Data block at offset 7:
        // TYPE = 0x74 (FILE_HEAD), FLAGS = 0x8000 (has ADD_SIZE), SIZE = 32
        data[9] = 0x74;
        data[10..12].copy_from_slice(&0x8000u16.to_le_bytes());
        data[12..14].copy_from_slice(&32u16.to_le_bytes());
        // ADD_SIZE = 500 at offset 7+7=14
        data[14..18].copy_from_slice(&500u32.to_le_bytes());

        // End block at 7 + 32 + 500 = 539:
        data[541] = 0x7B;
        data[542..544].copy_from_slice(&0x0000u16.to_le_bytes());
        data[544..546].copy_from_slice(&7u16.to_le_bytes());

        let dev = device_from(data);
        let result = rar_hint(dev.as_ref(), 0);
        // 7 (sig) + 32+500 (data block) + 7 (end block) = 546
        assert_eq!(result, Some(546));
    }

    #[test]
    fn rar5_end_of_archive() {
        // RAR5 signature (8 bytes) + end-of-archive block.
        let mut data = vec![0u8; 512];
        data[0..8].copy_from_slice(b"Rar!\x1A\x07\x01\x00");
        // End-of-archive block at offset 8:
        // CRC32(4) + header_size(vint=1 byte, value=3) + type(vint=1, value=5) + flags(vint=1, value=0)
        let off = 8;
        data[off..off + 4].copy_from_slice(&[0x00; 4]); // CRC32
        data[off + 4] = 3; // header_size = 3 (covers type+flags+nothing else)
        data[off + 5] = 5; // header_type = 5 (end of archive)
        data[off + 6] = 0; // header_flags = 0

        let dev = device_from(data);
        let result = rar_hint(dev.as_ref(), 0);
        // 8 (sig) + 4(crc) + 1(hdr_size_vint) + 3(header body) = 8 + 8 = 16
        assert_eq!(result, Some(16));
    }

    #[test]
    fn rar5_vint_parsing() {
        // Single byte: 0x05 → value=5, consumed=1
        assert_eq!(read_rar5_vint(&[0x05], 0), Some((5, 1)));
        // Multi-byte: 0x80|0x01 0x01 → (1) | (1 << 7) = 129
        assert_eq!(read_rar5_vint(&[0x81, 0x01], 0), Some((129, 2)));
    }

    #[test]
    fn rar_non_rar_returns_none() {
        let data = vec![0u8; 512];
        let dev = device_from(data);
        assert_eq!(rar_hint(dev.as_ref(), 0), None);
    }
}
