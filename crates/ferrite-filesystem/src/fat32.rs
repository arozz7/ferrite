//! Minimum-viable FAT32 read-only parser.
//!
//! Supports 8.3 and long-filename (LFN) directory entries, cluster-chain
//! traversal, deleted-file detection, and sequential file reads.

use std::io::Write;
use std::sync::Arc;

use tracing::trace;

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::{read_bytes, read_u16_le, read_u32_le};
use crate::{FileEntry, FilesystemParser, FilesystemType, RecoveryChance};

// ── Constants ─────────────────────────────────────────────────────────────────

const FAT32_TYPE_STRING: &[u8] = b"FAT32   ";
const ATTR_LONG_NAME: u8 = 0x0F;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_VOLUME_LABEL: u8 = 0x08;
const DELETED_MARKER: u8 = 0xE5;
const END_OF_DIR: u8 = 0x00;
const EOC_MIN: u32 = 0x0FFF_FFF8;

// ── Public struct ─────────────────────────────────────────────────────────────

/// Read-only FAT32 filesystem parser.
pub struct Fat32Parser {
    device: Arc<dyn BlockDevice>,
    fat_offset: u64,   // byte offset to FAT region
    data_offset: u64,  // byte offset to data region (cluster 2 starts here)
    cluster_size: u64, // bytes per cluster
    root_cluster: u32,
}

impl Fat32Parser {
    /// Parse the FAT32 BPB from `device` and return an initialised parser.
    pub fn new(device: Arc<dyn BlockDevice>) -> Result<Self> {
        let boot = read_bytes(device.as_ref(), 0, 512)?;

        // Boot signature
        if boot[510] != 0x55 || boot[511] != 0xAA {
            return Err(FilesystemError::InvalidStructure {
                context: "FAT32 boot sector",
                reason: "boot signature 0x55AA not found".to_string(),
            });
        }

        // FAT32 type string at offset 82
        if &boot[82..90] != FAT32_TYPE_STRING {
            return Err(FilesystemError::InvalidStructure {
                context: "FAT32 boot sector",
                reason: "type string \"FAT32   \" not found at offset 82".to_string(),
            });
        }

        let bytes_per_sector = read_u16_le(&boot, 11)? as u64;
        let sectors_per_cluster = boot[13] as u64;
        let reserved_sectors = read_u16_le(&boot, 14)? as u64;
        let num_fats = boot[16] as u64;
        let fat_size = read_u32_le(&boot, 36)? as u64;
        let root_cluster = read_u32_le(&boot, 44)?;

        if bytes_per_sector == 0 || sectors_per_cluster == 0 {
            return Err(FilesystemError::InvalidStructure {
                context: "FAT32 BPB",
                reason: "bytes_per_sector or sectors_per_cluster is zero".to_string(),
            });
        }

        let fat_offset = reserved_sectors * bytes_per_sector;
        let data_offset = fat_offset + num_fats * fat_size * bytes_per_sector;
        let cluster_size = sectors_per_cluster * bytes_per_sector;

        trace!(
            fat_offset,
            data_offset,
            cluster_size,
            root_cluster,
            "FAT32 parser initialised"
        );

        Ok(Self {
            device,
            fat_offset,
            data_offset,
            cluster_size,
            root_cluster,
        })
    }

    // ── Cluster helpers ───────────────────────────────────────────────────────

    fn cluster_offset(&self, cluster: u32) -> u64 {
        debug_assert!(cluster >= 2, "cluster index must be ≥ 2 per FAT spec");
        self.data_offset + (cluster as u64 - 2) * self.cluster_size
    }

    fn read_fat_entry(&self, cluster: u32) -> Result<u32> {
        let offset = self.fat_offset + cluster as u64 * 4;
        let raw = read_bytes(self.device.as_ref(), offset, 4)?;
        // Safety: read_bytes(offset, 4)? returns exactly 4 bytes.
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) & 0x0FFF_FFFF)
    }

    fn is_eoc(entry: u32) -> bool {
        entry >= EOC_MIN
    }

    fn read_cluster(&self, cluster: u32) -> Result<Vec<u8>> {
        if cluster < 2 {
            return Err(FilesystemError::InvalidStructure {
                context: "FAT32 cluster chain",
                reason: format!("reserved cluster index {cluster} — must be ≥ 2 per FAT spec"),
            });
        }
        read_bytes(
            self.device.as_ref(),
            self.cluster_offset(cluster),
            self.cluster_size as usize,
        )
    }

    // ── Directory helpers ─────────────────────────────────────────────────────

    /// Collect all raw 32-byte directory entries from the cluster chain
    /// beginning at `start_cluster`, stopping at the end-of-directory marker.
    fn raw_dir_entries(&self, start_cluster: u32) -> Result<Vec<[u8; 32]>> {
        let mut out = Vec::new();
        let mut cluster = start_cluster;
        loop {
            let data = self.read_cluster(cluster)?;
            for chunk in data.chunks_exact(32) {
                let mut entry = [0u8; 32];
                entry.copy_from_slice(chunk);
                if entry[0] == END_OF_DIR {
                    return Ok(out);
                }
                out.push(entry);
            }
            let next = self.read_fat_entry(cluster)?;
            if Self::is_eoc(next) {
                break;
            }
            cluster = next;
        }
        Ok(out)
    }

    /// Convert raw 32-byte entries (including LFN records) to [`FileEntry`]s.
    ///
    /// When `include_deleted` is `false`, entries whose first byte is
    /// `0xE5` are skipped.
    fn build_entries(&self, raw: &[[u8; 32]], include_deleted: bool) -> Vec<FileEntry> {
        let mut result = Vec::new();
        // LFN parts accumulated before the corresponding 8.3 entry.
        // Each element is (sequence_number, assembled_string).
        let mut lfn_parts: Vec<(u8, String)> = Vec::new();

        for entry in raw {
            let first = entry[0];
            let attr = entry[11];

            // ── LFN entry ────────────────────────────────────────────────────
            if attr == ATTR_LONG_NAME {
                let seq = first & 0x1F;
                lfn_parts.push((seq, lfn_name_part(entry)));
                continue;
            }

            // ── Volume label — skip ──────────────────────────────────────────
            if attr & ATTR_VOLUME_LABEL != 0 && attr & ATTR_DIRECTORY == 0 {
                lfn_parts.clear();
                continue;
            }

            // ── Deleted entry ─────────────────────────────────────────────────
            let is_deleted = first == DELETED_MARKER;
            if is_deleted && !include_deleted {
                lfn_parts.clear();
                continue;
            }

            // ── Dot / dot-dot — skip ─────────────────────────────────────────
            let short_name = build_short_name(entry, is_deleted);
            if short_name == "." || short_name == ".." {
                lfn_parts.clear();
                continue;
            }

            // ── Assemble name ─────────────────────────────────────────────────
            let name = if !lfn_parts.is_empty() {
                lfn_parts.sort_by_key(|(s, _)| *s);
                let assembled: String = lfn_parts
                    .iter()
                    .flat_map(|(_, s)| s.chars())
                    .collect::<String>()
                    .trim_end_matches('\0')
                    .to_string();
                assembled
            } else {
                short_name
            };
            lfn_parts.clear();

            let is_dir = attr & ATTR_DIRECTORY != 0;
            // Safety: entry is [u8; 32], all indices are in bounds.
            let cluster_hi = u16::from_le_bytes([entry[20], entry[21]]) as u32;
            let cluster_lo = u16::from_le_bytes([entry[26], entry[27]]) as u32;
            let first_cluster = (cluster_hi << 16) | cluster_lo;
            let size = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]) as u64;

            let data_byte_offset = if is_dir || first_cluster < 2 {
                None
            } else {
                Some(self.data_offset + (first_cluster as u64 - 2) * self.cluster_size)
            };
            // Safety: entry is [u8; 32], so all index accesses below are in bounds.
            let crt_time = u16::from_le_bytes([entry[14], entry[15]]);
            let crt_date = u16::from_le_bytes([entry[16], entry[17]]);
            let wrt_time = u16::from_le_bytes([entry[22], entry[23]]);
            let wrt_date = u16::from_le_bytes([entry[24], entry[25]]);
            result.push(FileEntry {
                name: name.clone(),
                path: format!("/{name}"),
                size,
                is_dir,
                is_deleted,
                created: fat_datetime_to_unix(crt_date, crt_time),
                modified: fat_datetime_to_unix(wrt_date, wrt_time),
                first_cluster: Some(first_cluster),
                mft_record: None,
                inode_number: None,
                data_byte_offset,
                recovery_chance: RecoveryChance::Unknown,
            });
        }
        result
    }

    /// Navigate the directory tree and return the starting cluster of the
    /// directory at `path`.
    fn resolve_dir_cluster(&self, path: &str) -> Result<u32> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut cluster = self.root_cluster;
        for part in &parts {
            let raw = self.raw_dir_entries(cluster)?;
            let entries = self.build_entries(&raw, false);
            let found = entries
                .iter()
                .find(|e| e.is_dir && e.name.eq_ignore_ascii_case(part));
            match found {
                Some(e) => cluster = e.first_cluster.unwrap_or(self.root_cluster),
                None => return Err(FilesystemError::NotFound(path.to_string())),
            }
        }
        Ok(cluster)
    }

    /// Follow `cluster`'s chain and write up to `file_size` bytes to `writer`.
    fn read_chain(
        &self,
        start_cluster: u32,
        file_size: u64,
        writer: &mut dyn Write,
    ) -> Result<u64> {
        let mut written = 0u64;
        let mut cluster = start_cluster;
        loop {
            let data = self.read_cluster(cluster)?;
            let remaining = file_size - written;
            let to_write = (data.len() as u64).min(remaining) as usize;
            writer
                .write_all(&data[..to_write])
                .map_err(|e| FilesystemError::InvalidStructure {
                    context: "read_file write",
                    reason: e.to_string(),
                })?;
            written += to_write as u64;
            if written >= file_size {
                break;
            }
            let next = self.read_fat_entry(cluster)?;
            if Self::is_eoc(next) {
                break;
            }
            cluster = next;
        }
        Ok(written)
    }
}

// ── FilesystemParser impl ─────────────────────────────────────────────────────

impl FilesystemParser for Fat32Parser {
    fn filesystem_type(&self) -> FilesystemType {
        FilesystemType::Fat32
    }

    fn root_directory(&self) -> Result<Vec<FileEntry>> {
        let raw = self.raw_dir_entries(self.root_cluster)?;
        Ok(self.build_entries(&raw, false))
    }

    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>> {
        let cluster = self.resolve_dir_cluster(path)?;
        let raw = self.raw_dir_entries(cluster)?;
        Ok(self.build_entries(&raw, false))
    }

    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64> {
        let cluster = entry
            .first_cluster
            .ok_or(FilesystemError::InvalidStructure {
                context: "read_file",
                reason: "FileEntry has no starting cluster".to_string(),
            })?;
        self.read_chain(cluster, entry.size, writer)
    }

    fn deleted_files(&self) -> Result<Vec<FileEntry>> {
        use std::collections::{HashSet, VecDeque};

        let mut result = Vec::new();
        // visited guards against corrupt FAT cycles (same cluster reachable via two paths).
        let mut visited: HashSet<u32> = HashSet::new();
        // Queue entries: (dir_start_cluster, path_prefix_for_entries_inside_this_dir).
        // Root entries use an empty prefix so paths come out as "/name".
        let mut queue: VecDeque<(u32, String)> = VecDeque::new();
        queue.push_back((self.root_cluster, String::new()));

        while let Some((cluster, dir_prefix)) = queue.pop_front() {
            if !visited.insert(cluster) {
                continue; // already walked this cluster — cycle or alias
            }
            let raw = match self.raw_dir_entries(cluster) {
                Ok(r) => r,
                Err(_) => continue, // damaged / unreadable cluster — skip, keep going
            };
            for mut entry in self.build_entries(&raw, true) {
                // build_entries emits "/<name>"; replace with the full tree path.
                let full_path = format!("{}/{}", dir_prefix, entry.name);
                entry.path = full_path.clone();

                if entry.is_deleted && !entry.is_dir {
                    entry.recovery_chance = if entry.size == 0 {
                        RecoveryChance::Unknown
                    } else if entry.first_cluster.map(|c| c >= 2).unwrap_or(false) {
                        // Start cluster recorded in the dirent — cluster chain intact or partially so.
                        RecoveryChance::Medium
                    } else {
                        RecoveryChance::Low
                    };
                    result.push(entry);
                } else if !entry.is_deleted && entry.is_dir {
                    // Recurse into every live subdirectory.
                    // Deleted directories are skipped: their cluster may have been reused.
                    if let Some(sub_cluster) = entry.first_cluster {
                        if sub_cluster >= 2 {
                            queue.push_back((sub_cluster, full_path));
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

// ── String helpers ────────────────────────────────────────────────────────────

/// Convert a FAT32 date/time pair to a Unix timestamp (seconds since 1970-01-01 UTC).
///
/// FAT date encoding (16-bit big-endian logical):
///   bits 15-9 = year offset from 1980, bits 8-5 = month (1-12), bits 4-0 = day (1-31)
///
/// FAT time encoding (16-bit):
///   bits 15-11 = hours, bits 10-5 = minutes, bits 4-0 = seconds/2
///
/// Returns `None` when `date` is zero (field not set) or any value is out of range.
fn fat_datetime_to_unix(date: u16, time: u16) -> Option<u64> {
    if date == 0 {
        return None;
    }
    let year = 1980u32 + (date >> 9) as u32;
    let month = ((date >> 5) & 0x0F) as u32;
    let day = (date & 0x1F) as u32;
    if month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }
    let hour = (time >> 11) as u32;
    let min = ((time >> 5) & 0x3F) as u32;
    let sec = (time & 0x1F) as u32 * 2;
    if hour > 23 || min > 59 || sec > 59 {
        return None;
    }

    // Days from Unix epoch (1970-01-01) to FAT epoch (1980-01-01) = 3652.
    const FAT_EPOCH_DAYS: u64 = 3652;
    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let is_leap = |y: u32| (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400);

    let mut days: u64 = FAT_EPOCH_DAYS;
    for y in 1980..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    for m in 1..month {
        let mut d = DAYS_IN_MONTH[(m - 1) as usize];
        if m == 2 && is_leap(year) {
            d += 1;
        }
        days += d as u64;
    }
    days += (day - 1) as u64;

    Some(days * 86_400 + hour as u64 * 3_600 + min as u64 * 60 + sec as u64)
}

/// Build a printable 8.3 filename from a directory entry.
fn build_short_name(entry: &[u8; 32], is_deleted: bool) -> String {
    let base_raw = &entry[0..8];
    let ext_raw = &entry[8..11];

    let first = if is_deleted { b'?' } else { base_raw[0] };
    let base: String = std::iter::once(first)
        .chain(base_raw[1..].iter().copied())
        .map(|b| if b == 0 { b' ' } else { b } as char)
        .collect::<String>()
        .trim_end()
        .to_string();

    let ext: String = ext_raw
        .iter()
        .map(|&b| if b == 0 { b' ' } else { b } as char)
        .collect::<String>()
        .trim_end()
        .to_string();

    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// Extract the 13 UTF-16 characters stored in one LFN directory entry.
fn lfn_name_part(entry: &[u8; 32]) -> String {
    // Three discontiguous runs of UTF-16LE code units:
    // bytes  1-10  → 5 chars
    // bytes 14-25  → 6 chars
    // bytes 28-31  → 2 chars
    let ranges = [1..11usize, 14..26, 28..32];
    let mut chars = Vec::with_capacity(13);
    'outer: for range in ranges {
        for chunk in entry[range].chunks_exact(2) {
            let cp = u16::from_le_bytes([chunk[0], chunk[1]]);
            if cp == 0x0000 || cp == 0xFFFF {
                break 'outer;
            }
            chars.push(char::from_u32(cp as u32).unwrap_or('\u{FFFD}'));
        }
    }
    chars.into_iter().collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal FAT32 disk image in memory.
    ///
    /// Layout (512-byte sectors, sector_size = 512):
    ///  - Sector 0:   Boot sector (BPB)
    ///  - Sectors 1-3: Reserved (zeros)
    ///  - Sector 4:   FAT1
    ///  - Sector 5:   FAT2
    ///  - Sector 6:   Root directory cluster (cluster 2)
    ///  - Sector 7:   File data cluster     (cluster 3)
    fn build_image() -> MockBlockDevice {
        let mut dev = MockBlockDevice::zeroed(4096, 512);

        // ── Boot sector ───────────────────────────────────────────────────────
        let mut boot = [0u8; 512];
        boot[3..11].copy_from_slice(b"MSDOS5.0");
        boot[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes_per_sector
        boot[13] = 1; // sectors_per_cluster
        boot[14..16].copy_from_slice(&4u16.to_le_bytes()); // reserved_sectors
        boot[16] = 2; // num_fats
        boot[36..40].copy_from_slice(&1u32.to_le_bytes()); // fat_size_32
        boot[44..48].copy_from_slice(&2u32.to_le_bytes()); // root_cluster
        boot[82..90].copy_from_slice(b"FAT32   ");
        boot[510] = 0x55;
        boot[511] = 0xAA;
        dev.write_sector(0, &boot);

        // ── FAT (sectors 4 and 5) ─────────────────────────────────────────────
        let mut fat = [0u8; 512];
        // cluster 0: media byte
        fat[0..4].copy_from_slice(&0x0FFF_FFF8u32.to_le_bytes());
        // cluster 1: reserved
        fat[4..8].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        // cluster 2: end-of-chain (root directory)
        fat[8..12].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        // cluster 3: end-of-chain (file)
        fat[12..16].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        dev.write_sector(4, &fat);
        dev.write_sector(5, &fat);

        // ── Root directory (sector 6 = cluster 2) ────────────────────────────
        let mut dir = [0u8; 512];
        // Entry 0: "HELLO   TXT" (cluster 3, 13 bytes)
        dir[0..8].copy_from_slice(b"HELLO   ");
        dir[8..11].copy_from_slice(b"TXT");
        dir[11] = 0x20; // archive
        dir[20..22].copy_from_slice(&0u16.to_le_bytes()); // cluster_hi
        dir[26..28].copy_from_slice(&3u16.to_le_bytes()); // cluster_lo
        dir[28..32].copy_from_slice(&13u32.to_le_bytes()); // size
                                                           // Entry 1: deleted "GONE    DAT" (cluster 3 reused just for structure)
        dir[32] = DELETED_MARKER;
        dir[33..40].copy_from_slice(b"ONE    ");
        dir[40..43].copy_from_slice(b"DAT");
        dir[43] = 0x20;
        dir[52..54].copy_from_slice(&0u16.to_le_bytes());
        dir[58..60].copy_from_slice(&3u16.to_le_bytes());
        dir[60..64].copy_from_slice(&4u32.to_le_bytes());
        // Entry 2: end-of-directory
        dir[64] = 0x00;
        dev.write_sector(6, &dir);

        // ── File data (sector 7 = cluster 3) ─────────────────────────────────
        let mut content = [0u8; 512];
        content[..13].copy_from_slice(b"Hello, World!");
        dev.write_sector(7, &content);

        dev
    }

    #[test]
    fn detects_fat32() {
        let dev = Arc::new(build_image());
        let parser = Fat32Parser::new(dev).unwrap();
        assert_eq!(parser.filesystem_type(), FilesystemType::Fat32);
    }

    #[test]
    fn root_directory_lists_file() {
        let dev = Arc::new(build_image());
        let parser = Fat32Parser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.name, "HELLO.TXT");
        assert_eq!(e.size, 13);
        assert!(!e.is_dir);
        assert!(!e.is_deleted);
    }

    #[test]
    fn read_file_returns_content() {
        let dev = Arc::new(build_image());
        let parser = Fat32Parser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let mut buf = Vec::new();
        let written = parser.read_file(&entries[0], &mut buf).unwrap();
        assert_eq!(written, 13);
        assert_eq!(&buf, b"Hello, World!");
    }

    #[test]
    fn deleted_files_found() {
        let dev = Arc::new(build_image());
        let parser = Fat32Parser::new(dev).unwrap();
        let deleted = parser.deleted_files().unwrap();
        assert_eq!(deleted.len(), 1);
        assert!(deleted[0].is_deleted);
    }

    /// Build a FAT32 image that has a live subdirectory containing a deleted file.
    ///
    /// Layout (10 sectors × 512 bytes, cluster_size = 512):
    ///  - Sector 0:  Boot sector
    ///  - Sectors 1-3: Reserved
    ///  - Sector 4:  FAT1  (clusters 0-5)
    ///  - Sector 5:  FAT2
    ///  - Sector 6:  Root dir  (cluster 2): HELLO.TXT + GONE.DAT (deleted) + SUBDIR/
    ///  - Sector 7:  HELLO.TXT data (cluster 3)
    ///  - Sector 8:  SUBDIR contents (cluster 4): . + .. + SUBDEL.TXT (deleted)
    ///  - Sector 9:  SUBDEL.TXT data (cluster 5)
    fn build_subdir_image() -> MockBlockDevice {
        let mut dev = MockBlockDevice::zeroed(10 * 512, 512);

        // ── Boot sector ───────────────────────────────────────────────────────
        let mut boot = [0u8; 512];
        boot[3..11].copy_from_slice(b"MSDOS5.0");
        boot[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes_per_sector
        boot[13] = 1; // sectors_per_cluster
        boot[14..16].copy_from_slice(&4u16.to_le_bytes()); // reserved_sectors
        boot[16] = 2; // num_fats
        boot[36..40].copy_from_slice(&1u32.to_le_bytes()); // fat_size_32 (1 sector per FAT)
        boot[44..48].copy_from_slice(&2u32.to_le_bytes()); // root_cluster = 2
        boot[82..90].copy_from_slice(b"FAT32   ");
        boot[510] = 0x55;
        boot[511] = 0xAA;
        dev.write_sector(0, &boot);

        // ── FAT (sectors 4 and 5) — 6 clusters ───────────────────────────────
        let mut fat = [0u8; 512];
        fat[0..4].copy_from_slice(&0x0FFF_FFF8u32.to_le_bytes()); // cluster 0: media
        fat[4..8].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // cluster 1: reserved
        fat[8..12].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // cluster 2: root dir EOC
        fat[12..16].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // cluster 3: HELLO.TXT EOC
        fat[16..20].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // cluster 4: SUBDIR EOC
        fat[20..24].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // cluster 5: SUBDEL.TXT EOC
        dev.write_sector(4, &fat);
        dev.write_sector(5, &fat);

        // ── Root directory (sector 6 = cluster 2) ────────────────────────────
        let mut root = [0u8; 512];
        // Entry 0: HELLO.TXT (live, cluster 3, 13 bytes)
        root[0..8].copy_from_slice(b"HELLO   ");
        root[8..11].copy_from_slice(b"TXT");
        root[11] = 0x20; // archive
        root[26..28].copy_from_slice(&3u16.to_le_bytes()); // cluster_lo = 3
        root[28..32].copy_from_slice(&13u32.to_le_bytes()); // size
                                                            // Entry 1: deleted GONE.DAT (root-level deleted file)
        root[32] = DELETED_MARKER;
        root[33..40].copy_from_slice(b"ONE    ");
        root[40..43].copy_from_slice(b"DAT");
        root[43] = 0x20;
        root[58..60].copy_from_slice(&3u16.to_le_bytes()); // cluster_lo = 3 (reused for test)
        root[60..64].copy_from_slice(&4u32.to_le_bytes()); // size = 4
                                                           // Entry 2: SUBDIR (live directory, cluster 4)
        root[64..72].copy_from_slice(b"SUBDIR  ");
        root[72..75].copy_from_slice(b"   ");
        root[75] = 0x10; // ATTR_DIRECTORY
        root[90..92].copy_from_slice(&4u16.to_le_bytes()); // cluster_lo = 4
                                                           // Entry 3: end-of-directory
        root[96] = 0x00;
        dev.write_sector(6, &root);

        // ── HELLO.TXT data (sector 7 = cluster 3) ────────────────────────────
        let mut content = [0u8; 512];
        content[..13].copy_from_slice(b"Hello, World!");
        dev.write_sector(7, &content);

        // ── SUBDIR contents (sector 8 = cluster 4) ───────────────────────────
        let mut sub = [0u8; 512];
        // Entry 0: . (self)
        sub[0..8].copy_from_slice(b".       ");
        sub[8..11].copy_from_slice(b"   ");
        sub[11] = 0x10;
        sub[26..28].copy_from_slice(&4u16.to_le_bytes()); // cluster = 4 (self)
                                                          // Entry 1: .. (parent)
        sub[32..40].copy_from_slice(b"..      ");
        sub[40..43].copy_from_slice(b"   ");
        sub[43] = 0x10;
        sub[58..60].copy_from_slice(&2u16.to_le_bytes()); // cluster = 2 (root)
                                                          // Entry 2: deleted SUBDEL.TXT (cluster 5, 5 bytes)
        sub[64] = DELETED_MARKER;
        sub[65..72].copy_from_slice(b"UBDEL  ");
        sub[72..75].copy_from_slice(b"TXT");
        sub[75] = 0x20;
        sub[90..92].copy_from_slice(&5u16.to_le_bytes()); // cluster_lo = 5
        sub[92..96].copy_from_slice(&5u32.to_le_bytes()); // size = 5
                                                          // Entry 3: end-of-directory
        sub[96] = 0x00;
        dev.write_sector(8, &sub);

        // ── SUBDEL.TXT data (sector 9 = cluster 5) ───────────────────────────
        let mut subdata = [0u8; 512];
        subdata[..5].copy_from_slice(b"hello");
        dev.write_sector(9, &subdata);

        dev
    }

    #[test]
    fn deleted_files_in_subdirectory_found() {
        let dev = Arc::new(build_subdir_image());
        let parser = Fat32Parser::new(dev).unwrap();
        let deleted = parser.deleted_files().unwrap();

        // Should find the root-level GONE.DAT AND the SUBDIR/SUBDEL.TXT.
        assert_eq!(
            deleted.len(),
            2,
            "expected 2 deleted files (root + subdirectory), got: {:?}",
            deleted.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
        assert!(deleted.iter().all(|e| e.is_deleted));

        let has_subdir_entry = deleted.iter().any(|e| e.path.contains("SUBDIR"));
        assert!(
            has_subdir_entry,
            "expected a deleted file under SUBDIR, paths: {:?}",
            deleted.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_non_fat32() {
        let dev = Arc::new(MockBlockDevice::zeroed(512, 512));
        assert!(Fat32Parser::new(dev).is_err());
    }

    #[test]
    fn data_byte_offset_for_regular_file() {
        // In build_image(): cluster_size=512, data_offset = 4*512 + 2*1*512 = 3072.
        // HELLO.TXT lives in cluster 3.
        // Expected: data_offset + (3 - 2) * cluster_size = 3072 + 512 = 3584.
        let dev = Arc::new(build_image());
        let parser = Fat32Parser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let file = entries.iter().find(|e| e.name == "HELLO.TXT").unwrap();
        assert_eq!(
            file.data_byte_offset,
            Some(3584),
            "unexpected data_byte_offset for HELLO.TXT"
        );
    }

    #[test]
    fn data_byte_offset_for_directory_is_none() {
        // Directories should never have a data_byte_offset.
        let dev: Arc<dyn BlockDevice> = Arc::new(build_image());
        let parser = Fat32Parser::new(Arc::clone(&dev)).unwrap();
        // Build a directory entry directly to verify the code path.
        // In practice, build_entries marks is_dir=true for ATTR_DIRECTORY entries.
        // Use the existing helper indirectly: create a raw entry that looks like a dir.
        let raw_dir_entry: [u8; 32] = {
            let mut e = [0u8; 32];
            e[0..8].copy_from_slice(b"TESTDIR ");
            e[8..11].copy_from_slice(b"   ");
            e[11] = 0x10; // ATTR_DIRECTORY
            e[20..22].copy_from_slice(&0u16.to_le_bytes()); // cluster_hi
            e[26..28].copy_from_slice(&4u16.to_le_bytes()); // cluster_lo = 4
            e[28..32].copy_from_slice(&0u32.to_le_bytes()); // size = 0
            e
        };
        let result = parser.build_entries(&[raw_dir_entry], false);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_dir);
        assert!(
            result[0].data_byte_offset.is_none(),
            "directory must not have data_byte_offset"
        );
    }

    #[test]
    fn fat_datetime_zero_date_is_none() {
        assert_eq!(fat_datetime_to_unix(0, 0), None);
    }

    #[test]
    fn fat_datetime_known_date() {
        // 2000-01-01 00:00:00 UTC = Unix timestamp 946_684_800
        // FAT date: year=2000 → offset=20, month=1, day=1
        //   = (20 << 9) | (1 << 5) | 1 = 10273
        let date: u16 = (20 << 9) | (1 << 5) | 1;
        assert_eq!(fat_datetime_to_unix(date, 0), Some(946_684_800));
    }

    #[test]
    fn fat_datetime_time_fields() {
        // 1980-01-01 01:02:04 UTC = FAT epoch + 3724 s
        // date = (0 << 9) | (1 << 5) | 1 = 33
        // time: hours=1, minutes=2, seconds/2=2 → (1<<11)|(2<<5)|2 = 2114
        let date: u16 = (1 << 5) | 1;
        let time: u16 = (1 << 11) | (2 << 5) | 2;
        let expected = 3652u64 * 86_400 + 3724; // FAT_EPOCH_DAYS * 86400 + 1h2m4s
        assert_eq!(fat_datetime_to_unix(date, time), Some(expected));
    }

    /// Feeding a 10-byte device to Fat32Parser::new must return Err, not panic.
    #[test]
    fn truncated_device_returns_err_not_panic() {
        let dev = Arc::new(MockBlockDevice::new(vec![0u8; 10], 512));
        let result = Fat32Parser::new(dev);
        assert!(result.is_err(), "expected Err on 10-byte device, got Ok");
    }

    /// A valid-size device without the FAT32 boot signature must return Err,
    /// not panic.
    #[test]
    fn invalid_boot_signature_returns_err() {
        let data = vec![0u8; 512]; // boot sig at [510..512] is 0x00 0x00, not 0x55 0xAA
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        let result = Fat32Parser::new(dev);
        assert!(
            result.is_err(),
            "expected Err for invalid FAT32 boot signature"
        );
    }

    /// Reading cluster indices 0 and 1 (FAT-reserved) must return Err rather
    /// than underflowing the offset arithmetic.
    #[test]
    fn read_cluster_rejects_reserved_indices() {
        let dev = Arc::new(build_image());
        let parser = Fat32Parser::new(dev).expect("valid test image");
        for reserved in [0u32, 1u32] {
            let result = parser.read_cluster(reserved);
            assert!(
                result.is_err(),
                "cluster {reserved} is FAT-reserved — expected Err, got Ok"
            );
        }
    }
}
