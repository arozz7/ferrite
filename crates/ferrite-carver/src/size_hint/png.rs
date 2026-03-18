//! PNG chunk-walking size-hint handler.
//!
//! Walks the PNG chunk structure to find the true end of the file (end of
//! the IEND chunk), rather than scanning raw bytes for the IEND footer
//! pattern — which can false-match inside compressed IDAT data and cause
//! premature truncation.
//!
//! PNG chunk layout:
//! ```text
//! [4-byte length BE][4-byte type][length bytes data][4-byte CRC]
//! ```
//! Total chunk size = `length + 12`.  The IEND chunk has `length = 0`.

use ferrite_blockdev::BlockDevice;

use super::helpers::read_u32_be;
use crate::carver_io::read_bytes_clamped;

/// Maximum number of chunks to walk before giving up (safety cap).
const MAX_CHUNKS: usize = 10_000;

/// Derive the file size of a PNG by walking its chunk structure.
///
/// Returns the byte offset of the end of the IEND chunk (i.e. the total
/// file size), or `None` if the chunk structure is malformed.
pub(super) fn png_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // PNG signature is 8 bytes; first chunk starts at offset 8.
    let mut pos: u64 = 8;

    for _ in 0..MAX_CHUNKS {
        // Read 8 bytes: 4-byte chunk length (BE) + 4-byte chunk type.
        let hdr = read_bytes_clamped(device, file_offset + pos, 8).ok()?;
        if hdr.len() < 8 {
            return None;
        }

        let chunk_data_len = read_u32_be(&hdr[0..4]) as u64;
        let chunk_type = &hdr[4..8];

        // Each chunk is: length(4) + type(4) + data(chunk_data_len) + CRC(4)
        let chunk_total = 12 + chunk_data_len;

        // Sanity: reject absurdly large chunks (> 256 MiB).
        if chunk_data_len > 256 * 1024 * 1024 {
            return None;
        }

        // If this is the IEND chunk, return the position after it.
        if chunk_type == b"IEND" {
            return Some(pos + chunk_total);
        }

        // Validate chunk type: all four bytes must be ASCII letters (A-Z, a-z).
        if !chunk_type.iter().all(|&b| b.is_ascii_alphabetic()) {
            return None;
        }

        pos += chunk_total;
    }

    // Exceeded chunk safety cap — fall back to footer search.
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal valid PNG with IHDR + IDAT + IEND.
    fn make_png(idat_data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();

        // PNG signature (8 bytes).
        buf.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // IHDR chunk: length=13, type="IHDR", 13 bytes data, 4 bytes CRC.
        let ihdr_data: [u8; 13] = [
            0x00, 0x00, 0x00, 0x01, // width = 1
            0x00, 0x00, 0x00, 0x01, // height = 1
            0x08, // bit depth = 8
            0x02, // color type = RGB
            0x00, // compression method
            0x00, // filter method
            0x00, // interlace method
        ];
        buf.extend_from_slice(&13u32.to_be_bytes()); // length
        buf.extend_from_slice(b"IHDR");
        buf.extend_from_slice(&ihdr_data);
        buf.extend_from_slice(&[0x00; 4]); // CRC placeholder

        // IDAT chunk.
        let len = idat_data.len() as u32;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(b"IDAT");
        buf.extend_from_slice(idat_data);
        buf.extend_from_slice(&[0x00; 4]); // CRC placeholder

        // IEND chunk: length=0, type="IEND", CRC.
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(b"IEND");
        buf.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]); // standard IEND CRC

        buf
    }

    #[test]
    fn png_hint_minimal() {
        let data = make_png(&[0xDE, 0xAD]);
        let expected_size = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        let result = png_hint(dev.as_ref(), 0);
        assert_eq!(result, Some(expected_size));
    }

    #[test]
    fn png_hint_with_offset() {
        let mut data = vec![0xFF; 100]; // junk prefix
        let png = make_png(&[0x01, 0x02, 0x03]);
        let expected_size = png.len() as u64;
        data.extend_from_slice(&png);
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        let result = png_hint(dev.as_ref(), 100);
        assert_eq!(result, Some(expected_size));
    }

    #[test]
    fn png_hint_false_iend_in_idat() {
        // IDAT data that contains the IEND footer bytes — should NOT cause
        // premature termination because we walk by chunk length, not by
        // scanning for the byte pattern.
        let fake_iend = [0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];
        let data = make_png(&fake_iend);
        let expected_size = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        let result = png_hint(dev.as_ref(), 0);
        assert_eq!(result, Some(expected_size));
    }

    #[test]
    fn png_hint_truncated_header() {
        // Only the PNG signature, no chunks.
        let data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(png_hint(dev.as_ref(), 0), None);
    }

    #[test]
    fn png_hint_corrupt_chunk_type() {
        let mut data = make_png(&[0x01]);
        // Corrupt the IDAT chunk type (byte 33 = 'I' in "IDAT").
        // IHDR starts at 8: 4 len + 4 type + 13 data + 4 CRC = 25 bytes.
        // IDAT starts at 8 + 25 = 33: 4 len + then type at 37.
        data[37] = 0x00; // non-ASCII-alpha → invalid chunk type
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(png_hint(dev.as_ref(), 0), None);
    }
}
