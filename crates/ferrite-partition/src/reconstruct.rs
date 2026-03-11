/// Reconstruct a `PartitionTable` from raw filesystem signature hits.
///
/// When a disk has no readable partition table (corrupt MBR, wiped GPT),
/// the scanner can locate filesystem VBRs / superblocks. This module turns
/// those positional hits into a best-effort `PartitionTable` so downstream
/// code can treat recovered partitions uniformly with real ones.
///
/// Each hit becomes one `PartitionEntry` with:
///   - `kind` = `PartitionKind::Recovered { fs_type }`
///   - LBA boundaries estimated from adjacent hits (last entry extends to disk end)
///   - Sequential `index` values
use crate::types::{
    FsSignatureHit, PartitionEntry, PartitionKind, PartitionTable, PartitionTableKind,
};

/// Build a [`PartitionTable`] from a list of filesystem-signature hits.
///
/// `hits` should be sorted by `offset_bytes` (ascending); unsorted input
/// still works but produces less useful LBA estimates.
pub fn from_scan_hits(
    hits: &[FsSignatureHit],
    disk_size_lba: u64,
    sector_size: u32,
) -> PartitionTable {
    let mut entries = Vec::with_capacity(hits.len());

    for (i, hit) in hits.iter().enumerate() {
        let start_lba = hit.offset_bytes / sector_size as u64;

        // End LBA = either start of next partition or end of disk
        let end_lba = hits
            .get(i + 1)
            .map(|next| next.offset_bytes / sector_size as u64)
            .unwrap_or(disk_size_lba)
            .saturating_sub(1);

        let size_lba = if end_lba >= start_lba {
            end_lba - start_lba + 1
        } else {
            1
        };

        entries.push(PartitionEntry {
            index: i as u32,
            start_lba,
            end_lba,
            size_lba,
            name: Some(format!("Recovered {} partition", hit.fs_type)),
            kind: PartitionKind::Recovered {
                fs_type: hit.fs_type,
            },
            bootable: false,
        });
    }

    PartitionTable {
        kind: PartitionTableKind::Recovered,
        sector_size,
        disk_size_lba,
        entries,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FsType;

    fn hit(offset_bytes: u64, fs_type: FsType) -> FsSignatureHit {
        FsSignatureHit {
            offset_bytes,
            fs_type,
        }
    }

    const SECTOR: u32 = 512;

    #[test]
    fn empty_hits_produces_empty_table() {
        let table = from_scan_hits(&[], 2048, SECTOR);
        assert_eq!(table.kind, PartitionTableKind::Recovered);
        assert!(table.entries.is_empty());
    }

    #[test]
    fn single_hit_extends_to_disk_end() {
        // Partition at LBA 2048, disk is 409600 LBA
        let hits = [hit(2048 * SECTOR as u64, FsType::Ntfs)];
        let table = from_scan_hits(&hits, 409600, SECTOR);
        assert_eq!(table.entries.len(), 1);

        let e = &table.entries[0];
        assert_eq!(e.start_lba, 2048);
        assert_eq!(e.end_lba, 409600 - 1);
        assert_eq!(e.size_lba, 409600 - 2048);
        assert_eq!(
            e.kind,
            PartitionKind::Recovered {
                fs_type: FsType::Ntfs
            }
        );
        assert_eq!(e.name.as_deref(), Some("Recovered NTFS partition"));
    }

    #[test]
    fn multiple_hits_assign_correct_boundaries() {
        let hits = [
            hit(2048 * SECTOR as u64, FsType::Ntfs),
            hit(206848 * SECTOR as u64, FsType::Fat32),
            hit(411648 * SECTOR as u64, FsType::Ext4),
        ];
        let disk_lba = 614400u64;
        let table = from_scan_hits(&hits, disk_lba, SECTOR);
        assert_eq!(table.entries.len(), 3);

        // First partition: LBA 2048 → 206847 (one before the next)
        assert_eq!(table.entries[0].start_lba, 2048);
        assert_eq!(table.entries[0].end_lba, 206848 - 1);

        // Second partition: LBA 206848 → 411647
        assert_eq!(table.entries[1].start_lba, 206848);
        assert_eq!(table.entries[1].end_lba, 411648 - 1);

        // Third partition: LBA 411648 → disk end - 1
        assert_eq!(table.entries[2].start_lba, 411648);
        assert_eq!(table.entries[2].end_lba, disk_lba - 1);
    }

    #[test]
    fn sequential_indices_assigned() {
        let hits = [
            hit(0, FsType::Ntfs),
            hit(204800 * SECTOR as u64, FsType::Ext4),
        ];
        let table = from_scan_hits(&hits, 409600, SECTOR);
        assert_eq!(table.entries[0].index, 0);
        assert_eq!(table.entries[1].index, 1);
    }

    #[test]
    fn recovered_entries_are_not_bootable() {
        let hits = [hit(0, FsType::Ntfs)];
        let table = from_scan_hits(&hits, 1024, SECTOR);
        assert!(!table.entries[0].bootable);
    }
}
