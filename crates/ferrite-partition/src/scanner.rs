/// Raw-device filesystem signature scanner.
///
/// Reads aligned sectors from a [`BlockDevice`] and checks for the
/// magic bytes that identify NTFS, FAT16, FAT32, and ext4 volume starts.
///
/// Magic signatures checked (relative to each scan position):
///   - NTFS  : bytes  3–10 == `b"NTFS    "`
///   - FAT32 : bytes 82–89 == `b"FAT32   "`
///   - FAT16 : bytes 54–61 == `b"FAT16   "`
///   - ext4  : bytes 1080–1081 == `[0x53, 0xEF]`  (superblock magic)
use ferrite_blockdev::{AlignedBuffer, BlockDevice};
use tracing::debug;

use crate::error::Result;
use crate::types::{FsSignatureHit, FsType};

const NTFS_OFFSET: usize = 3;
const NTFS_MAGIC: &[u8; 8] = b"NTFS    ";

const FAT32_OFFSET: usize = 82;
const FAT32_MAGIC: &[u8; 8] = b"FAT32   ";

const FAT16_OFFSET: usize = 54;
const FAT16_MAGIC: &[u8; 8] = b"FAT16   ";

// ext4 superblock starts at byte 1024 from the partition start;
// s_magic is at byte 56 within the superblock → offset 1080 overall.
const EXT4_OFFSET: usize = 1080;
const EXT4_MAGIC: &[u8; 2] = &[0x53, 0xEF];

// We need at least 1082 bytes to cover all signatures.
const MIN_SCAN_BYTES: u64 = (EXT4_OFFSET + EXT4_MAGIC.len()) as u64;

/// Controls how the scanner steps through the device.
pub struct ScanOptions {
    /// Step between successive scan positions in bytes.
    ///
    /// Must be a multiple of the device sector size.
    /// - Use `sector_size` for an exhaustive sector-by-sector scan.
    /// - Use `1 << 20` (1 MiB) for a fast alignment-based scan (most
    ///   modern partitions start on 1 MiB boundaries).
    pub step: u64,

    /// Byte offset at which to start scanning (inclusive). Defaults to 0.
    pub start_byte: u64,

    /// Byte offset at which to stop scanning (exclusive). `None` = end of device.
    pub end_byte: Option<u64>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            step: 512,
            start_byte: 0,
            end_byte: None,
        }
    }
}

/// Scan `device` for filesystem magic bytes and return all hits.
pub fn scan(device: &dyn BlockDevice, options: &ScanOptions) -> Result<Vec<FsSignatureHit>> {
    let sector_size = device.sector_size() as u64;
    let device_size = device.size();
    let scan_end = options.end_byte.unwrap_or(device_size).min(device_size);

    // Compute how many sectors we need to cover the deepest signature.
    let sectors_needed = MIN_SCAN_BYTES.div_ceil(sector_size);
    let buf_size = (sectors_needed * sector_size) as usize;

    let mut buf = AlignedBuffer::new(buf_size, device.sector_size() as usize);
    let mut hits = Vec::new();

    let mut pos = options.start_byte;
    while pos < scan_end {
        // Don't read past end of device.
        if pos + sector_size > device_size {
            break;
        }

        match device.read_at(pos, &mut buf) {
            Ok(0) => break,
            Err(_) => {
                pos += options.step;
                continue;
            }
            Ok(n) => {
                let data = &buf.as_slice()[..n];
                if let Some(fs_type) = detect(data) {
                    debug!(offset = pos, fs = %fs_type, "signature hit");
                    hits.push(FsSignatureHit {
                        offset_bytes: pos,
                        fs_type,
                    });
                }
            }
        }

        pos += options.step;
    }

    Ok(hits)
}

fn detect(data: &[u8]) -> Option<FsType> {
    if data.len() >= NTFS_OFFSET + NTFS_MAGIC.len()
        && &data[NTFS_OFFSET..NTFS_OFFSET + NTFS_MAGIC.len()] == NTFS_MAGIC
    {
        return Some(FsType::Ntfs);
    }

    if data.len() >= FAT32_OFFSET + FAT32_MAGIC.len()
        && &data[FAT32_OFFSET..FAT32_OFFSET + FAT32_MAGIC.len()] == FAT32_MAGIC
    {
        return Some(FsType::Fat32);
    }

    if data.len() >= FAT16_OFFSET + FAT16_MAGIC.len()
        && &data[FAT16_OFFSET..FAT16_OFFSET + FAT16_MAGIC.len()] == FAT16_MAGIC
    {
        return Some(FsType::Fat16);
    }

    if data.len() >= EXT4_OFFSET + EXT4_MAGIC.len()
        && &data[EXT4_OFFSET..EXT4_OFFSET + EXT4_MAGIC.len()] == EXT4_MAGIC
    {
        return Some(FsType::Ext4);
    }

    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Write;

    use ferrite_blockdev::FileBlockDevice;
    use tempfile::NamedTempFile;

    use super::*;

    const SECTOR: usize = 512;

    fn make_device(sectors: usize, setup: impl FnOnce(&mut Vec<u8>)) -> FileBlockDevice {
        let size = sectors * SECTOR;
        let mut data = vec![0u8; size];
        setup(&mut data);
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        tmp.flush().unwrap();
        FileBlockDevice::open(tmp.into_temp_path().keep().unwrap()).unwrap()
    }

    #[test]
    fn detects_ntfs_at_start() {
        let dev = make_device(4, |d| {
            d[NTFS_OFFSET..NTFS_OFFSET + 8].copy_from_slice(b"NTFS    ");
        });
        let hits = scan(&dev, &ScanOptions::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].offset_bytes, 0);
        assert_eq!(hits[0].fs_type, FsType::Ntfs);
    }

    #[test]
    fn detects_fat32_at_start() {
        let dev = make_device(4, |d| {
            d[FAT32_OFFSET..FAT32_OFFSET + 8].copy_from_slice(b"FAT32   ");
        });
        let hits = scan(&dev, &ScanOptions::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].fs_type, FsType::Fat32);
    }

    #[test]
    fn detects_fat16_at_start() {
        let dev = make_device(4, |d| {
            d[FAT16_OFFSET..FAT16_OFFSET + 8].copy_from_slice(b"FAT16   ");
        });
        let hits = scan(&dev, &ScanOptions::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].fs_type, FsType::Fat16);
    }

    #[test]
    fn detects_ext4_superblock_magic() {
        // ext4 superblock magic needs 3 sectors (bytes 0–1535)
        let dev = make_device(4, |d| {
            d[EXT4_OFFSET] = 0x53;
            d[EXT4_OFFSET + 1] = 0xEF;
        });
        let hits = scan(&dev, &ScanOptions::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].fs_type, FsType::Ext4);
    }

    #[test]
    fn no_signature_returns_empty() {
        let dev = make_device(4, |_| {});
        let hits = scan(&dev, &ScanOptions::default()).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn detects_signature_at_second_sector_with_step() {
        // Place NTFS magic at sector 1 (byte offset 512)
        let dev = make_device(5, |d| {
            d[512 + NTFS_OFFSET..512 + NTFS_OFFSET + 8].copy_from_slice(b"NTFS    ");
        });
        let opts = ScanOptions {
            step: 512,
            start_byte: 0,
            end_byte: None,
        };
        let hits = scan(&dev, &opts).unwrap();
        assert!(hits
            .iter()
            .any(|h| h.offset_bytes == 512 && h.fs_type == FsType::Ntfs));
    }

    #[test]
    fn scan_respects_start_and_end_byte() {
        // NTFS at offset 0, but we scan from byte 512 onward → should not see it
        let dev = make_device(4, |d| {
            d[NTFS_OFFSET..NTFS_OFFSET + 8].copy_from_slice(b"NTFS    ");
        });
        let opts = ScanOptions {
            step: 512,
            start_byte: 512,
            end_byte: None,
        };
        let hits = scan(&dev, &opts).unwrap();
        assert!(hits.is_empty());
    }
}
