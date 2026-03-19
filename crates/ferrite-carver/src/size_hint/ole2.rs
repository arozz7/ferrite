//! OLE2 Compound File Binary Format size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

pub(super) fn ole2_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let shift_bytes = read_bytes_clamped(device, file_offset + 30, 2).ok()?;
    let shift_arr: [u8; 2] = shift_bytes[..2].try_into().ok()?;
    let sector_shift = u16::from_le_bytes(shift_arr) as u32;
    if !(7..=16).contains(&sector_shift) {
        return None;
    }
    let sector_size = 1u64 << sector_shift;

    let fat_bytes = read_bytes_clamped(device, file_offset + 44, 4).ok()?;
    let fat_arr: [u8; 4] = fat_bytes[..4].try_into().ok()?;
    let csect_fat = u32::from_le_bytes(fat_arr) as u64;

    let addressable = csect_fat.saturating_mul(sector_size / 4);
    Some(addressable.saturating_add(1).saturating_mul(sector_size))
}
