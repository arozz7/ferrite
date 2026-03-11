use ferrite_blockdev::{AlignedBuffer, BlockDevice};

use crate::error::{FilesystemError, Result};

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
