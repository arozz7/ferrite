//! GIF block-walking size-hint handler.
//!
//! Walks the GIF block structure to find the true end of the file (the
//! `0x3B` trailer byte), rather than scanning raw bytes for the footer
//! pattern `00 3B` — which easily false-matches inside LZW compressed
//! image data and causes premature truncation.
//!
//! GIF structure:
//! ```text
//! Header(6) + LSD(7) + [GCT] + blocks* + Trailer(0x3B)
//! ```
//!
//! Blocks are either:
//! - Extension: `0x21 <type> <sub-blocks> 0x00`
//! - Image:     `0x2C <descriptor(9)> [LCT] <lzw_min(1)> <sub-blocks> 0x00`
//!
//! Sub-blocks: repeated `[len: u8] [len bytes]` until `len == 0`.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Maximum number of top-level blocks before giving up.
const MAX_BLOCKS: usize = 10_000;

/// Maximum cumulative sub-block bytes before giving up (safety cap).
const MAX_SUB_BLOCK_TOTAL: u64 = 50 * 1024 * 1024; // 50 MiB

/// Derive the file size of a GIF by walking its block structure.
///
/// Returns the byte offset of the end of the trailer (total file size),
/// or `None` if the structure is malformed.
pub(super) fn gif_hint(device: &dyn BlockDevice, file_offset: u64, max_size: u64) -> Option<u64> {
    let device_size = device.size();
    let max_end = file_offset.saturating_add(max_size).min(device_size);

    // ── Header (6 bytes) + Logical Screen Descriptor (7 bytes) ──────────
    let hdr = read_bytes_clamped(device, file_offset, 13).ok()?;
    if hdr.len() < 13 {
        return None;
    }

    // Verify GIF magic.
    if &hdr[0..4] != b"GIF8" {
        return None;
    }

    // Parse Global Color Table flag and size from the packed byte.
    let packed = hdr[10];
    let has_gct = (packed >> 7) & 1 == 1;
    let gct_size = if has_gct {
        3 * (1u64 << ((packed & 0x07) + 1))
    } else {
        0
    };

    let mut pos = file_offset + 13 + gct_size;

    // ── Walk blocks ─────────────────────────────────────────────────────
    for _ in 0..MAX_BLOCKS {
        if pos >= max_end {
            return None;
        }

        let block_type = read_byte(device, pos)?;
        match block_type {
            // Trailer — end of GIF.
            0x3B => return Some(pos - file_offset + 1),

            // Extension block: 0x21 <ext_type> <sub-blocks> 0x00
            0x21 => {
                // Skip extension type byte.
                pos += 2;
                pos = skip_sub_blocks(device, pos, max_end)?;
            }

            // Image descriptor: 0x2C + 9 bytes + [LCT] + LZW min + sub-blocks + 0x00
            0x2C => {
                let desc = read_bytes_clamped(device, pos + 1, 9).ok()?;
                if desc.len() < 9 {
                    return None;
                }
                let img_packed = desc[8];
                let has_lct = (img_packed >> 7) & 1 == 1;
                let lct_size = if has_lct {
                    3 * (1u64 << ((img_packed & 0x07) + 1))
                } else {
                    0
                };

                // Skip: image descriptor (10) + LCT + LZW min code size (1).
                pos += 10 + lct_size + 1;
                pos = skip_sub_blocks(device, pos, max_end)?;
            }

            // Invalid block type.
            _ => return None,
        }
    }

    // Exceeded block safety cap.
    None
}

/// Read a single byte from the device.
fn read_byte(device: &dyn BlockDevice, offset: u64) -> Option<u8> {
    let buf = read_bytes_clamped(device, offset, 1).ok()?;
    buf.first().copied()
}

/// Walk past a sequence of GIF sub-blocks (length-prefixed, terminated by
/// a zero-length sub-block).  Returns the offset after the terminator.
fn skip_sub_blocks(device: &dyn BlockDevice, mut pos: u64, max_end: u64) -> Option<u64> {
    let mut total: u64 = 0;
    loop {
        if pos >= max_end {
            return None;
        }
        let len = read_byte(device, pos)? as u64;
        pos += 1;
        if len == 0 {
            return Some(pos); // block terminator
        }
        total += len;
        if total > MAX_SUB_BLOCK_TOTAL {
            return None;
        }
        pos += len;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal valid GIF89a with a single 1×1 image.
    fn make_gif(extra_lzw: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();

        // Header.
        buf.extend_from_slice(b"GIF89a");

        // Logical Screen Descriptor: 1×1, no GCT.
        buf.extend_from_slice(&[
            0x01, 0x00, // width = 1
            0x01, 0x00, // height = 1
            0x00, // packed: no GCT
            0x00, // bg color
            0x00, // pixel aspect ratio
        ]);

        // Image descriptor: 0x2C, left=0, top=0, w=1, h=1, packed=0.
        buf.push(0x2C);
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);

        // LZW minimum code size.
        buf.push(0x02);

        // Sub-block: 2 bytes of LZW data.
        buf.push(0x02); // sub-block length
        buf.push(0x4C); // LZW data byte 1
        buf.push(0x01); // LZW data byte 2

        // Extra sub-block data (for testing false footer matches).
        if !extra_lzw.is_empty() {
            let len = extra_lzw.len().min(255) as u8;
            buf.push(len);
            buf.extend_from_slice(&extra_lzw[..len as usize]);
        }

        // Sub-block terminator.
        buf.push(0x00);

        // Trailer.
        buf.push(0x3B);

        buf
    }

    #[test]
    fn gif_hint_minimal() {
        let data = make_gif(&[]);
        let expected = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(gif_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn gif_hint_with_offset() {
        let mut data = vec![0xFF; 200]; // junk prefix
        let gif = make_gif(&[]);
        let expected = gif.len() as u64;
        data.extend_from_slice(&gif);
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(gif_hint(dev.as_ref(), 200, 10_485_760), Some(expected));
    }

    #[test]
    fn gif_hint_false_footer_in_lzw() {
        // LZW data contains `00 3B` — should NOT cause premature stop.
        let data = make_gif(&[0x00, 0x3B, 0xAA, 0xBB]);
        let expected = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(gif_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn gif_hint_with_gce_extension() {
        let mut buf = Vec::new();

        // Header + LSD (no GCT).
        buf.extend_from_slice(b"GIF89a");
        buf.extend_from_slice(&[0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);

        // Graphic Control Extension.
        buf.push(0x21); // extension introducer
        buf.push(0xF9); // GCE label
        buf.push(0x04); // block size
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // 4 bytes of GCE data
        buf.push(0x00); // block terminator

        // Image descriptor.
        buf.push(0x2C);
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        buf.push(0x02); // LZW min code
        buf.push(0x02); // sub-block len
        buf.push(0x4C);
        buf.push(0x01);
        buf.push(0x00); // terminator
        buf.push(0x3B); // trailer

        let expected = buf.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(buf, 512));
        assert_eq!(gif_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn gif_hint_with_gct() {
        let mut buf = Vec::new();

        // Header.
        buf.extend_from_slice(b"GIF89a");

        // LSD: has GCT with 4 colors (size field = 1 → 2^(1+1) = 4).
        buf.extend_from_slice(&[
            0x01, 0x00, 0x01, 0x00,
            0x81, // GCT flag=1, size=1 (4 colors)
            0x00, 0x00,
        ]);

        // GCT: 4 × 3 = 12 bytes.
        buf.extend_from_slice(&[0u8; 12]);

        // Image + LZW + trailer.
        buf.push(0x2C);
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        buf.push(0x02);
        buf.push(0x02);
        buf.push(0x4C);
        buf.push(0x01);
        buf.push(0x00);
        buf.push(0x3B);

        let expected = buf.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(buf, 512));
        assert_eq!(gif_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn gif_hint_truncated() {
        // GIF with no trailer — should return None.
        let mut data = make_gif(&[]);
        data.pop(); // remove trailer 0x3B
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(gif_hint(dev.as_ref(), 0, 10_485_760), None);
    }
}
