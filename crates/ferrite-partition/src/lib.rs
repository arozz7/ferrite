//! `ferrite-partition` — MBR/GPT partition table parsing and filesystem
//! signature scanning.
//!
//! # High-level usage
//!
//! ```ignore
//! // Read the partition table from a block device
//! let table = ferrite_partition::read_partition_table(&device)?;
//!
//! // Fall back to signature scanning when the table is absent or corrupt
//! if table.is_empty() {
//!     let hits = ferrite_partition::scan(&device, &ScanOptions::default())?;
//!     let recovered = ferrite_partition::reconstruct(&hits, device.size() / device.sector_size() as u64, device.sector_size());
//! }
//! ```

mod error;
mod gpt;
mod mbr;
mod reconstruct;
mod scanner;
mod types;

pub use error::{PartitionError, Result};
pub use reconstruct::from_scan_hits;
pub use scanner::{scan, ScanOptions};
pub use types::{
    FsSignatureHit, FsType, PartitionEntry, PartitionKind, PartitionTable, PartitionTableKind,
};

use ferrite_blockdev::{AlignedBuffer, BlockDevice};

/// Read the partition table, falling back to filesystem-signature scanning when
/// the MBR/GPT table is absent or yields no entries.
///
/// Algorithm:
/// 1. Try [`read_partition_table`].
/// 2. If the returned table has at least one entry, return it immediately.
/// 3. Otherwise scan the first 2 GiB of the device at 1 MiB alignment steps
///    looking for filesystem boot-sector signatures (NTFS, FAT16/32, ext4).
/// 4. Reconstruct a [`PartitionTable`] with `kind == PartitionTableKind::Recovered`
///    from any hits found and return it (may still be empty on blank/unformatted media).
pub fn read_partition_table_with_fallback(device: &dyn BlockDevice) -> Result<PartitionTable> {
    // A missing/invalid MBR or GPT is treated the same as an empty table: fall
    // through to the signature scan rather than returning an error.
    if let Ok(tbl) = read_partition_table(device) {
        if !tbl.is_empty() {
            return Ok(tbl);
        }
    }
    // No real partition table — scan first 2 GiB at 1 MiB alignment.
    const TWO_GIB: u64 = 2 * 1024 * 1024 * 1024;
    let opts = ScanOptions {
        step: 1 << 20, // 1 MiB
        start_byte: 0,
        end_byte: Some(TWO_GIB.min(device.size())),
    };
    let hits = scan(device, &opts)?;
    let disk_lba = device.size() / device.sector_size() as u64;
    Ok(from_scan_hits(&hits, disk_lba, device.sector_size()))
}

/// Read the partition table from a block device.
///
/// Automatically detects MBR vs. GPT (via protective MBR check) and returns
/// the parsed table. Returns an empty `PartitionTable` with
/// [`PartitionTableKind::Mbr`] when the device is too small to hold a
/// partition table.
///
/// For GPT, only the primary header is used. If you suspect the primary is
/// corrupt, scan the device with [`scan`] and reconstruct via [`from_scan_hits`].
pub fn read_partition_table(device: &dyn BlockDevice) -> Result<PartitionTable> {
    let sector_size = device.sector_size() as usize;
    let disk_size_lba = device.size() / device.sector_size() as u64;

    // ── Read LBA 0 ────────────────────────────────────────────────────────────
    let mut buf = AlignedBuffer::new(sector_size, sector_size);
    device.read_at(0, &mut buf)?;
    let lba0 = buf.as_slice()[..sector_size].to_vec();

    // ── Choose MBR or GPT ────────────────────────────────────────────────────
    if mbr::is_protective_mbr(&lba0) {
        read_gpt(device, disk_size_lba, sector_size)
    } else {
        mbr::parse(&lba0, disk_size_lba, device.sector_size())
    }
}

fn read_gpt(
    device: &dyn BlockDevice,
    disk_size_lba: u64,
    sector_size: usize,
) -> Result<PartitionTable> {
    // ── Read GPT header (LBA 1) ───────────────────────────────────────────────
    let mut buf = AlignedBuffer::new(sector_size, sector_size);
    device.read_at(device.sector_size() as u64, &mut buf)?;
    let header = buf.as_slice()[..sector_size].to_vec();

    // Extract entry metadata before validation so we can allocate the right buffer.
    // These are read again (with full validation) inside gpt::parse.
    if header.len() < 92 {
        return Err(PartitionError::BufferTooSmall {
            needed: 92,
            got: header.len(),
        });
    }

    let num_entries = u32::from_le_bytes(header[80..84].try_into().unwrap_or([0; 4])) as usize;
    let entry_size = u32::from_le_bytes(header[84..88].try_into().unwrap_or([0; 4])) as usize;
    let entry_start_lba = u64::from_le_bytes(header[72..80].try_into().unwrap_or([0; 8]));

    // Guard against nonsensical values before allocating.
    let array_bytes = num_entries.saturating_mul(entry_size);
    let sectors_needed = array_bytes.div_ceil(sector_size).max(1);
    let buf_size = sectors_needed * sector_size;

    // ── Read partition entries ────────────────────────────────────────────────
    let mut entries_buf = AlignedBuffer::new(buf_size, sector_size);
    device.read_at(
        entry_start_lba * device.sector_size() as u64,
        &mut entries_buf,
    )?;
    let entries_data = entries_buf.as_slice()[..buf_size].to_vec();

    gpt::parse(&header, &entries_data, disk_size_lba, device.sector_size())
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
    fn fallback_empty_device_returns_empty_recovered_table() {
        // All zeros — no MBR signature, no filesystem magic. Should return a
        // Recovered table with zero entries (not an error).
        let sectors = (2 * 1024 * 1024 / SECTOR) + 4; // just over 2 MiB
        let dev = make_device(sectors, |_| {});
        let tbl = read_partition_table_with_fallback(&dev).unwrap();
        assert_eq!(tbl.kind, PartitionTableKind::Recovered);
        assert!(tbl.entries.is_empty());
    }

    #[test]
    fn fallback_finds_ntfs_when_no_partition_table() {
        // No MBR boot signature; NTFS VBR placed at 1 MiB offset.
        let one_mib = 1024 * 1024usize;
        let sectors = (one_mib + 4 * SECTOR) / SECTOR;
        let dev = make_device(sectors, |d| {
            // NTFS OEM ID at 1 MiB + 3 bytes
            d[one_mib + 3..one_mib + 11].copy_from_slice(b"NTFS    ");
        });
        let tbl = read_partition_table_with_fallback(&dev).unwrap();
        assert_eq!(tbl.kind, PartitionTableKind::Recovered);
        assert_eq!(tbl.entries.len(), 1);
        assert_eq!(
            tbl.entries[0].kind,
            PartitionKind::Recovered {
                fs_type: FsType::Ntfs
            }
        );
    }

    #[test]
    fn fallback_passes_through_valid_mbr_table() {
        // Build a minimal MBR with one FAT32 entry so read_partition_table()
        // succeeds and the fallback short-circuits at the "not empty" check.
        let sectors = 64;
        let dev = make_device(sectors, |d| {
            // MBR boot signature
            d[510] = 0x55;
            d[511] = 0xAA;
            // Partition entry 0: type=0x0B (FAT32), LBA start=2, size=10
            d[446 + 4] = 0x0B;
            d[446 + 8] = 2; // start LBA LE u32
            d[446 + 12] = 10; // size LBA LE u32
        });
        let tbl = read_partition_table_with_fallback(&dev).unwrap();
        // Should be MBR kind — fallback was not triggered.
        assert_eq!(tbl.kind, PartitionTableKind::Mbr);
        assert!(!tbl.entries.is_empty());
    }
}
