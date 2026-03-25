//! Standard MIDI File (SMF) chunk-walking size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Derive the total file size of a Standard MIDI File.
///
/// SMF layout:
/// ```text
/// MThd header chunk (14 bytes total):
///   offset  0: "MThd"       — 4-byte ASCII magic
///   offset  4: header_len   — u32 BE; always 6
///   offset  8: format       — u16 BE; 0, 1, or 2
///   offset 10: n_tracks     — u16 BE; number of MTrk chunks that follow
///   offset 12: division     — u16 BE; timing resolution
///
/// Each MTrk chunk:
///   offset  0: "MTrk"       — 4-byte ASCII magic
///   offset  4: track_len    — u32 BE; byte length of the track data
///   offset  8..             — track event data (track_len bytes)
/// ```
///
/// Total file size = 14 + sum_over_tracks(8 + track_len)
///
/// Returns `None` when:
/// - The MThd header cannot be read or is malformed.
/// - A track chunk read fails before all `n_tracks` are consumed.
/// - The computed total overflows u64.
pub(super) fn midi_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // Read the 14-byte MThd header.
    let hdr = read_bytes_clamped(device, file_offset, 14).ok()?;
    if hdr.len() < 14 {
        return None;
    }

    // Verify MThd magic.
    if &hdr[0..4] != b"MThd" {
        return None;
    }

    // header_len must be 6.
    let header_len = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    if header_len != 6 {
        return None;
    }

    let n_tracks = u16::from_be_bytes([hdr[10], hdr[11]]) as u64;

    // Start of first MTrk chunk: 8 (chunk header) + 6 (header data) = 14.
    let mut total: u64 = 14;
    let mut pos = file_offset + 14;

    for _ in 0..n_tracks {
        // Read MTrk chunk header (8 bytes: "MTrk" + u32 BE track_len).
        let chunk_hdr = read_bytes_clamped(device, pos, 8).ok()?;
        if chunk_hdr.len() < 8 {
            return None;
        }
        if &chunk_hdr[0..4] != b"MTrk" {
            return None;
        }
        let track_len =
            u32::from_be_bytes([chunk_hdr[4], chunk_hdr[5], chunk_hdr[6], chunk_hdr[7]]) as u64;

        let chunk_total = 8u64.checked_add(track_len)?;
        total = total.checked_add(chunk_total)?;
        pos = pos.checked_add(chunk_total)?;
    }

    // Sanity: a valid MIDI file must be at least the MThd + one MTrk header.
    if total < 14 {
        return None;
    }

    Some(total)
}
