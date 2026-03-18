//! 7-Zip archive size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

pub(super) fn seven_zip_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let off_bytes = read_bytes_clamped(device, file_offset + 12, 8).ok()?;
    let off_arr: [u8; 8] = off_bytes[..8].try_into().ok()?;
    let next_offset = u64::from_le_bytes(off_arr);

    let sz_bytes = read_bytes_clamped(device, file_offset + 20, 8).ok()?;
    let sz_arr: [u8; 8] = sz_bytes[..8].try_into().ok()?;
    let next_size = u64::from_le_bytes(sz_arr);

    Some(32u64.saturating_add(next_offset).saturating_add(next_size))
}
