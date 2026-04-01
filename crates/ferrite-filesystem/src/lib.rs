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

mod apfs;
mod detect;
mod error;
mod exfat;
mod ext4;
mod ext4_dir;
mod fat32;
mod io;
mod ntfs;
mod ntfs_helpers;
mod offset_device;

pub use apfs::ApfsParser;
pub use error::{FilesystemError, Result};
pub use exfat::ExFatParser;
pub use ext4::Ext4Parser;
pub use fat32::Fat32Parser;
pub use ntfs::NtfsParser;

use std::io::Write;
use std::sync::Arc;

use ferrite_blockdev::BlockDevice;

use crate::detect::{detect_at, probe_filesystem};
use crate::offset_device::OffsetDevice;

// ── Public types ──────────────────────────────────────────────────────────────

/// Identified filesystem variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemType {
    Ntfs,
    Fat32,
    Ext4,
    ExFat,
    Apfs,
    /// HFS+ or HFSX — detected but no parser implemented (detect-only).
    HfsPlus,
    /// BitLocker-encrypted NTFS volume — detected via `-FVE-FS-` OEM ID.
    /// No parser is implemented; the volume must be decrypted before recovery.
    Encrypted,
    Unknown,
}

impl std::fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilesystemType::Ntfs => write!(f, "NTFS"),
            FilesystemType::Fat32 => write!(f, "FAT32"),
            FilesystemType::Ext4 => write!(f, "ext4"),
            FilesystemType::ExFat => write!(f, "exFAT"),
            FilesystemType::Apfs => write!(f, "APFS"),
            FilesystemType::HfsPlus => write!(f, "HFS+"),
            FilesystemType::Encrypted => write!(f, "BitLocker (encrypted)"),
            FilesystemType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Estimated probability that a deleted file's data is still intact on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecoveryChance {
    /// Cluster/block pointers are intact and data appears unallocated — very likely recoverable.
    High,
    /// Partial information available — recovery may be incomplete.
    Medium,
    /// Clusters reallocated or no block info — recovery unlikely.
    Low,
    /// Not assessed (live files, directories, or unsupported filesystem).
    Unknown,
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

    /// Absolute byte offset of the file's first data byte within the volume
    /// device (i.e. relative to offset 0 of the device passed to the parser).
    ///
    /// `None` for directories, resident/inline files (too small to carve), or
    /// when the first cluster/block is unreadable.  When `Some`, this value
    /// can be compared directly against [`ferrite_carver::CarveHit::byte_offset`]
    /// after adjusting for the partition's position on the raw device.
    pub data_byte_offset: Option<u64>,

    /// Estimated probability that a deleted file's data is still intact.
    /// Always [`RecoveryChance::Unknown`] for live (non-deleted) files.
    pub recovery_chance: RecoveryChance,
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

    /// Walk ALL file entries — live and deleted — across the entire filesystem.
    ///
    /// Used by [`build_metadata_index`] to build the offset→name lookup table.
    /// The default implementation merges `root_directory()` and
    /// `deleted_files()`; parsers that can do a deeper scan (e.g. NTFS via
    /// full MFT traversal) should override this method.
    fn enumerate_files(&self) -> Result<Vec<FileEntry>> {
        let mut all = self.root_directory().unwrap_or_default();
        all.retain(|e| !e.is_dir);
        all.extend(self.deleted_files().unwrap_or_default());
        Ok(all)
    }
}

// ── Metadata index ────────────────────────────────────────────────────────────

/// Condensed file metadata stored in a [`MetadataIndex`] entry.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// Original filename (no path separator).
    pub name: String,
    /// Full path within the volume (e.g. `"/Photos/trip.jpg"`).
    pub path: String,
    /// File size in bytes as recorded by the filesystem.
    pub size: u64,
    /// `true` when the entry was found in a deleted state.
    pub is_deleted: bool,
    /// Creation timestamp (Unix seconds), if available.
    pub created: Option<u64>,
    /// Last-modification timestamp (Unix seconds), if available.
    pub modified: Option<u64>,
}

/// Maps absolute device byte offsets → original file metadata.
///
/// Built by [`build_metadata_index`] and used during carving extraction to
/// name recovered files after their originals instead of by raw offset.
#[derive(Debug, Default)]
pub struct MetadataIndex {
    entries: std::collections::HashMap<u64, FileMetadata>,
}

impl MetadataIndex {
    /// Look up the file whose first data byte is at `byte_offset` on the
    /// raw device.  Returns `None` when no filesystem entry maps to that offset.
    pub fn lookup(&self, byte_offset: u64) -> Option<&FileMetadata> {
        self.entries.get(&byte_offset)
    }

    /// Number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Scan `device` for all recognisable filesystems/partitions and build a
/// byte-offset → file-metadata lookup table.
///
/// `device` should be the **full raw device** (whole disk or image).
/// Partition offsets are resolved automatically via MBR/GPT probing.
///
/// Returns an empty [`MetadataIndex`] when no filesystem can be parsed; never
/// returns an error so callers do not need to handle partial results.
pub fn build_metadata_index(device: Arc<dyn BlockDevice>) -> MetadataIndex {
    let mut index = MetadataIndex::default();

    // Collect all partition offsets to probe (including offset 0 for raw volumes).
    let mut offsets: Vec<u64> = vec![0];
    if let Some(mbr_parts) = partition_byte_offsets_mbr(device.as_ref()) {
        offsets.extend(mbr_parts);
    } else if let Some(gpt_parts) = partition_byte_offsets_gpt(device.as_ref()) {
        offsets.extend(gpt_parts);
    }
    offsets.dedup();

    for part_offset in offsets {
        let vol: Arc<dyn BlockDevice> = if part_offset > 0 {
            Arc::new(OffsetDevice {
                inner: Arc::clone(&device),
                offset: part_offset,
            })
        } else {
            Arc::clone(&device)
        };

        let fs_type = detect_at(vol.as_ref(), 0);
        let parser: Box<dyn FilesystemParser> = match fs_type {
            FilesystemType::Ntfs => match NtfsParser::new(Arc::clone(&vol)) {
                Ok(p) => Box::new(p),
                Err(_) => continue,
            },
            FilesystemType::Fat32 => match Fat32Parser::new(Arc::clone(&vol)) {
                Ok(p) => Box::new(p),
                Err(_) => continue,
            },
            FilesystemType::Ext4 => match Ext4Parser::new(Arc::clone(&vol)) {
                Ok(p) => Box::new(p),
                Err(_) => continue,
            },
            FilesystemType::ExFat => match ExFatParser::new(Arc::clone(&vol)) {
                Ok(p) => Box::new(p),
                Err(_) => continue,
            },
            FilesystemType::Apfs => match ApfsParser::new(Arc::clone(&vol)) {
                Ok(p) => Box::new(p),
                Err(_) => continue,
            },
            _ => continue,
        };

        let files = match parser.enumerate_files() {
            Ok(f) => f,
            Err(_) => continue,
        };

        for file in files {
            if let Some(vol_offset) = file.data_byte_offset {
                let abs_offset = part_offset + vol_offset;
                // First match wins — earlier partitions take priority.
                index.entries.entry(abs_offset).or_insert(FileMetadata {
                    name: file.name,
                    path: file.path,
                    size: file.size,
                    is_deleted: file.is_deleted,
                    created: file.created,
                    modified: file.modified,
                });
            }
        }
    }

    index
}

/// Collect all partition start byte offsets from an MBR partition table.
fn partition_byte_offsets_mbr(device: &dyn BlockDevice) -> Option<Vec<u64>> {
    let boot = io::read_bytes(device, 0, 512).ok()?;
    if boot.len() < 512 || boot[510] != 0x55 || boot[511] != 0xAA {
        return None;
    }
    let sector_size = device.sector_size() as u64;
    let mut offsets = Vec::new();
    for i in 0..4usize {
        let base = 446 + i * 16;
        if boot[base + 4] == 0 {
            continue;
        }
        let lba = u32::from_le_bytes(boot[base + 8..base + 12].try_into().ok()?) as u64;
        if lba > 0 {
            offsets.push(lba * sector_size);
        }
    }
    if offsets.is_empty() {
        None
    } else {
        Some(offsets)
    }
}

/// Collect all partition start byte offsets from a GPT partition table.
fn partition_byte_offsets_gpt(device: &dyn BlockDevice) -> Option<Vec<u64>> {
    let sector_size = device.sector_size() as u64;
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
    let probe_count = entry_count.min(128);
    let entries_data = io::read_bytes(
        device,
        entry_start_lba * sector_size,
        probe_count * entry_size,
    )
    .ok()?;

    let mut offsets = Vec::new();
    for i in 0..probe_count {
        let base = i * entry_size;
        if base + 48 > entries_data.len() {
            break;
        }
        if entries_data[base..base + 16].iter().all(|&b| b == 0) {
            continue;
        }
        let start_lba = u64::from_le_bytes(entries_data[base + 32..base + 40].try_into().ok()?);
        if start_lba > 0 {
            offsets.push(start_lba * sector_size);
        }
    }
    if offsets.is_empty() {
        None
    } else {
        Some(offsets)
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

/// Detect the filesystem type at a specific LBA on `device`, bypassing any
/// partition table.
///
/// Converts `lba` to a byte offset using the device sector size and delegates
/// to the internal boot-sector / superblock probe.  Returns
/// [`FilesystemType::Unknown`] when no known signature is found.
///
/// Useful when the partition table is absent or corrupt: pair with
/// `ferrite_partition::scan()` to find candidate LBAs, then call this
/// function to confirm each hit before opening a parser.
pub fn detect_filesystem_at(device: &dyn BlockDevice, lba: u64) -> FilesystemType {
    detect_at(device, lba * device.sector_size() as u64)
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
        FilesystemType::ExFat => Ok(Box::new(ExFatParser::new(vol)?)),
        FilesystemType::Apfs => Ok(Box::new(ApfsParser::new(vol)?)),
        FilesystemType::HfsPlus | FilesystemType::Encrypted | FilesystemType::Unknown => {
            Err(FilesystemError::UnknownFilesystem)
        }
    }
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
    fn detect_bitlocker_volume() {
        let mut data = vec![0u8; 512];
        // BitLocker replaces the NTFS OEM ID with `-FVE-FS-` at bytes [3..11].
        data[3..11].copy_from_slice(b"-FVE-FS-");
        let dev = MockBlockDevice::new(data, 512);
        assert_eq!(detect_filesystem(&dev), FilesystemType::Encrypted);
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
    fn open_exfat_returns_err_on_truncated_device() {
        // A 512-byte device with the exFAT OEM name but no valid VBR fields
        // should produce a parse error (BufferTooSmall or InvalidStructure),
        // not UnknownFilesystem, now that ExFatParser is wired in.
        let mut data = vec![0u8; 512];
        data[3..11].copy_from_slice(b"EXFAT   ");
        // BytesPerSectorShift=9 (valid), but FatOffset/ClusterHeapOffset point
        // beyond the 512-byte device, so reading the FAT/directory will fail.
        data[108] = 9; // BytesPerSectorShift
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        // Parser construction itself succeeds; errors happen on first I/O.
        // We just verify open_filesystem no longer returns UnknownFilesystem.
        let result = open_filesystem(dev);
        assert!(
            !matches!(result, Err(FilesystemError::UnknownFilesystem)),
            "exFAT should now be handled by ExFatParser, not return UnknownFilesystem"
        );
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

    #[test]
    fn metadata_index_lookup_and_empty() {
        // An empty index reports is_empty() and lookup returns None.
        let idx = MetadataIndex::default();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
        assert!(idx.lookup(0).is_none());
        assert!(idx.lookup(12345).is_none());
    }

    #[test]
    fn metadata_index_insert_and_lookup() {
        let mut idx = MetadataIndex::default();
        idx.entries.insert(
            4096,
            FileMetadata {
                name: "photo.jpg".into(),
                path: "/DCIM/photo.jpg".into(),
                size: 1024,
                is_deleted: false,
                created: None,
                modified: None,
            },
        );
        assert_eq!(idx.len(), 1);
        assert!(!idx.is_empty());
        let meta = idx.lookup(4096).expect("expected entry at offset 4096");
        assert_eq!(meta.name, "photo.jpg");
        assert_eq!(meta.path, "/DCIM/photo.jpg");
        assert!(idx.lookup(4097).is_none());
    }

    #[test]
    fn build_metadata_index_on_empty_device_returns_empty() {
        let dev = Arc::new(MockBlockDevice::zeroed(4096, 512));
        let idx = build_metadata_index(dev);
        assert!(
            idx.is_empty(),
            "expected empty index for unformatted device"
        );
    }

    #[test]
    fn detect_filesystem_at_lba_zero_ntfs() {
        let mut data = vec![0u8; 512];
        data[3..11].copy_from_slice(b"NTFS    ");
        let dev = MockBlockDevice::new(data, 512);
        assert_eq!(detect_filesystem_at(&dev, 0), FilesystemType::Ntfs);
    }

    #[test]
    fn detect_filesystem_at_nonzero_lba() {
        // Place NTFS magic at LBA 2 (byte offset 1024 for 512-byte sectors).
        let sector_size = 512usize;
        let mut data = vec![0u8; 4 * sector_size];
        let ntfs_start = 2 * sector_size;
        data[ntfs_start + 3..ntfs_start + 11].copy_from_slice(b"NTFS    ");
        let dev = MockBlockDevice::new(data, sector_size as u32);
        assert_eq!(detect_filesystem_at(&dev, 0), FilesystemType::Unknown);
        assert_eq!(detect_filesystem_at(&dev, 2), FilesystemType::Ntfs);
    }

    #[test]
    fn detect_filesystem_at_unknown_on_zero_device() {
        let dev = MockBlockDevice::zeroed(2048, 512);
        assert_eq!(detect_filesystem_at(&dev, 0), FilesystemType::Unknown);
    }
}
