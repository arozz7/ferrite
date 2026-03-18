//! SQLite database size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

pub(super) fn sqlite_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let ps_bytes = read_bytes_clamped(device, file_offset + 16, 2).ok()?;
    let ps_arr: [u8; 2] = ps_bytes[..2].try_into().ok()?;
    let raw_page_size = u16::from_be_bytes(ps_arr);
    let page_size: u64 = if raw_page_size == 1 {
        65536
    } else {
        raw_page_size as u64
    };
    if page_size < 512 {
        return None;
    }

    let dp_bytes = read_bytes_clamped(device, file_offset + 28, 4).ok()?;
    let dp_arr: [u8; 4] = dp_bytes[..4].try_into().ok()?;
    let db_pages = u32::from_be_bytes(dp_arr) as u64;
    if db_pages == 0 {
        return None;
    }

    Some(page_size.saturating_mul(db_pages))
}
