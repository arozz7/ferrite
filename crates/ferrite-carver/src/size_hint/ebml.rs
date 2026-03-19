//! EBML/MKV/WebM size-hint handler — reads Segment element size.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Read an EBML Variable-Size Integer (VINT) from `data` at `offset`.
///
/// Leading byte: count leading zero bits + 1 = byte width.
/// Mask off the marker bit, read remaining bytes big-endian.
///
/// Returns `(value, bytes_consumed)` or `None` on invalid/truncated input.
/// The value has the VINT marker bit masked off (decoded value).
pub(super) fn read_ebml_vint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    if offset >= data.len() {
        return None;
    }
    let first = data[offset];
    if first == 0 {
        return None; // invalid: no marker bit
    }

    let width = first.leading_zeros() as usize + 1;
    if width > 8 || offset + width > data.len() {
        return None;
    }

    let mut value = (first as u64) & !(1u64 << (8 - width));
    for i in 1..width {
        value = (value << 8) | data[offset + i] as u64;
    }

    Some((value, width))
}

/// Check if a VINT value is "unknown size" (all data bits are 1).
///
/// For an N-byte VINT, unknown = (2^(7*N)) - 1.
fn is_unknown_size(value: u64, width: usize) -> bool {
    let max = (1u64 << (7 * width)) - 1;
    value == max
}

/// Derive the true file size of an MKV/WebM file from the EBML Segment element.
///
/// Algorithm:
/// 1. Read EBML header element ID (`0x1A45DFA3`) + VINT size → skip header
/// 2. Read Segment element ID (`0x18538067`) + VINT size
/// 3. If size is "unknown" (all-ones after masking), return None
/// 4. File size = current_offset + segment_size
pub(super) fn ebml_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // Read enough bytes for EBML header + Segment header (generous upper bound).
    let data = read_bytes_clamped(device, file_offset, 64).ok()?;
    if data.len() < 12 {
        return None;
    }

    // EBML header element ID: 0x1A45DFA3 (4 bytes).
    if data[0..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        return None;
    }

    // EBML header size (VINT).
    let (header_size, vint_len) = read_ebml_vint(&data, 4)?;
    let segment_start = 4 + vint_len + header_size as usize;

    // Read from the Segment element.
    if segment_start + 12 > data.len() {
        // Need more data; re-read from the expected position.
        let seg_data = read_bytes_clamped(device, file_offset + segment_start as u64, 12).ok()?;
        if seg_data.len() < 12 {
            return None;
        }
        return parse_segment(&seg_data, 0, file_offset, segment_start);
    }

    parse_segment(&data, segment_start, file_offset, segment_start)
}

fn parse_segment(
    data: &[u8],
    data_offset: usize,
    file_offset: u64,
    segment_start: usize,
) -> Option<u64> {
    // Segment element ID: 0x18538067 (4 bytes).
    if data_offset + 4 > data.len() {
        return None;
    }
    if data[data_offset..data_offset + 4] != [0x18, 0x53, 0x80, 0x67] {
        return None;
    }

    // Segment size (VINT).
    let (seg_size, vint_len) = read_ebml_vint(data, data_offset + 4)?;
    if is_unknown_size(seg_size, vint_len) {
        return None;
    }

    let total = segment_start as u64 + 4 + vint_len as u64 + seg_size;
    // Sanity: don't return absurdly small sizes.
    if total < 12 {
        return None;
    }

    // The total is relative to the start of the EBML header in the file.
    let _ = file_offset; // file_offset is the base; total is relative to it.
    Some(total)
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
    fn ebml_vint_one_byte() {
        // 0x82 = 1000_0010 → width=1, value = 0x02
        assert_eq!(read_ebml_vint(&[0x82], 0), Some((0x02, 1)));
    }

    #[test]
    fn ebml_vint_two_bytes() {
        // 0x40 0x10 → width=2, value = 0x0010 = 16
        assert_eq!(read_ebml_vint(&[0x40, 0x10], 0), Some((16, 2)));
    }

    #[test]
    fn ebml_vint_unknown_detection() {
        // 1-byte unknown: 0xFF → value = 0x7F, width=1
        assert!(is_unknown_size(0x7F, 1));
        // 2-byte unknown: 0x7FFF → value = 0x3FFF, width=2
        assert!(is_unknown_size(0x3FFF, 2));
        // Not unknown
        assert!(!is_unknown_size(100, 1));
    }

    #[test]
    fn ebml_mkv_basic() {
        // Minimal EBML: header ID (4) + header VINT size (1, value=2) + header body (2 bytes)
        //             + Segment ID (4) + Segment VINT size (1, value=1000)
        // total = 4+1+2 + 4+1+1000 = 7 + 1005 = 1012
        // But Segment VINT can't encode 1000 in 1 byte. Let's use 2-byte VINT.
        let mut data = vec![0u8; 2048];
        // EBML header
        data[0..4].copy_from_slice(&[0x1A, 0x45, 0xDF, 0xA3]);
        // Header size: VINT 1 byte = 0x82 (value 2)
        data[4] = 0x82;
        // Header body: 2 dummy bytes
        data[5] = 0x00;
        data[6] = 0x00;

        // Segment element at offset 7
        data[7..11].copy_from_slice(&[0x18, 0x53, 0x80, 0x67]);
        // Segment size: VINT 2 bytes = 0x43 0xE8 (value = 0x03E8 = 1000)
        data[11] = 0x43;
        data[12] = 0xE8;

        let dev = device_from(data);
        let result = ebml_hint(dev.as_ref(), 0);
        // total = 7 (header) + 4 (seg ID) + 2 (seg VINT) + 1000 (seg data) = 1013
        assert_eq!(result, Some(1013));
    }

    #[test]
    fn ebml_unknown_size_returns_none() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(&[0x1A, 0x45, 0xDF, 0xA3]);
        data[4] = 0x82; // header size = 2
                        // Segment at offset 7
        data[7..11].copy_from_slice(&[0x18, 0x53, 0x80, 0x67]);
        // VINT "unknown" for 1-byte: 0xFF (value=0x7F)
        data[11] = 0xFF;

        let dev = device_from(data);
        assert_eq!(ebml_hint(dev.as_ref(), 0), None);
    }

    #[test]
    fn ebml_no_segment_returns_none() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(&[0x1A, 0x45, 0xDF, 0xA3]);
        data[4] = 0x82;
        // Wrong element at offset 7
        data[7..11].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);

        let dev = device_from(data);
        assert_eq!(ebml_hint(dev.as_ref(), 0), None);
    }
}
