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
/// For GPT, the primary header (LBA 1) is tried first.  If it is unreadable
/// or fails CRC checks, the backup header at the last LBA is tried
/// automatically and a [`PartitionTable::note`] is set to inform the caller.
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

/// Try reading a GPT header from the given `header_lba` and its associated
/// partition entry array.  Returns the parsed table on success.
fn try_gpt_at(
    device: &dyn BlockDevice,
    header_lba: u64,
    disk_size_lba: u64,
    sector_size: usize,
) -> Result<PartitionTable> {
    let mut buf = AlignedBuffer::new(sector_size, sector_size);
    device.read_at(header_lba * device.sector_size() as u64, &mut buf)?;
    let header = buf.as_slice()[..sector_size].to_vec();

    if header.len() < 92 {
        return Err(PartitionError::BufferTooSmall {
            needed: 92,
            got: header.len(),
        });
    }

    let num_entries = u32::from_le_bytes(header[80..84].try_into().unwrap_or([0; 4])) as usize;
    let entry_size = u32::from_le_bytes(header[84..88].try_into().unwrap_or([0; 4])) as usize;
    let entry_start_lba = u64::from_le_bytes(header[72..80].try_into().unwrap_or([0; 8]));

    let array_bytes = num_entries.saturating_mul(entry_size);
    let sectors_needed = array_bytes.div_ceil(sector_size).max(1);
    let buf_size = sectors_needed * sector_size;

    let mut entries_buf = AlignedBuffer::new(buf_size, sector_size);
    device.read_at(
        entry_start_lba * device.sector_size() as u64,
        &mut entries_buf,
    )?;
    let entries_data = entries_buf.as_slice()[..buf_size].to_vec();

    gpt::parse(&header, &entries_data, disk_size_lba, device.sector_size())
}

fn read_gpt(
    device: &dyn BlockDevice,
    disk_size_lba: u64,
    sector_size: usize,
) -> Result<PartitionTable> {
    // Try primary header at LBA 1.
    if let Ok(tbl) = try_gpt_at(device, 1, disk_size_lba, sector_size) {
        return Ok(tbl);
    }

    // Primary unreadable or corrupt — fall back to backup header at the last LBA.
    let backup_lba = disk_size_lba.saturating_sub(1);
    if backup_lba < 2 {
        return Err(PartitionError::InvalidGptHeader(
            "device too small to contain a GPT backup header".to_string(),
        ));
    }
    let mut tbl = try_gpt_at(device, backup_lba, disk_size_lba, sector_size)?;
    tbl.note = Some(
        "GPT primary header unreadable — partition table read from backup header at last LBA"
            .to_string(),
    );
    Ok(tbl)
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

    /// Build a minimal valid GPT header at `header_lba` with no partition entries,
    /// pointing its entry array at `entry_array_lba`.  Returns 512 bytes.
    fn build_gpt_header(header_lba: u64, entry_array_lba: u64, disk_size_lba: u64) -> [u8; 512] {
        use byteorder::{ByteOrder, LittleEndian};
        let mut h = [0u8; 512];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&[0x00, 0x00, 0x01, 0x00]); // revision 1.0
        LittleEndian::write_u32(&mut h[12..16], 92); // header size
        LittleEndian::write_u64(&mut h[24..32], header_lba); // MyLBA
        LittleEndian::write_u64(
            &mut h[32..40],
            if header_lba == 1 {
                disk_size_lba - 1
            } else {
                1
            },
        );
        LittleEndian::write_u64(&mut h[40..48], 34); // FirstUsable
        LittleEndian::write_u64(&mut h[48..56], disk_size_lba.saturating_sub(34)); // LastUsable
        LittleEndian::write_u64(&mut h[72..80], entry_array_lba); // entry start LBA
        LittleEndian::write_u32(&mut h[80..84], 0); // num entries
        LittleEndian::write_u32(&mut h[84..88], 128); // entry size
        LittleEndian::write_u32(&mut h[88..92], 0); // array CRC (empty = 0)
        let crc = crc32fast::hash(&h[..92]);
        LittleEndian::write_u32(&mut h[16..20], crc);
        h
    }

    #[test]
    fn gpt_backup_header_used_when_primary_corrupt() {
        // Build a device where LBA 1 (primary GPT header) is zeroed/corrupt but
        // the backup header at the last LBA is valid.
        //
        // Layout (32 sectors, 512 B each = 16 KiB):
        //   LBA 0   : protective MBR
        //   LBA 1   : corrupt primary header (all zeros)
        //   LBA 2   : entry array for backup header (empty, 1 sector)
        //   LBA 31  : valid backup GPT header
        let total_sectors = 32usize;
        let disk_size_lba = total_sectors as u64;
        let dev = make_device(total_sectors, |d| {
            // Protective MBR at LBA 0: type 0xEE in slot 0.
            d[446 + 4] = 0xEE;
            d[510] = 0x55;
            d[511] = 0xAA;
            // LBA 1 stays all-zeros (corrupt primary header).
            // Backup header at LBA 31, entry array at LBA 2.
            let backup = build_gpt_header(31, 2, disk_size_lba);
            let backup_off = 31 * SECTOR;
            d[backup_off..backup_off + SECTOR].copy_from_slice(&backup);
        });
        let tbl = read_partition_table(&dev).unwrap();
        assert_eq!(tbl.kind, PartitionTableKind::Gpt);
        assert!(
            tbl.note.as_deref().unwrap_or("").contains("backup header"),
            "expected backup-header note, got: {:?}",
            tbl.note
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
