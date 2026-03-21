//! ISO 9660 disc image size-hint handler.
//!
//! Reads the Primary Volume Descriptor (PVD) at sector 16 (file offset 32 768)
//! and derives the exact image size from the embedded volume block count and
//! logical block size fields.  Without this hint the ISO signature falls back
//! to its 8.75 GiB `max_size`, producing an oversized file full of trailing
//! garbage that prevents Windows from mounting it.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Byte offset of the PVD sector from the start of the ISO image.
const PVD_OFFSET: u64 = 32_768; // sector 16 × 2048 bytes/sector

/// Byte offset of Volume Space Size (u32 LE) within the PVD sector.
const VSS_LE_OFFSET: u64 = 80;

/// Byte offset of Logical Block Size (u16 LE) within the PVD sector.
const LBS_LE_OFFSET: u64 = 128;

pub(super) fn iso9660_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // Read Volume Space Size: u32 LE at PVD+80.
    let vss_buf = read_bytes_clamped(device, file_offset + PVD_OFFSET + VSS_LE_OFFSET, 4).ok()?;
    let vss_arr: [u8; 4] = vss_buf[..4].try_into().ok()?;
    let volume_space_size = u32::from_le_bytes(vss_arr) as u64;
    if volume_space_size == 0 {
        return None;
    }

    // Read Logical Block Size: u16 LE at PVD+128 (almost always 2048).
    let lbs_buf = read_bytes_clamped(device, file_offset + PVD_OFFSET + LBS_LE_OFFSET, 2).ok()?;
    let lbs_arr: [u8; 2] = lbs_buf[..2].try_into().ok()?;
    let logical_block_size = u16::from_le_bytes(lbs_arr) as u64;

    // Sanity-check: must be a power of 2 in [512, 32768].
    if !(512..=32_768).contains(&logical_block_size) || !logical_block_size.is_power_of_two() {
        return None;
    }

    Some(volume_space_size.saturating_mul(logical_block_size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_blockdev::MockBlockDevice;

    fn make_device_with_pvd(volume_blocks: u32, block_size: u16) -> MockBlockDevice {
        // ISO image needs at least PVD_OFFSET + 132 bytes.
        let size = PVD_OFFSET + 200;
        let mut data = vec![0u8; size as usize];

        // Write Volume Space Size (u32 LE) at PVD+80.
        let vss = volume_blocks.to_le_bytes();
        let vss_pos = (PVD_OFFSET + VSS_LE_OFFSET) as usize;
        data[vss_pos..vss_pos + 4].copy_from_slice(&vss);

        // Write Logical Block Size (u16 LE) at PVD+128.
        let lbs = block_size.to_le_bytes();
        let lbs_pos = (PVD_OFFSET + LBS_LE_OFFSET) as usize;
        data[lbs_pos..lbs_pos + 2].copy_from_slice(&lbs);

        MockBlockDevice::new(data, 512)
    }

    #[test]
    fn iso9660_standard_2048_block() {
        // Typical CD-ROM: 350 000 blocks × 2048 = 716 800 000 bytes (~683 MiB).
        let device = make_device_with_pvd(350_000, 2048);
        let size = iso9660_hint(&device, 0).unwrap();
        assert_eq!(size, 350_000 * 2048);
    }

    #[test]
    fn iso9660_zero_volume_blocks_returns_none() {
        let device = make_device_with_pvd(0, 2048);
        assert!(iso9660_hint(&device, 0).is_none());
    }

    #[test]
    fn iso9660_bad_block_size_returns_none() {
        // Block size 999 is not a power of two — should be rejected.
        let device = make_device_with_pvd(100_000, 999);
        assert!(iso9660_hint(&device, 0).is_none());
    }

    #[test]
    fn iso9660_nonzero_file_offset() {
        // Image starts 4096 bytes into the device (e.g. carved from a larger image).
        let base: u64 = 4096;
        let size = (PVD_OFFSET + 200 + base) as usize;
        let mut data = vec![0u8; size];

        let blocks: u32 = 200_000;
        let bsize: u16 = 2048;

        let vss_pos = (base + PVD_OFFSET + VSS_LE_OFFSET) as usize;
        data[vss_pos..vss_pos + 4].copy_from_slice(&blocks.to_le_bytes());

        let lbs_pos = (base + PVD_OFFSET + LBS_LE_OFFSET) as usize;
        data[lbs_pos..lbs_pos + 2].copy_from_slice(&bsize.to_le_bytes());

        let device = MockBlockDevice::new(data, 512);
        let result = iso9660_hint(&device, base).unwrap();
        assert_eq!(result, 200_000 * 2048);
    }
}
