//! Internal filesystem-detection and partition-probing helpers.

use ferrite_blockdev::BlockDevice;

use crate::io;
use crate::FilesystemType;

// ── Filesystem probing ────────────────────────────────────────────────────────

/// Returns `(FilesystemType, byte_offset)` for the first recognisable
/// filesystem found on `device`.
///
/// Detection order:
/// 1. Sector 0 directly (raw volume / partition image)
/// 2. MBR partition table entries
/// 3. GPT partition table entries
pub(crate) fn probe_filesystem(device: &dyn BlockDevice) -> (FilesystemType, u64) {
    // Raw volume or partition image — filesystem starts at byte 0.
    let fs = detect_at(device, 0);
    if fs != FilesystemType::Unknown {
        return (fs, 0);
    }

    // Whole disk — try MBR then GPT.
    if let Some(result) = probe_mbr(device) {
        return result;
    }
    if let Some(result) = probe_gpt(device) {
        return result;
    }

    (FilesystemType::Unknown, 0)
}

/// Detect a filesystem signature at a specific byte offset within `device`.
pub(crate) fn detect_at(device: &dyn BlockDevice, byte_offset: u64) -> FilesystemType {
    // Boot sector (NTFS / exFAT / FAT32)
    if let Ok(boot) = io::read_bytes(device, byte_offset, 512) {
        if boot.len() >= 11 && &boot[3..11] == b"NTFS    " {
            return FilesystemType::Ntfs;
        }
        if boot.len() >= 11 && &boot[3..11] == b"EXFAT   " {
            return FilesystemType::ExFat;
        }
        if boot.len() >= 512
            && boot[510] == 0x55
            && boot[511] == 0xAA
            && boot.len() >= 90
            && &boot[82..90] == b"FAT32   "
        {
            return FilesystemType::Fat32;
        }
    }

    // Volume header at offset +1024 (HFS+ and ext4)
    if let Ok(sb) = io::read_bytes(device, byte_offset + 1024, 60) {
        if sb.len() >= 2 {
            let hfs_magic = u16::from_be_bytes([sb[0], sb[1]]);
            if hfs_magic == 0x482B || hfs_magic == 0x4858 {
                return FilesystemType::HfsPlus;
            }
        }
        if sb.len() >= 58 {
            let ext4_magic = u16::from_le_bytes([sb[56], sb[57]]);
            if ext4_magic == 0xEF53 {
                return FilesystemType::Ext4;
            }
        }
    }

    FilesystemType::Unknown
}

/// Scan MBR partition entries at byte 446 and probe each non-empty slot.
fn probe_mbr(device: &dyn BlockDevice) -> Option<(FilesystemType, u64)> {
    let boot = io::read_bytes(device, 0, 512).ok()?;
    if boot.len() < 512 || boot[510] != 0x55 || boot[511] != 0xAA {
        return None;
    }

    let sector_size = device.sector_size() as u64;

    for i in 0..4usize {
        let base = 446 + i * 16;
        if base + 16 > boot.len() {
            break;
        }
        let part_type = boot[base + 4];
        if part_type == 0 {
            continue; // empty slot
        }
        let lba = u32::from_le_bytes(boot[base + 8..base + 12].try_into().ok()?) as u64;
        if lba == 0 {
            continue;
        }
        let byte_offset = lba * sector_size;
        let fs = detect_at(device, byte_offset);
        if fs != FilesystemType::Unknown {
            return Some((fs, byte_offset));
        }
    }
    None
}

/// Scan GPT partition entries and probe each non-empty slot.
fn probe_gpt(device: &dyn BlockDevice) -> Option<(FilesystemType, u64)> {
    let sector_size = device.sector_size() as u64;

    // GPT header lives at sector 1.
    let header = io::read_bytes(device, sector_size, 92).ok()?;
    if header.len() < 92 || &header[0..8] != b"EFI PART" {
        return None;
    }

    let entry_start_lba = u64::from_le_bytes(header[72..80].try_into().ok()?);
    let entry_count = u32::from_le_bytes(header[80..84].try_into().ok()?) as usize;
    let entry_size = u32::from_le_bytes(header[84..88].try_into().ok()?) as usize;

    if entry_size < 128 || entry_count == 0 {
        return None;
    }

    // Cap at 128 entries to bound the read size.
    let probe_count = entry_count.min(128);
    let entries_data = io::read_bytes(
        device,
        entry_start_lba * sector_size,
        probe_count * entry_size,
    )
    .ok()?;

    for i in 0..probe_count {
        let base = i * entry_size;
        if base + 48 > entries_data.len() {
            break;
        }
        // Skip if type GUID is all-zero (unused partition entry).
        if entries_data[base..base + 16].iter().all(|&b| b == 0) {
            continue;
        }
        let start_lba = u64::from_le_bytes(entries_data[base + 32..base + 40].try_into().ok()?);
        if start_lba == 0 {
            continue;
        }
        let byte_offset = start_lba * sector_size;
        let fs = detect_at(device, byte_offset);
        if fs != FilesystemType::Unknown {
            return Some((fs, byte_offset));
        }
    }
    None
}
