use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::error::{FilesystemError, Result};

// ── Byte-parsing helpers ───────────────────────────────────────────────────────

/// Read a little-endian `u16` from `buf[offset..offset+2]`.
///
/// Returns `Err(InvalidStructure)` if the buffer is too short, allowing callers
/// to propagate parse errors instead of panicking on corrupt disk data.
pub fn read_u16_le(buf: &[u8], offset: usize) -> Result<u16> {
    buf.get(offset..offset + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| FilesystemError::InvalidStructure {
            context: "byte parse",
            reason: format!("u16 at offset {offset}: buffer is {} bytes", buf.len()),
        })
}

/// Read a little-endian `u32` from `buf[offset..offset+4]`.
pub fn read_u32_le(buf: &[u8], offset: usize) -> Result<u32> {
    buf.get(offset..offset + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| FilesystemError::InvalidStructure {
            context: "byte parse",
            reason: format!("u32 at offset {offset}: buffer is {} bytes", buf.len()),
        })
}

/// Read a little-endian `u64` from `buf[offset..offset+8]`.
pub fn read_u64_le(buf: &[u8], offset: usize) -> Result<u64> {
    buf.get(offset..offset + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or_else(|| FilesystemError::InvalidStructure {
            context: "byte parse",
            reason: format!("u64 at offset {offset}: buffer is {} bytes", buf.len()),
        })
}

/// Read exactly `len` bytes starting at byte `offset` from `device`.
///
/// Internally aligns the I/O request to sector boundaries as required by the
/// [`BlockDevice`] contract.  Returns an owned `Vec<u8>` of exactly `len`
/// bytes on success.
pub fn read_bytes(device: &dyn BlockDevice, offset: u64, len: usize) -> Result<Vec<u8>> {
    if len == 0 {
        return Ok(Vec::new());
    }

    let sector_size = device.sector_size() as u64;
    let start_sector = offset / sector_size;
    let end_sector = (offset + len as u64).div_ceil(sector_size);
    let sectors_count = (end_sector - start_sector) as usize;
    let buf_size = sectors_count * sector_size as usize;

    let mut buf = AlignedBuffer::new(buf_size, sector_size as usize);
    let bytes_read = device
        .read_at(start_sector * sector_size, &mut buf)
        .map_err(FilesystemError::BlockDevice)?;

    let start_in_buf = (offset % sector_size) as usize;
    let available = bytes_read.saturating_sub(start_in_buf);

    if available < len {
        return Err(FilesystemError::BufferTooSmall {
            needed: len,
            got: available,
        });
    }

    Ok(buf.as_slice()[start_in_buf..start_in_buf + len].to_vec())
}
