//! ADTS (Audio Data Transport Stream) size-hint walker for AAC audio.
//!
//! ADTS is the raw container for AAC audio.  Every frame begins with a 12-bit
//! sync word (`0xFFF`) followed by a fixed header that encodes the exact frame
//! length in a 13-bit field spanning bytes 3–5.  This allows lossless,
//! index-free frame walking.
//!
//! The walker starts at `file_offset`, reads frame headers one by one, and
//! advances by the frame's self-reported length.  It stops at the first frame
//! with an invalid sync word, an implausible frame length, or a device read
//! error.  Returns `None` when fewer than [`MIN_FRAMES`] valid frames are
//! found (likely a false positive).

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Minimum number of consecutive valid ADTS frames required before we trust
/// the result.  Raises the confidence bar against spurious 0xFFF0 patterns.
const MIN_FRAMES: u32 = 4;

/// Minimum ADTS frame length: 7-byte header when `protection_absent = 1`.
const MIN_FRAME_LEN: usize = 7;

/// Maximum ADTS frame length.  The 13-bit field allows up to 8191 bytes.
const MAX_FRAME_LEN: usize = 8191;

/// Safety cap on frame iterations (~1 GiB of minimum-size frames).
const MAX_FRAMES: u32 = 500_000;

/// Walk ADTS frames from `file_offset` and return the total byte length of the
/// continuous stream, or `None` if the data does not look like a real AAC
/// ADTS stream.
pub(super) fn adts_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let device_size = device.size();
    let mut pos = file_offset;
    let mut frame_count: u32 = 0;

    for _ in 0..MAX_FRAMES {
        if pos >= device_size {
            break;
        }
        // Read 7 bytes — the minimum ADTS frame header size.
        let hdr = read_bytes_clamped(device, pos, 7).ok()?;
        if hdr.len() < 6 {
            break;
        }

        let b0 = hdr[0];
        let b1 = hdr[1];

        // Top 12 bits must form the sync word 0xFFF.
        if b0 != 0xFF || (b1 & 0xF0) != 0xF0 {
            break;
        }
        // Layer bits (b1[2:1]) must be 00 — non-zero means MP3, not AAC.
        if b1 & 0x06 != 0x00 {
            break;
        }

        // 13-bit frame-length field at header bits [30:18]:
        //   byte3[1:0] << 11  |  byte4[7:0] << 3  |  byte5[7:5]
        let frame_len =
            ((hdr[3] & 0x03) as usize) << 11 | (hdr[4] as usize) << 3 | (hdr[5] as usize) >> 5;

        if !(MIN_FRAME_LEN..=MAX_FRAME_LEN).contains(&frame_len) {
            break;
        }

        pos = pos.saturating_add(frame_len as u64);
        frame_count += 1;
    }

    if frame_count < MIN_FRAMES {
        return None;
    }
    Some(pos - file_offset)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal ADTS frame:
    ///   byte 0: 0xFF
    ///   byte 1: 0xF1 (MPEG-4, layer=00, protection_absent=1)
    ///   byte 2: profile+SFI+private+channel (arbitrary valid)
    ///   bytes 3-5: encode `frame_len` in the 13-bit field
    ///   byte 6: buffer_fullness (0x1F = VBR)
    fn make_adts_frame(frame_len: usize) -> Vec<u8> {
        assert!((7..=8191).contains(&frame_len));
        let mut f = vec![0u8; frame_len];
        f[0] = 0xFF;
        f[1] = 0xF1; // MPEG-4, no CRC
        f[2] = 0x50; // profile=1, SFI=4 (44100Hz), channel=2
                     // Encode frame_len into bits [30:18] of bytes 3-5.
        f[3] = ((frame_len >> 11) & 0x03) as u8;
        f[4] = ((frame_len >> 3) & 0xFF) as u8;
        f[5] = (((frame_len & 0x07) << 5) | 0x1F) as u8; // low 3 bits + fullness=VBR
        f[6] = 0xFC; // number_of_raw_data_blocks=0
        f
    }

    fn make_device_with_frames(n: usize, frame_len: usize) -> MockBlockDevice {
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..n {
            data.extend_from_slice(&make_adts_frame(frame_len));
        }
        MockBlockDevice::new(data, 512)
    }

    #[test]
    fn four_frames_produces_size() {
        let frame_len = 512;
        let dev = make_device_with_frames(4, frame_len);
        let result = adts_hint(&dev, 0);
        assert_eq!(result, Some((4 * frame_len) as u64));
    }

    #[test]
    fn ten_frames_produces_correct_size() {
        let frame_len = 1024;
        let dev = make_device_with_frames(10, frame_len);
        let result = adts_hint(&dev, 0);
        assert_eq!(result, Some((10 * frame_len) as u64));
    }

    #[test]
    fn fewer_than_min_frames_returns_none() {
        let frame_len = 512;
        let dev = make_device_with_frames(3, frame_len); // MIN_FRAMES = 4
        let result = adts_hint(&dev, 0);
        assert!(
            result.is_none(),
            "3 frames should return None (below MIN_FRAMES)"
        );
    }

    #[test]
    fn invalid_sync_after_valid_frames_stops_walk() {
        // 5 valid frames followed by garbage.
        let frame_len = 256;
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..5 {
            data.extend_from_slice(&make_adts_frame(frame_len));
        }
        data.extend_from_slice(&[0x00u8; 256]); // garbage — no sync
        let dev = MockBlockDevice::new(data, 512);
        let result = adts_hint(&dev, 0);
        assert_eq!(result, Some((5 * frame_len) as u64));
    }

    #[test]
    fn zero_frame_len_in_header_returns_none() {
        // Synthesise a fake ADTS header with frame_len=0 — should return None.
        let mut bad = vec![0u8; 16];
        bad[0] = 0xFF;
        bad[1] = 0xF1;
        // frame_len field = 0 (bytes 3-5 stay 0x00) < MIN_FRAME_LEN → None
        let dev = MockBlockDevice::new(bad, 512);
        assert!(adts_hint(&dev, 0).is_none());
    }

    #[test]
    fn offset_walk_starts_at_file_offset() {
        // Put 4 valid frames at byte 100, preceded by zeros.
        let frame_len = 512;
        let prefix = vec![0u8; 100];
        let mut data = prefix.clone();
        for _ in 0..4 {
            data.extend_from_slice(&make_adts_frame(frame_len));
        }
        let dev = MockBlockDevice::new(data, 512);
        // Walk starting at offset 100; result is relative to file_offset.
        let result = adts_hint(&dev, 100);
        assert_eq!(result, Some((4 * frame_len) as u64));
    }
}
