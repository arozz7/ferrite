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

use ferrite_blockdev::BlockDevice;

// ── Public types ──────────────────────────────────────────────────────────────

/// Identified filesystem variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemType {
    Ntfs,
    Fat32,
    Ext4,
    Unknown,
}

impl std::fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilesystemType::Ntfs => write!(f, "NTFS"),
            FilesystemType::Fat32 => write!(f, "FAT32"),
            FilesystemType::Ext4 => write!(f, "ext4"),
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

// ── Free functions ────────────────────────────────────────────────────────────

/// Probe `device` and return the filesystem type found at the start of the
/// volume, or [`FilesystemType::Unknown`] when none of the known signatures
/// match.
///
/// Detection order:
/// 1. NTFS  — OEM ID `"NTFS    "` at offset 3
/// 2. FAT32 — type string `"FAT32   "` at offset 82 + 0x55AA boot signature
/// 3. ext4  — magic `0xEF53` at superblock offset 1024 + 56
pub fn detect_filesystem(device: &dyn BlockDevice) -> FilesystemType {
    // ── NTFS ─────────────────────────────────────────────────────────────────
    if let Ok(boot) = io::read_bytes(device, 0, 512) {
        if boot.len() >= 11 && &boot[3..11] == b"NTFS    " {
            return FilesystemType::Ntfs;
        }
        // ── FAT32 ─────────────────────────────────────────────────────────────
        if boot.len() >= 512
            && boot[510] == 0x55
            && boot[511] == 0xAA
            && boot.len() >= 90
            && &boot[82..90] == b"FAT32   "
        {
            return FilesystemType::Fat32;
        }
    }

    // ── ext4 ─────────────────────────────────────────────────────────────────
    if let Ok(sb) = io::read_bytes(device, 1024, 58 + 2) {
        let magic = u16::from_le_bytes([sb[56], sb[57]]);
        if magic == 0xEF53 {
            return FilesystemType::Ext4;
        }
    }

    FilesystemType::Unknown
}

/// Open a parser for the filesystem detected on `device`.
///
/// Returns `Err(FilesystemError::UnknownFilesystem)` when the volume cannot be
/// identified.
pub fn open_filesystem(device: Arc<dyn BlockDevice>) -> Result<Box<dyn FilesystemParser>> {
    match detect_filesystem(device.as_ref()) {
        FilesystemType::Ntfs => Ok(Box::new(NtfsParser::new(device)?)),
        FilesystemType::Fat32 => Ok(Box::new(Fat32Parser::new(device)?)),
        FilesystemType::Ext4 => Ok(Box::new(Ext4Parser::new(device)?)),
        FilesystemType::Unknown => Err(FilesystemError::UnknownFilesystem),
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
}
