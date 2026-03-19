//! OGG bitstream container size-hint handler (page walker).

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

pub(super) fn ogg_stream_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    const OGG_MAGIC: &[u8; 4] = b"OggS";
    const MAX_PAGES: u32 = 100_000;

    let device_size = device.size();
    let mut pos = file_offset;

    for _ in 0..MAX_PAGES {
        if pos >= device_size {
            break;
        }
        let hdr = read_bytes_clamped(device, pos, 27).ok()?;
        if hdr.len() < 27 {
            break;
        }
        if &hdr[0..4] != OGG_MAGIC {
            break;
        }
        let header_type = hdr[5];
        let num_segments = hdr[26] as u64;

        let seg_table = read_bytes_clamped(device, pos + 27, num_segments as usize).ok()?;
        if seg_table.len() < num_segments as usize {
            break;
        }
        let data_size: u64 = seg_table.iter().map(|&b| b as u64).sum();
        let page_size = 27 + num_segments + data_size;

        if header_type & 0x04 != 0 {
            return Some(pos - file_offset + page_size);
        }

        pos = pos.saturating_add(page_size);
    }

    None
}
