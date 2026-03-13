//! `ferrite-filesystem` — read-only filesystem parsers for NTFS, FAT32, and ext4.
//!
//! # Overview
//!
//! ```ignore
//! use std::sync::Arc;
//! use ferrite_filesystem::{detect_filesystem, open_filesystem, FilesystemType};
//!
//! // Probe the volume
//! let fs_type = detect_filesystem(device.as_ref())?;
//!
//! // Open a typed parser
//! let parser = open_filesystem(Arc::clone(&device))?;
//! let entries = parser.root_directory()?;
//! ```

mod error;
mod ext4;
mod fat32;
mod io;
mod ntfs;

pub use error::{FilesystemError, Result};
pub use ext4::Ext4Parser;
pub use fat32::Fat32Parser;
pub use ntfs::NtfsParser;

use std::io::Write;
use std::sync::Arc;

use ferrite_blockdev::{AlignedBuffer, BlockDevice};
use ferrite_core::types::DeviceInfo;

// ── Public types ──────────────────────────────────────────────────────────────

/// Identified filesystem variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemType {
    Ntfs,
    Fat32,
    Ext4,
    /// exFAT — detected but no parser implemented (detect-only).
    ExFat,
    /// HFS+ or HFSX — detected but no parser implemented (detect-only).
    HfsPlus,
    Unknown,
}

impl std::fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilesystemType::Ntfs => write!(f, "NTFS"),
            FilesystemType::Fat32 => write!(f, "FAT32"),
            FilesystemType::Ext4 => write!(f, "ext4"),
            FilesystemType::ExFat => write!(f, "exFAT"),
            FilesystemType::HfsPlus => write!(f, "HFS+"),
            FilesystemType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// A single file or directory entry returned by a [`FilesystemParser`].
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Filename (no directory separator).
    pub name: String,
    /// Absolute path within the volume.
    pub path: String,
    /// File size in bytes (`0` for directories).
    pub size: u64,
    pub is_dir: bool,
    /// `true` when the entry was found in a deleted state (first byte `0xE5`
    /// for FAT32, in-use flag clear for NTFS, `de_inode == 0` for ext4).
    pub is_deleted: bool,
    /// Creation time as a Unix timestamp, when available.
    pub created: Option<u64>,
    /// Last modification time as a Unix timestamp, when available.
    pub modified: Option<u64>,

    // Filesystem-specific locators (only one will be `Some` at a time).
    /// FAT32: starting cluster number.
    pub first_cluster: Option<u32>,
    /// NTFS: MFT record number.
    pub mft_record: Option<u64>,
    /// ext4: inode number.
    pub inode_number: Option<u32>,
}

// ── Core trait ────────────────────────────────────────────────────────────────

/// Read-only interface for all supported filesystem parsers.
///
/// Implementors must be `Send + Sync`; the parser state is immutable after
/// construction.
pub trait FilesystemParser: Send + Sync {
    /// The filesystem variant this parser handles.
    fn filesystem_type(&self) -> FilesystemType;

    /// Return all entries directly inside the root directory.
    fn root_directory(&self) -> Result<Vec<FileEntry>>;

    /// Return all entries directly inside the directory at `path`.
    ///
    /// `path` uses forward slashes, e.g. `"Windows/System32"`.
    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>>;

    /// Read `entry`'s content and write it to `writer`.
    ///
    /// Returns the number of bytes written.
    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64>;

    /// Return entries that appear to have been deleted (best-effort).
    fn deleted_files(&self) -> Result<Vec<FileEntry>>;
}

// ── OffsetDevice ──────────────────────────────────────────────────────────────

/// Wraps a `BlockDevice` and adds a fixed byte offset to every read.
///
/// Used internally so filesystem parsers — which always read from offset 0 of
/// their "volume" — can transparently operate on a partition that starts
/// somewhere in the middle of a whole-disk device.
struct OffsetDevice {
    inner: Arc<dyn BlockDevice>,
    offset: u64,
}

impl BlockDevice for OffsetDevice {
    fn read_at(
        &self,
        offset: u64,
        buf: &mut AlignedBuffer,
    ) -> ferrite_blockdev::Result<usize> {
        self.inner.read_at(self.offset + offset, buf)
    }

    fn size(&self) -> u64 {
        self.inner.size().saturating_sub(self.offset)
    }

    fn sector_size(&self) -> u32 {
        self.inner.sector_size()
    }

    fn device_info(&self) -> &DeviceInfo {
        self.inner.device_info()
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Probe `device` and return the filesystem type found at the start of the
/// volume, or [`FilesystemType::Unknown`] when none of the known signatures
/// match.
///
/// Checks sector 0 first (raw partition/image), then probes MBR and GPT
/// partition tables so whole-disk devices are handled transparently.
pub fn detect_filesystem(device: &dyn BlockDevice) -> FilesystemType {
    probe_filesystem(device).0
}

/// Open a parser for the filesystem detected on `device`.
///
/// Handles whole-disk devices by probing MBR/GPT partition tables and
/// wrapping the device with an [`OffsetDevice`] when the filesystem starts
/// at a non-zero byte offset.
///
/// Returns `Err(FilesystemError::UnknownFilesystem)` when the volume cannot
/// be identified.
pub fn open_filesystem(device: Arc<dyn BlockDevice>) -> Result<Box<dyn FilesystemParser>> {
    let (fs_type, offset) = probe_filesystem(device.as_ref());

    let vol: Arc<dyn BlockDevice> = if offset > 0 {
        Arc::new(OffsetDevice {
            inner: device,
            offset,
        })
    } else {
        device
    };

    match fs_type {
        FilesystemType::Ntfs => Ok(Box::new(NtfsParser::new(vol)?)),
        FilesystemType::Fat32 => Ok(Box::new(Fat32Parser::new(vol)?)),
        FilesystemType::Ext4 => Ok(Box::new(Ext4Parser::new(vol)?)),
        FilesystemType::ExFat | FilesystemType::HfsPlus | FilesystemType::Unknown => {
            Err(FilesystemError::UnknownFilesystem)
        }
    }
}

// ── Internal probing ──────────────────────────────────────────────────────────

/// Returns `(FilesystemType, byte_offset)` for the first recognisable
/// filesystem found on `device`.
///
/// Detection order:
/// 1. Sector 0 directly (raw volume / partition image)
/// 2. MBR partition table entries
/// 3. GPT partition table entries
fn probe_filesystem(device: &dyn BlockDevice) -> (FilesystemType, u64) {
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
fn detect_at(device: &dyn BlockDevice, byte_offset: u64) -> FilesystemType {
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
        let lba = u32::from_le_bytes(
            boot[base + 8..base + 12]
                .try_into()
                .ok()?,
        ) as u64;
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

    let entry_start_lba =
        u64::from_le_bytes(header[72..80].try_into().ok()?);
    let entry_count =
        u32::from_le_bytes(header[80..84].try_into().ok()?) as usize;
    let entry_size =
        u32::from_le_bytes(header[84..88].try_into().ok()?) as usize;

    if entry_size < 128 || entry_count == 0 {
        return None;
    }

    // Cap at 128 entries to bound the read size.
    let probe_count = entry_count.min(128);
    let entries_data =
        io::read_bytes(device, entry_start_lba * sector_size, probe_count * entry_size).ok()?;

    for i in 0..probe_count {
        let base = i * entry_size;
        if base + 48 > entries_data.len() {
            break;
        }
        // Skip if type GUID is all-zero (unused partition entry).
        if entries_data[base..base + 16].iter().all(|&b| b == 0) {
            continue;
        }
        let start_lba = u64::from_le_bytes(
            entries_data[base + 32..base + 40].try_into().ok()?,
        );
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

// ── Integration-level tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    #[test]
    fn detect_returns_unknown_for_empty_device() {
        let dev = MockBlockDevice::zeroed(2048, 512);
        assert_eq!(detect_filesystem(&dev), FilesystemType::Unknown);
    }

    #[test]
    fn open_filesystem_errors_on_empty() {
        let dev = Arc::new(MockBlockDevice::zeroed(2048, 512));
        assert!(matches!(
            open_filesystem(dev),
            Err(FilesystemError::UnknownFilesystem)
        ));
    }

    #[test]
    fn detect_exfat_volume() {
        let mut data = vec![0u8; 512];
        data[3..11].copy_from_slice(b"EXFAT   ");
        let dev = MockBlockDevice::new(data, 512);
        assert_eq!(detect_filesystem(&dev), FilesystemType::ExFat);
    }

    #[test]
    fn detect_hfsplus_volume() {
        // Need at least 1024 + 60 bytes for io::read_bytes(device, 1024, 60).
        let mut data = vec![0u8; 1084];
        // HFS+ volume header magic 0x482B (big-endian) at device offset 1024.
        data[1024] = 0x48;
        data[1025] = 0x2B;
        let dev = MockBlockDevice::new(data, 512);
        assert_eq!(detect_filesystem(&dev), FilesystemType::HfsPlus);
    }

    #[test]
    fn detect_hfsx_volume() {
        let mut data = vec![0u8; 1084];
        // HFSX magic 0x4858 (big-endian) at device offset 1024.
        data[1024] = 0x48;
        data[1025] = 0x58;
        let dev = MockBlockDevice::new(data, 512);
        assert_eq!(detect_filesystem(&dev), FilesystemType::HfsPlus);
    }

    #[test]
    fn open_exfat_returns_unknown_filesystem_error() {
        let mut data = vec![0u8; 512];
        data[3..11].copy_from_slice(b"EXFAT   ");
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        assert!(matches!(
            open_filesystem(dev),
            Err(FilesystemError::UnknownFilesystem)
        ));
    }

    #[test]
    fn detect_ntfs_on_mbr_first_partition() {
        // Build a fake MBR disk: MBR at sector 0, NTFS volume at sector 2.
        let sector_size = 512usize;
        let total_sectors = 8;
        let mut data = vec![0u8; sector_size * total_sectors];

        // MBR boot signature
        data[510] = 0x55;
        data[511] = 0xAA;

        // First partition entry at offset 446: type=0x07 (NTFS), LBA start=2
        data[446 + 4] = 0x07; // partition type
        data[446 + 8] = 2; // start LBA (little-endian u32)
        data[446 + 9] = 0;
        data[446 + 10] = 0;
        data[446 + 11] = 0;

        // NTFS OEM ID at sector 2
        let ntfs_start = 2 * sector_size;
        data[ntfs_start + 3..ntfs_start + 11].copy_from_slice(b"NTFS    ");

        let dev = MockBlockDevice::new(data, sector_size as u32);
        assert_eq!(detect_filesystem(&dev), FilesystemType::Ntfs);
    }
}
