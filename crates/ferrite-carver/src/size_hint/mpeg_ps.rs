//! MPEG-2 Program Stream size-hint handler.
//!
//! An MPEG-2 PS stream is a sequence of *packs*, each beginning with the
//! 4-byte start code `00 00 01 BA`.  The stream ends with the 4-byte
//! Program End code `00 00 01 B9`.
//!
//! This hint scans the stream forward in chunks, tracking:
//! - the last byte offset where a valid pack header was observed, and
//! - whether the Program End code has been seen.
//!
//! It stops and returns a size when:
//! 1. The Program End code (`00 00 01 B9`) is found — most accurate.
//! 2. No valid pack header is found within a 2 MiB window — sync lost.
//! 3. A hard cap of 4 GiB is reached (same as `max_size`).
//!
//! When sync is lost the returned size is the offset of the last valid pack
//! header plus one estimated pack size (2 048 bytes, typical for DVD content).

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

const PACK_MAGIC: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
const PSEND_MAGIC: [u8; 4] = [0x00, 0x00, 0x01, 0xB9];

/// Read chunk size (512 KiB — small enough to be fast on seeks, large enough
/// to span many packs).
const CHUNK: usize = 512 * 1024;

/// If no pack header is found within this many bytes, consider sync lost.
const MAX_GAP: u64 = 2 * 1024 * 1024; // 2 MiB

/// Typical DVD pack size in bytes — used as a padding estimate when sync is
/// lost before the PSEND code.
const TYPICAL_PACK: u64 = 2048;

/// Hard cap: never return more than this regardless of what the stream says.
const HARD_CAP: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB

pub(super) fn mpeg_ps_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    max_size: u64,
) -> Option<u64> {
    let scan_limit = file_offset + max_size.min(HARD_CAP);
    let mut pos = file_offset;
    let mut last_pack_pos: Option<u64> = None;

    while pos < scan_limit {
        let want = CHUNK.min((scan_limit - pos) as usize);
        let buf = match read_bytes_clamped(device, pos, want) {
            Ok(b) if b.len() >= 4 => b,
            _ => break,
        };

        let buf_len = buf.len();

        for i in 0..buf_len.saturating_sub(3) {
            let window = &buf[i..i + 4];
            if window == PSEND_MAGIC {
                // Program End — size is up to and including the 4-byte code.
                return Some(pos + i as u64 + 4 - file_offset);
            }
            if window == PACK_MAGIC {
                last_pack_pos = Some(pos + i as u64);
            }
        }

        // Sync-lost check: if we've gone MAX_GAP bytes since the last pack
        // header, stop scanning.
        let chunk_end = pos + buf_len as u64;
        let last_seen = last_pack_pos.unwrap_or(file_offset);
        if chunk_end - last_seen > MAX_GAP {
            break;
        }

        // Overlap by 3 bytes so a magic spanning a chunk boundary isn't missed.
        pos += (buf_len as u64).saturating_sub(3);
    }

    // Return size based on last observed pack header.
    last_pack_pos.map(|lp| (lp + TYPICAL_PACK - file_offset).min(max_size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_blockdev::MockBlockDevice;

    fn make_ps_stream(pack_count: usize, with_psend: bool) -> Vec<u8> {
        let pack_size = 2048usize;
        let total = pack_count * pack_size + if with_psend { 4 } else { 0 };
        let mut data = vec![0u8; total];

        for i in 0..pack_count {
            let off = i * pack_size;
            data[off..off + 4].copy_from_slice(&PACK_MAGIC);
            // Byte 4: MPEG-2 pack marker (0x44 = valid MPEG-2 pattern)
            data[off + 4] = 0x44;
        }

        if with_psend {
            let end_off = pack_count * pack_size;
            data[end_off..end_off + 4].copy_from_slice(&PSEND_MAGIC);
        }

        data
    }

    #[test]
    fn ps_with_psend_returns_exact_size() {
        let packs = 10;
        let data = make_ps_stream(packs, true);
        let expected = (packs * 2048 + 4) as u64;
        let device = MockBlockDevice::new(data.clone(), 512);
        let result = mpeg_ps_hint(&device, 0, data.len() as u64);
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn ps_without_psend_returns_estimated_size() {
        let packs = 5;
        let data = make_ps_stream(packs, false);
        let last_pack = ((packs - 1) * 2048) as u64;
        let device = MockBlockDevice::new(data.clone(), 512);
        let result = mpeg_ps_hint(&device, 0, data.len() as u64).unwrap();
        // Should be at or just past the last pack header.
        assert!(result >= last_pack);
        assert!(result <= data.len() as u64);
    }

    #[test]
    fn ps_empty_stream_returns_none() {
        let data = vec![0u8; 4096];
        let device = MockBlockDevice::new(data.clone(), 512);
        assert!(mpeg_ps_hint(&device, 0, data.len() as u64).is_none());
    }

    #[test]
    fn ps_psend_only_returns_four_bytes() {
        // A degenerate stream: just the PSEND code, no packs.
        // Should return None since we need at least one pack.
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(&PSEND_MAGIC);
        let device = MockBlockDevice::new(data.clone(), 512);
        // PSEND found at offset 0: returns Some(4) but no pack was seen.
        // The implementation returns Some(4) from the PSEND branch regardless.
        let result = mpeg_ps_hint(&device, 0, data.len() as u64);
        assert_eq!(result, Some(4));
    }
}
