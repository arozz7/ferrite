//! ELF size-hint handler — derives file size from section/program headers.

use ferrite_blockdev::BlockDevice;

use super::helpers::{read_u16, read_u32, read_u64};
use crate::carver_io::read_bytes_clamped;

/// Derive the true file size of an ELF executable from its section and program
/// headers.
///
/// Returns `max(section_table_end, max_segment_extent)`.
pub(super) fn elf_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    const MAX_HEADERS: u16 = 4096;

    let ident = read_bytes_clamped(device, file_offset, 6).ok()?;
    if ident.len() < 6 {
        return None;
    }
    // Already validated by header magic (7F ELF), but double-check class and data.
    let class = ident[4]; // 1=32-bit, 2=64-bit
    let data = ident[5]; // 1=LE, 2=BE
    if class != 1 && class != 2 {
        return None;
    }
    let le = match data {
        1 => true,
        2 => false,
        _ => return None,
    };

    let mut max_extent: u64 = 0;

    if class == 1 {
        // 32-bit ELF
        let hdr = read_bytes_clamped(device, file_offset, 52).ok()?;
        if hdr.len() < 52 {
            return None;
        }

        let e_phoff = read_u32(&hdr[28..32], le) as u64;
        let e_phentsize = read_u16(&hdr[42..44], le) as u64;
        let e_phnum = read_u16(&hdr[44..46], le);

        let e_shoff = read_u32(&hdr[32..36], le) as u64;
        let e_shentsize = read_u16(&hdr[46..48], le) as u64;
        let e_shnum = read_u16(&hdr[48..50], le);

        // Section table end
        if e_shoff > 0 && e_shnum > 0 && e_shnum <= MAX_HEADERS {
            max_extent = max_extent.max(e_shoff + e_shentsize * e_shnum as u64);
        }

        // Walk program headers
        if e_phoff > 0 && e_phnum > 0 && e_phnum <= MAX_HEADERS {
            let ph_table_size = e_phentsize * e_phnum as u64;
            let ph_data =
                read_bytes_clamped(device, file_offset + e_phoff, ph_table_size as usize).ok()?;
            for i in 0..e_phnum as u64 {
                let base = (i * e_phentsize) as usize;
                if base + 20 > ph_data.len() {
                    break;
                }
                // p_offset: u32 @4, p_filesz: u32 @16
                let p_offset = read_u32(&ph_data[base + 4..base + 8], le) as u64;
                let p_filesz = read_u32(&ph_data[base + 16..base + 20], le) as u64;
                max_extent = max_extent.max(p_offset.saturating_add(p_filesz));
            }
            max_extent = max_extent.max(e_phoff + ph_table_size);
        }
    } else {
        // 64-bit ELF
        let hdr = read_bytes_clamped(device, file_offset, 64).ok()?;
        if hdr.len() < 64 {
            return None;
        }

        let e_phoff = read_u64(&hdr[32..40], le);
        let e_phentsize = read_u16(&hdr[54..56], le) as u64;
        let e_phnum = read_u16(&hdr[56..58], le);

        let e_shoff = read_u64(&hdr[40..48], le);
        let e_shentsize = read_u16(&hdr[58..60], le) as u64;
        let e_shnum = read_u16(&hdr[60..62], le);

        // Section table end
        if e_shoff > 0 && e_shnum > 0 && e_shnum <= MAX_HEADERS {
            max_extent = max_extent.max(e_shoff + e_shentsize * e_shnum as u64);
        }

        // Walk program headers
        if e_phoff > 0 && e_phnum > 0 && e_phnum <= MAX_HEADERS {
            let ph_table_size = e_phentsize * e_phnum as u64;
            let ph_data =
                read_bytes_clamped(device, file_offset + e_phoff, ph_table_size as usize).ok()?;
            for i in 0..e_phnum as u64 {
                let base = (i * e_phentsize) as usize;
                if base + 48 > ph_data.len() {
                    break;
                }
                // 64-bit: p_offset: u64 @8, p_filesz: u64 @32
                let p_offset = read_u64(&ph_data[base + 8..base + 16], le);
                let p_filesz = read_u64(&ph_data[base + 32..base + 40], le);
                max_extent = max_extent.max(p_offset.saturating_add(p_filesz));
            }
            max_extent = max_extent.max(e_phoff + ph_table_size);
        }
    }

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
    fn elf64_le_basic() {
        // Craft a minimal 64-bit LE ELF.
        let mut data = vec![0u8; 8192];
        // ELF magic + class + data
        data[0..4].copy_from_slice(&[0x7F, 0x45, 0x4C, 0x46]); // \x7FELF
        data[4] = 2; // 64-bit
        data[5] = 1; // LE

        // e_phoff = 64 (u64 LE @32)
        data[32..40].copy_from_slice(&64u64.to_le_bytes());
        // e_phentsize = 56 (u16 LE @54)
        data[54..56].copy_from_slice(&56u16.to_le_bytes());
        // e_phnum = 1 (u16 LE @56)
        data[56..58].copy_from_slice(&1u16.to_le_bytes());

        // e_shoff = 4096 (u64 LE @40)
        data[40..48].copy_from_slice(&4096u64.to_le_bytes());
        // e_shentsize = 64 (u16 LE @58)
        data[58..60].copy_from_slice(&64u16.to_le_bytes());
        // e_shnum = 5 (u16 LE @60)
        data[60..62].copy_from_slice(&5u16.to_le_bytes());

        // Program header at 64: p_offset(u64 @8)=1024, p_filesz(u64 @32)=2048
        data[64 + 8..64 + 16].copy_from_slice(&1024u64.to_le_bytes());
        data[64 + 32..64 + 40].copy_from_slice(&2048u64.to_le_bytes());

        let dev = device_from(data);
        let result = elf_hint(dev.as_ref(), 0);
        // section_table_end = 4096 + 64*5 = 4416
        // segment_extent = 1024 + 2048 = 3072
        // max = 4416
        assert_eq!(result, Some(4416));
    }

    #[test]
    fn elf32_be_basic() {
        let mut data = vec![0u8; 4096];
        data[0..4].copy_from_slice(&[0x7F, 0x45, 0x4C, 0x46]);
        data[4] = 1; // 32-bit
        data[5] = 2; // BE

        // e_phoff = 52 (u32 BE @28)
        data[28..32].copy_from_slice(&52u32.to_be_bytes());
        // e_phentsize = 32 (u16 BE @42)
        data[42..44].copy_from_slice(&32u16.to_be_bytes());
        // e_phnum = 1 (u16 BE @44)
        data[44..46].copy_from_slice(&1u16.to_be_bytes());

        // e_shoff = 0 (no section table)
        // e_shnum = 0

        // Program header at 52: p_offset(u32 @4)=256, p_filesz(u32 @16)=512
        data[52 + 4..52 + 8].copy_from_slice(&256u32.to_be_bytes());
        data[52 + 16..52 + 20].copy_from_slice(&512u32.to_be_bytes());

        let dev = device_from(data);
        let result = elf_hint(dev.as_ref(), 0);
        // max(segment: 256+512=768, ph_table: 52+32=84)
        assert_eq!(result, Some(768));
    }

    #[test]
    fn elf_invalid_class_returns_none() {
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(&[0x7F, 0x45, 0x4C, 0x46]);
        data[4] = 3; // invalid class
        data[5] = 1;
        let dev = device_from(data);
        assert_eq!(elf_hint(dev.as_ref(), 0), None);
    }
}
