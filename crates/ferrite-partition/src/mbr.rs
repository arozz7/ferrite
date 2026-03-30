/// MBR (Master Boot Record) partition table parser.
///
/// The MBR occupies the first 512 bytes of a disk:
///   - Bytes   0–445: Bootstrap code (ignored)
///   - Bytes 446–509: Four 16-byte partition entries
///   - Bytes 510–511: Boot signature `0x55 0xAA`
///
/// Each 16-byte entry:
///   - Byte  0     : Status (`0x80` = active/bootable)
///   - Bytes 1–3   : CHS of first sector (ignored)
///   - Byte  4     : Partition type
///   - Bytes 5–7   : CHS of last sector (ignored)
///   - Bytes 8–11  : LBA start (u32 LE)
///   - Bytes 12–15 : LBA size  (u32 LE)
use byteorder::{ByteOrder, LittleEndian};

use crate::error::{PartitionError, Result};
use crate::types::{PartitionEntry, PartitionKind, PartitionTable, PartitionTableKind};

const MBR_SIZE: usize = 512;
const BOOT_SIG: [u8; 2] = [0x55, 0xAA];
const BOOT_SIG_OFFSET: usize = 510;
const TABLE_OFFSET: usize = 446;
const ENTRY_SIZE: usize = 16;
const NUM_ENTRIES: usize = 4;

/// Parse a raw 512-byte MBR sector into a [`PartitionTable`].
///
/// `disk_size_lba` is informational — the caller supplies it from the device.
pub fn parse(data: &[u8], disk_size_lba: u64, sector_size: u32) -> Result<PartitionTable> {
    if data.len() < MBR_SIZE {
        return Err(PartitionError::BufferTooSmall {
            needed: MBR_SIZE,
            got: data.len(),
        });
    }

    let sig = &data[BOOT_SIG_OFFSET..BOOT_SIG_OFFSET + 2];
    if sig != BOOT_SIG {
        return Err(PartitionError::InvalidSignature {
            context: "MBR",
            expected: "55 AA",
            found: sig.to_vec(),
        });
    }

    let mut entries = Vec::new();
    for i in 0..NUM_ENTRIES {
        let base = TABLE_OFFSET + i * ENTRY_SIZE;
        let e = &data[base..base + ENTRY_SIZE];

        let status = e[0];
        let partition_type = e[4];

        // Empty slot
        if partition_type == 0x00 {
            continue;
        }

        let start_lba = LittleEndian::read_u32(&e[8..12]) as u64;
        let size_lba = LittleEndian::read_u32(&e[12..16]) as u64;

        if start_lba == 0 || size_lba == 0 {
            continue;
        }

        entries.push(PartitionEntry {
            index: i as u32,
            start_lba,
            end_lba: start_lba + size_lba - 1,
            size_lba,
            name: None,
            kind: PartitionKind::Mbr { partition_type },
            bootable: status == 0x80,
        });
    }

    Ok(PartitionTable {
        kind: PartitionTableKind::Mbr,
        sector_size,
        disk_size_lba,
        entries,
        note: None,
    })
}

/// Returns `true` if the given MBR sector contains a GPT protective entry
/// (partition type `0xEE` in the first slot).
pub fn is_protective_mbr(data: &[u8]) -> bool {
    data.len() >= MBR_SIZE && data[TABLE_OFFSET + 4] == 0xEE
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal MBR sector with one partition entry at slot `slot`.
    fn make_mbr(slot: usize, status: u8, ptype: u8, start_lba: u32, size_lba: u32) -> [u8; 512] {
        let mut mbr = [0u8; 512];
        // Boot signature
        mbr[510] = 0x55;
        mbr[511] = 0xAA;

        let base = TABLE_OFFSET + slot * ENTRY_SIZE;
        mbr[base] = status;
        mbr[base + 4] = ptype;
        LittleEndian::write_u32(&mut mbr[base + 8..base + 12], start_lba);
        LittleEndian::write_u32(&mut mbr[base + 12..base + 16], size_lba);
        mbr
    }

    #[test]
    fn parses_single_entry() {
        let mbr = make_mbr(0, 0x80, 0x07, 2048, 204800);
        let table = parse(&mbr, 409600, 512).unwrap();
        assert_eq!(table.kind, PartitionTableKind::Mbr);
        assert_eq!(table.entries.len(), 1);

        let e = &table.entries[0];
        assert_eq!(e.start_lba, 2048);
        assert_eq!(e.size_lba, 204800);
        assert_eq!(e.end_lba, 2048 + 204800 - 1);
        assert!(e.bootable);
        assert_eq!(
            e.kind,
            PartitionKind::Mbr {
                partition_type: 0x07
            }
        );
    }

    #[test]
    fn skips_empty_entries() {
        let mbr = make_mbr(1, 0x00, 0x83, 2048, 204800); // slot 0 is empty (type 0x00)
        let table = parse(&mbr, 409600, 512).unwrap();
        // Slot 0 is empty; slot 1 has Linux partition
        assert_eq!(table.entries.len(), 1);
        assert_eq!(table.entries[0].index, 1);
    }

    #[test]
    fn non_bootable_entry() {
        let mbr = make_mbr(0, 0x00, 0x83, 2048, 204800);
        let table = parse(&mbr, 409600, 512).unwrap();
        assert!(!table.entries[0].bootable);
    }

    #[test]
    fn invalid_signature_returns_error() {
        let mut mbr = make_mbr(0, 0x80, 0x07, 2048, 204800);
        mbr[510] = 0x00; // corrupt signature
        let err = parse(&mbr, 409600, 512).unwrap_err();
        assert!(matches!(
            err,
            PartitionError::InvalidSignature { context: "MBR", .. }
        ));
    }

    #[test]
    fn buffer_too_small_returns_error() {
        let err = parse(&[0u8; 256], 0, 512).unwrap_err();
        assert!(matches!(
            err,
            PartitionError::BufferTooSmall { needed: 512, .. }
        ));
    }

    #[test]
    fn start_byte_and_size_bytes() {
        let mbr = make_mbr(0, 0x80, 0x07, 2048, 204800);
        let table = parse(&mbr, 409600, 512).unwrap();
        let e = &table.entries[0];
        assert_eq!(e.start_byte(512), 2048 * 512);
        assert_eq!(e.size_bytes(512), 204800 * 512);
    }

    #[test]
    fn protective_mbr_detected() {
        let mbr = make_mbr(0, 0x00, 0xEE, 1, u32::MAX);
        assert!(is_protective_mbr(&mbr));
    }

    #[test]
    fn non_protective_mbr() {
        let mbr = make_mbr(0, 0x80, 0x07, 2048, 204800);
        assert!(!is_protective_mbr(&mbr));
    }
}
