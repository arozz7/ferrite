//! PE/EXE size-hint handler — derives file size from the PE section table.

use ferrite_blockdev::BlockDevice;

use super::helpers::{read_u16_le, read_u32_le};
use crate::carver_io::read_bytes_clamped;

/// Derive the true file size of a Windows PE executable from its section table.
///
/// Algorithm:
/// 1. Read `e_lfanew` (u32 LE @60) → PE signature offset
/// 2. Validate `PE\0\0` at `e_lfanew`
/// 3. Read `NumberOfSections` (u16 LE @`e_lfanew+6`), `SizeOfOptionalHeader` (u16 LE @`e_lfanew+20`)
/// 4. Section table starts at `e_lfanew + 24 + SizeOfOptionalHeader`
/// 5. Walk sections (40 bytes each): `max(PointerToRawData + SizeOfRawData)`
/// 6. Safety: cap at 256 sections, validate offsets
pub(super) fn pe_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    const MAX_SECTIONS: u16 = 256;

    // Read e_lfanew at offset 60 (u32 LE).
    let lfanew_bytes = read_bytes_clamped(device, file_offset + 60, 4).ok()?;
    if lfanew_bytes.len() < 4 {
        return None;
    }
    let e_lfanew = read_u32_le(&lfanew_bytes) as u64;

    // Sanity: e_lfanew must be in [64, 16384] (same as pre_validate).
    if !(64..=16384).contains(&e_lfanew) {
        return None;
    }

    // Validate PE signature at e_lfanew.
    let pe_sig = read_bytes_clamped(device, file_offset + e_lfanew, 4).ok()?;
    if pe_sig.len() < 4 || &pe_sig[..4] != b"PE\0\0" {
        return None;
    }

    // COFF header: NumberOfSections (u16 LE @e_lfanew+6)
    let coff_bytes = read_bytes_clamped(device, file_offset + e_lfanew + 4, 20).ok()?;
    if coff_bytes.len() < 20 {
        return None;
    }
    let num_sections = read_u16_le(&coff_bytes[2..4]);
    let size_of_optional = read_u16_le(&coff_bytes[16..18]);

    if num_sections == 0 || num_sections > MAX_SECTIONS {
        return None;
    }

    // Section table starts at e_lfanew + 24 + SizeOfOptionalHeader.
    let section_table_off = e_lfanew + 24 + size_of_optional as u64;
    let table_size = num_sections as u64 * 40;

    let sections =
        read_bytes_clamped(device, file_offset + section_table_off, table_size as usize).ok()?;
    if (sections.len() as u64) < table_size {
        return None;
    }

    let mut max_extent: u64 = 0;
    for i in 0..num_sections as usize {
        let base = i * 40;
        // PointerToRawData: u32 LE at section offset +20
        // SizeOfRawData:    u32 LE at section offset +16
        let size_of_raw = read_u32_le(&sections[base + 16..base + 20]) as u64;
        let ptr_to_raw = read_u32_le(&sections[base + 20..base + 24]) as u64;
        let extent = ptr_to_raw.saturating_add(size_of_raw);
        if extent > max_extent {
            max_extent = extent;
        }
    }

    // Also account for the section table itself.
    let table_end = section_table_off + table_size;
    max_extent = max_extent.max(table_end);

    if max_extent > 0 {
        Some(max_extent)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_blockdev::MockBlockDevice;
    use std::sync::Arc;

    fn device_from(data: Vec<u8>) -> Arc<dyn ferrite_blockdev::BlockDevice> {
        Arc::new(MockBlockDevice::new(data, 512))
    }

    #[test]
    fn pe_basic_two_sections() {
        // Craft a minimal PE with 2 sections.
        let mut data = vec![0u8; 4096];
        // MZ header
        data[0..2].copy_from_slice(b"MZ");
        // e_lfanew at offset 60 = 128
        data[60..64].copy_from_slice(&128u32.to_le_bytes());

        // PE signature at 128
        data[128..132].copy_from_slice(b"PE\0\0");
        // COFF header at 132: Machine(2), NumberOfSections(2), ...
        data[134..136].copy_from_slice(&2u16.to_le_bytes()); // num_sections
                                                             // SizeOfOptionalHeader at 132+16=148
        data[148..150].copy_from_slice(&112u16.to_le_bytes()); // typical optional header size

        // Section table at 128 + 24 + 112 = 264
        // Section 0: SizeOfRawData=512 @16, PointerToRawData=1024 @20
        let sec0 = 264;
        data[sec0 + 16..sec0 + 20].copy_from_slice(&512u32.to_le_bytes());
        data[sec0 + 20..sec0 + 24].copy_from_slice(&1024u32.to_le_bytes());

        // Section 1: SizeOfRawData=256 @16, PointerToRawData=2048 @20
        let sec1 = 264 + 40;
        data[sec1 + 16..sec1 + 20].copy_from_slice(&256u32.to_le_bytes());
        data[sec1 + 20..sec1 + 24].copy_from_slice(&2048u32.to_le_bytes());

        let dev = device_from(data);
        let result = pe_hint(dev.as_ref(), 0);
        // max(1024+512=1536, 2048+256=2304) = 2304
        assert_eq!(result, Some(2304));
    }

    #[test]
    fn pe_invalid_lfanew_returns_none() {
        let mut data = vec![0u8; 512];
        data[0..2].copy_from_slice(b"MZ");
        // e_lfanew = 0 (invalid)
        data[60..64].copy_from_slice(&0u32.to_le_bytes());
        let dev = device_from(data);
        assert_eq!(pe_hint(dev.as_ref(), 0), None);
    }

    #[test]
    fn pe_bad_pe_sig_returns_none() {
        let mut data = vec![0u8; 4096];
        data[0..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&128u32.to_le_bytes());
        data[128..132].copy_from_slice(b"XX\0\0"); // wrong signature
        let dev = device_from(data);
        assert_eq!(pe_hint(dev.as_ref(), 0), None);
    }
}
