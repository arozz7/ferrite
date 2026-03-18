//! TrueType / OpenType font size-hint handler.
//!
//! Reads the font table directory and returns the maximum extent
//! (`table_offset + table_length`) across all tables, which is the
//! minimum file size needed to contain the complete font.

use ferrite_blockdev::BlockDevice;

use super::helpers::read_u32_be;
use crate::carver_io::read_bytes_clamped;

/// Derive the file size of a TrueType/OpenType font from its table directory.
///
/// Font header layout (12 bytes):
///   - `sfVersion`:     4 bytes (0x00010000 for TrueType)
///   - `numTables`:     u16 BE @4
///   - `searchRange`:   u16 BE @6
///   - `entrySelector`: u16 BE @8
///   - `rangeShift`:    u16 BE @10
///
/// Each table record (16 bytes):
///   - `tag`:      4 bytes
///   - `checksum`: u32 BE
///   - `offset`:   u32 BE @8
///   - `length`:   u32 BE @12
///
/// Returns `max(offset + length)` across all table records.
pub(super) fn ttf_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // Read header (12 bytes).
    let hdr = read_bytes_clamped(device, file_offset, 12).ok()?;
    if hdr.len() < 12 {
        return None;
    }

    let num_tables = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
    if !(1..=100).contains(&num_tables) {
        return None;
    }

    // Read entire table directory: numTables * 16 bytes starting at offset 12.
    let dir_size = num_tables * 16;
    let dir = read_bytes_clamped(device, file_offset + 12, dir_size).ok()?;
    if dir.len() < dir_size {
        return None;
    }

    let mut max_extent: u64 = 0;
    for i in 0..num_tables {
        let base = i * 16;
        let offset = read_u32_be(&dir[base + 8..base + 12]) as u64;
        let length = read_u32_be(&dir[base + 12..base + 16]) as u64;
        let extent = offset.saturating_add(length);
        if extent > max_extent {
            max_extent = extent;
        }
    }

    if max_extent > 0 {
        Some(max_extent)
    } else {
        None
    }
}
