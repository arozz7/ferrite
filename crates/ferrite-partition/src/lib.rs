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
