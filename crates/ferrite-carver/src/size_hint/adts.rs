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
/// the result.  Higher values dramatically reduce false positives on random data.
const MIN_FRAMES: u32 = 8;

/// Minimum ADTS frame length: 7-byte header when `protection_absent = 1`.
const MIN_FRAME_LEN: usize = 7;

/// Maximum ADTS frame length.  The 13-bit field allows up to 8191 bytes.
const MAX_FRAME_LEN: usize = 8191;

/// Safety cap on frame iterations.  A 50 MiB file at typical AAC frame sizes
/// (~500 bytes) is ~100 000 frames; 200 000 gives plenty of headroom while
/// capping walk time on random data.
const MAX_FRAMES: u32 = 200_000;

/// Walk ADTS frames from `file_offset` and return the total byte length of the
/// continuous stream, or `None` if the data does not look like a real AAC
/// ADTS stream.
///
/// Beyond sync and layer bits, we enforce two stream-level invariants that
/// must hold throughout any legitimate ADTS file:
///
/// * **Consistent `sampling_freq_index`** — the 4-bit sample-rate index
///   (b2[5:2]) encodes the fixed sample rate of the stream.  It never changes
///   between frames.  Random data will almost always vary this field.
///
/// * **Valid `sampling_freq_index`** — values 13–15 are reserved/invalid per
///   the AAC spec; a frame reporting those values is not a real ADTS frame.
pub(super) fn adts_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let device_size = device.size();
    let mut pos = file_offset;
    let mut frame_count: u32 = 0;
    let mut expected_sfi: Option<u8> = None; // sampling_freq_index of first frame

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

        // sampling_freq_index occupies bits [21:18] of the header = b2[5:2].
        // Valid values: 0–12.  Values 13–15 are reserved.
        let sfi = (hdr[2] >> 2) & 0x0F;
        if sfi > 12 {
            break;
        }
        // All frames in a stream must share the same sample rate.
        match expected_sfi {
            None => expected_sfi = Some(sfi),
            Some(e) if e != sfi => break, // sample rate changed — random data
            _ => {}
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
    fn eight_frames_produces_size() {
        let frame_len = 512;
        let dev = make_device_with_frames(8, frame_len);
        let result = adts_hint(&dev, 0);
        assert_eq!(result, Some((8 * frame_len) as u64));
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
        let dev = make_device_with_frames(7, frame_len); // MIN_FRAMES = 8
        let result = adts_hint(&dev, 0);
        assert!(
            result.is_none(),
            "7 frames should return None (below MIN_FRAMES)"
        );
    }

    #[test]
    fn invalid_sync_after_valid_frames_stops_walk() {
        // 10 valid frames followed by garbage — ≥ MIN_FRAMES so returns a size.
        let frame_len = 256;
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..10 {
            data.extend_from_slice(&make_adts_frame(frame_len));
        }
        data.extend_from_slice(&[0x00u8; 256]); // garbage — no sync
        let dev = MockBlockDevice::new(data, 512);
        let result = adts_hint(&dev, 0);
        assert_eq!(result, Some((10 * frame_len) as u64));
    }

    #[test]
    fn inconsistent_sfi_terminates_walk() {
        // First 8 frames have sfi=4 (44100 Hz); frame 9 has sfi=3 (48000 Hz).
        // The sfi change should stop the walk at 8 frames and return a size.
        let frame_len = 256;
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..8 {
            data.extend_from_slice(&make_adts_frame(frame_len));
        }
        // Frame 9: change sfi from 4 (0b0100) to 3 (0b0011) in b2[5:2].
        let mut bad_frame = make_adts_frame(frame_len);
        bad_frame[2] = (bad_frame[2] & 0xC3) | (3u8 << 2); // sfi=3
        data.extend_from_slice(&bad_frame);
        let dev = MockBlockDevice::new(data, 512);
        // Walk stops at frame 9; 8 frames were valid → returns size for 8 frames.
        assert_eq!(adts_hint(&dev, 0), Some((8 * frame_len) as u64));
    }

    #[test]
    fn invalid_sfi_in_first_frame_returns_none() {
        // sfi = 14 (reserved/invalid) in the very first frame → None.
        let frame_len = 256;
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..10 {
            let mut f = make_adts_frame(frame_len);
            f[2] = (f[2] & 0xC3) | (14u8 << 2); // sfi=14 (invalid)
            data.extend_from_slice(&f);
        }
        let dev = MockBlockDevice::new(data, 512);
        assert!(adts_hint(&dev, 0).is_none());
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
        // Put 8 valid frames at byte 100, preceded by zeros.
        let frame_len = 512;
        let prefix = vec![0u8; 100];
        let mut data = prefix.clone();
        for _ in 0..8 {
            data.extend_from_slice(&make_adts_frame(frame_len));
        }
        let dev = MockBlockDevice::new(data, 512);
        // Walk starting at offset 100; result is relative to file_offset.
        let result = adts_hint(&dev, 100);
        assert_eq!(result, Some((8 * frame_len) as u64));
    }
}
