//! JPEG segment-walking size-hint handler.
//!
//! Walks the JPEG segment structure to locate the true End-of-Image (`FF D9`)
//! marker, rather than relying on a raw byte-pattern footer scan.
//!
//! ## Why a footer scan fails for JPEG
//!
//! EXIF APP1 blocks embed a full JPEG thumbnail.  That thumbnail ends with its
//! own `FF D9` EOI, which a naive footer scan finds first — truncating the
//! extracted file to the thumbnail's few kilobytes.
//!
//! ## How this walker works
//!
//! **Phase 1 — segment walk:** Starting after the SOI marker, each JPEG
//! segment is `FF <marker> <len_hi> <len_lo> <data>` where the 2-byte length
//! field includes itself but not the 2-byte marker.  The walker skips each
//! segment (including the APP1 EXIF block that contains the thumbnail) until
//! it reaches the SOS (`FF DA`) marker.
//!
//! **Phase 2 — entropy scan:** After the SOS header, the compressed image
//! data begins.  It has no length field, but by the JPEG spec any `0xFF` byte
//! in the bitstream is escaped as `FF 00` (a "stuffed byte").  Therefore the
//! first unescaped `FF D9` we find *is* the real EOI.  Restart markers
//! (`FF D0`–`FF D7`) and stuffed bytes (`FF 00`) are skipped transparently.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Maximum number of segments to walk before giving up.
const MAX_SEGMENTS: usize = 512;

/// Chunk size for entropy-data scanning (Phase 2).
const SCAN_CHUNK: usize = 8192;

/// Derive the total file size of a JPEG by walking its segment structure.
///
/// Returns the byte count from `file_offset` to the end of the EOI marker,
/// or `None` if the structure is malformed or the device ends before EOI.
pub(super) fn jpeg_hint(device: &dyn BlockDevice, file_offset: u64, max_size: u64) -> Option<u64> {
    let max_end = file_offset.saturating_add(max_size).min(device.size());

    // Skip SOI (FF D8).
    let mut pos = file_offset + 2;

    // ── Phase 1: segment walk ────────────────────────────────────────────
    for _ in 0..MAX_SEGMENTS {
        if pos + 2 > max_end {
            return None;
        }

        let mb = read_bytes_clamped(device, pos, 2).ok()?;
        if mb.len() < 2 || mb[0] != 0xFF {
            return None; // Not a marker — corrupt stream.
        }

        let marker = mb[1];

        match marker {
            // Stand-alone markers (no length field).
            0xD8 => pos += 2,                           // SOI (unexpected but skip)
            0xD9 => return Some(pos - file_offset + 2), // EOI
            0xD0..=0xD7 => pos += 2,                    // RST0-RST7

            // SOS: skip the scan header, then switch to entropy scan.
            0xDA => {
                let lb = read_bytes_clamped(device, pos + 2, 2).ok()?;
                if lb.len() < 2 {
                    return None;
                }
                let seg_len = u16::from_be_bytes([lb[0], lb[1]]) as u64;
                pos += 2 + seg_len; // now at start of entropy-coded data
                return scan_for_eoi(device, pos, file_offset, max_end);
            }

            // Standard segment: marker (2) + length (2) + data (len-2).
            _ => {
                let lb = read_bytes_clamped(device, pos + 2, 2).ok()?;
                if lb.len() < 2 {
                    return None;
                }
                let seg_len = u16::from_be_bytes([lb[0], lb[1]]) as u64;
                if seg_len < 2 {
                    return None; // Minimum valid length is 2.
                }
                pos += 2 + seg_len;
            }
        }
    }

    None // Exceeded segment safety cap.
}

/// Phase 2: scan entropy-coded data for the real EOI (`FF D9`).
///
/// Any `FF 00` (stuffed byte) or `FF D0`–`FF D7` (restart markers) are
/// skipped.  The first `FF D9` we encounter is the true end of the image.
fn scan_for_eoi(
    device: &dyn BlockDevice,
    mut pos: u64,
    file_offset: u64,
    max_end: u64,
) -> Option<u64> {
    let mut saw_ff = false;

    while pos < max_end {
        let read_len = SCAN_CHUNK.min((max_end - pos) as usize);
        let chunk = read_bytes_clamped(device, pos, read_len).ok()?;
        if chunk.is_empty() {
            return None;
        }

        for (i, &b) in chunk.iter().enumerate() {
            if saw_ff {
                match b {
                    0x00 | 0xD0..=0xD7 => {} // Stuffed byte or RST — not a real marker.
                    0xD9 => {
                        // True EOI found.
                        let abs = pos + i as u64 + 1; // byte after 0xD9
                        return Some(abs - file_offset);
                    }
                    0xFF => {
                        // Two consecutive FFs — the second starts a new potential marker.
                        // saw_ff stays true.
                        continue;
                    }
                    _ => {} // Some other marker mid-stream (e.g. DNL, DRI) — keep scanning.
                }
            }
            saw_ff = b == 0xFF;
        }

        pos += chunk.len() as u64;
    }

    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal but structurally valid JPEG:
    ///   SOI + APP0(len) + DQT(len) + SOF0(len) + DHT(len) + SOS(len) + <scan> + EOI
    fn make_jpeg(extra_app_data: &[u8], scan_data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();

        // SOI
        buf.extend_from_slice(&[0xFF, 0xD8]);

        // APP0 (JFIF) — length = 2 + 14 = 16 (length field includes itself)
        let app0_payload: &[u8] = &[
            0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
        ]; // 14 bytes
        buf.extend_from_slice(&[0xFF, 0xE0]);
        let app0_len = (2 + app0_payload.len()) as u16;
        buf.extend_from_slice(&app0_len.to_be_bytes());
        buf.extend_from_slice(app0_payload);

        // Extra APP data (simulates an EXIF block containing a thumbnail with its own FF D9)
        if !extra_app_data.is_empty() {
            buf.extend_from_slice(&[0xFF, 0xE1]);
            let elen = (2 + extra_app_data.len()) as u16;
            buf.extend_from_slice(&elen.to_be_bytes());
            buf.extend_from_slice(extra_app_data);
        }

        // DQT (minimal, length = 2 + 65 = 67)
        buf.extend_from_slice(&[0xFF, 0xDB]);
        buf.extend_from_slice(&67u16.to_be_bytes());
        buf.extend_from_slice(&[0u8; 65]);

        // SOF0 (minimal, length = 2 + 11 = 13)
        buf.extend_from_slice(&[0xFF, 0xC0]);
        buf.extend_from_slice(&13u16.to_be_bytes());
        buf.extend_from_slice(&[
            0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0x00, 0x00,
        ]);

        // DHT (minimal, length = 2 + 17 = 19)
        buf.extend_from_slice(&[0xFF, 0xC4]);
        buf.extend_from_slice(&19u16.to_be_bytes());
        buf.extend_from_slice(&[0u8; 17]);

        // SOS header (length = 2 + 8 = 10)
        buf.extend_from_slice(&[0xFF, 0xDA]);
        buf.extend_from_slice(&10u16.to_be_bytes());
        buf.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x00, 0x00]);

        // Entropy-coded scan data
        buf.extend_from_slice(scan_data);

        // EOI
        buf.extend_from_slice(&[0xFF, 0xD9]);

        buf
    }

    #[test]
    fn jpeg_hint_minimal() {
        let data = make_jpeg(&[], &[0xAA, 0xBB, 0xCC]);
        let expected = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(jpeg_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn jpeg_hint_skips_thumbnail_eoi() {
        // APP1 block contains a fake thumbnail that ends with FF D9.
        // The walker must skip the whole APP1 block and find the real EOI at the end.
        let thumbnail_data: Vec<u8> = vec![
            0xFF, 0xD8, // fake thumbnail SOI
            0x00, 0x01, 0x02, 0x03, // some bytes
            0xFF, 0xD9, // fake thumbnail EOI — should NOT be returned
        ];
        let scan_data: &[u8] = &[0x55, 0x66, 0x77];
        let data = make_jpeg(&thumbnail_data, scan_data);
        let expected = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(jpeg_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn jpeg_hint_stuffed_ff_in_scan() {
        // FF 00 in scan data must not terminate the walk.
        let scan_data: &[u8] = &[0xFF, 0x00, 0xFF, 0x00, 0xAA, 0xBB];
        let data = make_jpeg(&[], scan_data);
        let expected = data.len() as u64;
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(jpeg_hint(dev.as_ref(), 0, 10_485_760), Some(expected));
    }

    #[test]
    fn jpeg_hint_with_file_offset() {
        let mut buf = vec![0u8; 512]; // junk prefix
        let jpeg = make_jpeg(&[], &[0xDE, 0xAD]);
        let expected = jpeg.len() as u64;
        buf.extend_from_slice(&jpeg);
        let dev = Arc::new(MockBlockDevice::new(buf, 512));
        assert_eq!(jpeg_hint(dev.as_ref(), 512, 10_485_760), Some(expected));
    }

    #[test]
    fn jpeg_hint_truncated_returns_none() {
        // JPEG without a final EOI — should return None.
        let mut data = make_jpeg(&[], &[0xAA]);
        data.truncate(data.len() - 2); // remove FF D9
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert_eq!(jpeg_hint(dev.as_ref(), 0, 10_485_760), None);
    }
}
