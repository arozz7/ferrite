use uuid::Uuid;

/// Filesystem type detected by magic-byte scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    Ntfs,
    Fat16,
    Fat32,
    Ext4,
}

impl std::fmt::Display for FsType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ntfs => write!(f, "NTFS"),
            Self::Fat16 => write!(f, "FAT16"),
            Self::Fat32 => write!(f, "FAT32"),
            Self::Ext4 => write!(f, "ext4"),
        }
    }
}

/// How a partition entry was discovered.
#[derive(Debug, Clone, PartialEq)]
pub enum PartitionKind {
    /// MBR primary entry with its raw partition-type byte.
    Mbr { partition_type: u8 },
    /// GPT entry with type GUID and unique partition GUID.
    Gpt { type_guid: Uuid, part_guid: Uuid },
    /// Inferred from a raw filesystem-signature scan (no partition table present).
    Recovered { fs_type: FsType },
}

/// A single partition entry.
#[derive(Debug, Clone)]
pub struct PartitionEntry {
    /// Zero-based index within the source table or scan result.
    pub index: u32,
    /// First LBA of the partition.
    pub start_lba: u64,
    /// Last LBA of the partition (inclusive).
    pub end_lba: u64,
    /// Number of sectors.
    pub size_lba: u64,
    /// Human-readable name (GPT only; `None` for MBR / recovered).
    pub name: Option<String>,
    /// Discovery source.
    pub kind: PartitionKind,
    /// True for MBR bootable partitions (status byte `0x80`).
    pub bootable: bool,
}

impl PartitionEntry {
    /// Byte offset of the first sector on disk.
    pub fn start_byte(&self, sector_size: u32) -> u64 {
        self.start_lba * sector_size as u64
    }

    /// Total size in bytes.
    pub fn size_bytes(&self, sector_size: u32) -> u64 {
        self.size_lba * sector_size as u64
    }
}

/// Source format of a `PartitionTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionTableKind {
    Mbr,
    Gpt,
    /// Reconstructed from a filesystem scan — no real table was present.
    Recovered,
}

/// A parsed or reconstructed partition table.
#[derive(Debug, Clone)]
pub struct PartitionTable {
    pub kind: PartitionTableKind,
    pub sector_size: u32,
    pub disk_size_lba: u64,
    pub entries: Vec<PartitionEntry>,
}

impl PartitionTable {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A filesystem magic-byte match found during a raw-device scan.
#[derive(Debug, Clone)]
pub struct FsSignatureHit {
    /// Byte offset on the device where the filesystem VBR / superblock begins.
    pub offset_bytes: u64,
    pub fs_type: FsType,
}
