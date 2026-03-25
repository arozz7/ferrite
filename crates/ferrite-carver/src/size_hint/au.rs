//! Sun/NeXT AU audio file size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Derive the total file size of a Sun AU audio file.
///
/// The AU header layout (all fields big-endian):
/// ```text
/// offset  0: magic       — ".snd" (4 bytes)
/// offset  4: data_offset — u32 BE; byte offset to the audio data from file start
/// offset  8: data_size   — u32 BE; byte length of the audio data
/// ```
///
/// `total_size = data_offset + data_size`
///
/// Returns `None` when:
/// - The header cannot be read.
/// - `data_size == 0xFFFF_FFFF` (streaming / unknown length).
/// - The computed total would be zero or unreasonably small.
pub(super) fn au_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // Read 12 bytes: magic(4) + data_offset(4) + data_size(4).
    let data = read_bytes_clamped(device, file_offset, 12).ok()?;
    if data.len() < 12 {
        return None;
    }

    let data_offset = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as u64;
    let data_size = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

    // 0xFFFF_FFFF means streaming / length unknown — fall back to max_size.
    if data_size == 0xFFFF_FFFF {
        return None;
    }

    let total = data_offset.checked_add(data_size as u64)?;
    // Sanity: total must be at least as large as the minimum AU header (24 bytes).
    if total < 24 {
        return None;
    }

    Some(total)
}
