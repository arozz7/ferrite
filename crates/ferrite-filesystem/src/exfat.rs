//! Minimum-viable exFAT read-only parser.
//!
//! Supports:
//! - Root and sub-directory listing
//! - FAT cluster-chain traversal for file reads
//! - Deleted-entry detection (entry-type high-bit cleared)
//! - UTF-16LE filename assembly from File Name directory entries

use std::io::Write;
use std::sync::Arc;

use tracing::trace;

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::{read_bytes, read_u32_le};
use crate::{FileEntry, FilesystemParser, FilesystemType, RecoveryChance};

// ── Entry type constants ──────────────────────────────────────────────────────

/// Live File entry (directory set primary).
const ETYPE_FILE_LIVE: u8 = 0x85;
/// Deleted File entry.
const ETYPE_FILE_DEL: u8 = 0x05;
/// Live Stream Extension (secondary).
const ETYPE_STREAM_LIVE: u8 = 0xC0;
/// Deleted Stream Extension.
const ETYPE_STREAM_DEL: u8 = 0x40;
/// Live File Name (secondary).
const ETYPE_NAME_LIVE: u8 = 0xC1;
/// Deleted File Name.
const ETYPE_NAME_DEL: u8 = 0x41;

/// Directory attribute: sub-directory.
const ATTR_DIRECTORY: u16 = 0x0010;

/// FAT end-of-chain threshold (≥ 0xFFFFFFF8).
const EOC_MIN: u32 = 0xFFFF_FFF8;

// ── Public struct ─────────────────────────────────────────────────────────────

/// Read-only exFAT filesystem parser.
pub struct ExFatParser {
    device: Arc<dyn BlockDevice>,
    fat_byte_offset: u64,    // byte offset of FAT region from volume start
    cluster_heap_offset: u64, // byte offset where cluster 2 starts
    bytes_per_cluster: u64,
    root_cluster: u32,
}

impl ExFatParser {
    /// Parse the exFAT VBR from `device` and return an initialised parser.
    pub fn new(device: Arc<dyn BlockDevice>) -> Result<Self> {
        let boot = read_bytes(device.as_ref(), 0, 512)?;

        if boot.len() < 11 || &boot[3..11] != b"EXFAT   " {
            return Err(FilesystemError::InvalidStructure {
                context: "exFAT boot sector",
                reason: "OEM name \"EXFAT   \" not found at offset 3".to_string(),
            });
        }

        // BytesPerSectorShift @ 108 and SectorsPerClusterShift @ 109
        // are defined by the exFAT specification.
        let bytes_per_sector_shift = boot[108] as u32;
        let sectors_per_cluster_shift = boot[109] as u32;

        if !(9..=12).contains(&bytes_per_sector_shift) {
            return Err(FilesystemError::InvalidStructure {
                context: "exFAT VBR",
                reason: format!(
                    "BytesPerSectorShift {bytes_per_sector_shift} out of valid range [9,12]"
                ),
            });
        }

        let bytes_per_sector = 1u64 << bytes_per_sector_shift;
        let bytes_per_cluster = bytes_per_sector << sectors_per_cluster_shift;

        // FatOffset @ 80 — sector offset of FAT from start of volume.
        let fat_sector_offset = read_u32_le(&boot, 80)? as u64;
        // ClusterHeapOffset @ 88 — sector offset of cluster heap.
        let cluster_heap_sector = read_u32_le(&boot, 88)? as u64;
        // RootDirectoryFirstCluster @ 96.
        let root_cluster = read_u32_le(&boot, 96)?;

        let fat_byte_offset = fat_sector_offset * bytes_per_sector;
        let cluster_heap_offset = cluster_heap_sector * bytes_per_sector;

        trace!(
            fat_byte_offset,
            cluster_heap_offset,
            bytes_per_cluster,
            root_cluster,
            "exFAT parser initialised"
        );

        Ok(Self {
            device,
            fat_byte_offset,
            cluster_heap_offset,
            bytes_per_cluster,
            root_cluster,
        })
    }

    // ── Cluster helpers ───────────────────────────────────────────────────────

    fn cluster_byte_offset(&self, cluster: u32) -> u64 {
        self.cluster_heap_offset + (cluster as u64 - 2) * self.bytes_per_cluster
    }

    fn read_fat_entry(&self, cluster: u32) -> Result<u32> {
        let offset = self.fat_byte_offset + cluster as u64 * 4;
        let raw = read_bytes(self.device.as_ref(), offset, 4)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn is_eoc(entry: u32) -> bool {
        entry >= EOC_MIN
    }

    fn is_cluster_free(&self, cluster: u32) -> bool {
        self.read_fat_entry(cluster)
            .map(|e| e == 0)
            .unwrap_or(false)
    }

    fn read_cluster(&self, cluster: u32) -> Result<Vec<u8>> {
        read_bytes(
            self.device.as_ref(),
            self.cluster_byte_offset(cluster),
            self.bytes_per_cluster as usize,
        )
    }

    // ── Directory helpers ─────────────────────────────────────────────────────

    /// Collect all raw 32-byte directory entries from the cluster chain
    /// starting at `start_cluster`, stopping at the end-of-directory marker.
    fn raw_dir_entries(&self, start_cluster: u32) -> Result<Vec<[u8; 32]>> {
        let mut out = Vec::new();
        let mut cluster = start_cluster;
        loop {
            let data = self.read_cluster(cluster)?;
            for chunk in data.chunks_exact(32) {
                let mut entry = [0u8; 32];
                entry.copy_from_slice(chunk);
                if entry[0] == 0x00 {
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

    /// Parse raw 32-byte directory entries into [`FileEntry`] values.
    ///
    /// exFAT directory entries come in sets: one File primary entry followed
    /// by `secondary_count` continuation entries (Stream Extension + one or
    /// more File Name entries).  The high bit of the type byte is clear for
    /// deleted entries.
    ///
    /// When `include_deleted` is `false`, deleted entry sets are skipped.
    fn build_entries(&self, raw: &[[u8; 32]], include_deleted: bool) -> Vec<FileEntry> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < raw.len() {
            let entry = &raw[i];
            let etype = entry[0];

            // Only start a set on a File primary entry.
            if etype != ETYPE_FILE_LIVE && etype != ETYPE_FILE_DEL {
                i += 1;
                continue;
            }

            let is_deleted = etype == ETYPE_FILE_DEL;
            let secondary_count = entry[1] as usize;

            if is_deleted && !include_deleted {
                i += 1 + secondary_count;
                continue;
            }

            // Need at least one Stream Extension and one File Name entry.
            if secondary_count < 2 {
                i += 1;
                continue;
            }

            // File attributes @ bytes 4-5 (LE).
            let attrs = u16::from_le_bytes([entry[4], entry[5]]);
            let is_dir = attrs & ATTR_DIRECTORY != 0;

            // Timestamps — exFAT 32-bit format (creation @ 8, modified @ 16).
            let created = exfat_ts_to_unix(u32::from_le_bytes([
                entry[8], entry[9], entry[10], entry[11],
            ]));
            let modified = exfat_ts_to_unix(u32::from_le_bytes([
                entry[16], entry[17], entry[18], entry[19],
            ]));

            // Stream Extension is always the first secondary entry.
            let stream_idx = i + 1;
            if stream_idx >= raw.len() {
                i += 1 + secondary_count;
                continue;
            }
            let stream = &raw[stream_idx];
            let expected_stream = if is_deleted {
                ETYPE_STREAM_DEL
            } else {
                ETYPE_STREAM_LIVE
            };
            if stream[0] != expected_stream {
                i += 1;
                continue;
            }

            let name_len = stream[3] as usize;
            // ValidDataLength @ bytes 8-15 (u64 LE) — actual data bytes.
            let data_length = u64::from_le_bytes([
                stream[8], stream[9], stream[10], stream[11],
                stream[12], stream[13], stream[14], stream[15],
            ]);
            // FirstCluster @ bytes 20-23 (u32 LE).
            let first_cluster = u32::from_le_bytes([
                stream[20], stream[21], stream[22], stream[23],
            ]);

            // Assemble filename from one or more File Name entries.
            let mut name_u16: Vec<u16> = Vec::with_capacity(name_len);
            let name_entries = secondary_count - 1;
            for k in 0..name_entries {
                let idx = stream_idx + 1 + k;
                if idx >= raw.len() {
                    break;
                }
                let name_entry = &raw[idx];
                let expected_name = if is_deleted {
                    ETYPE_NAME_DEL
                } else {
                    ETYPE_NAME_LIVE
                };
                if name_entry[0] != expected_name {
                    break;
                }
                // Up to 15 UTF-16LE code units @ bytes 2-31.
                for c in 0..15usize {
                    let base = 2 + c * 2;
                    let cp = u16::from_le_bytes([name_entry[base], name_entry[base + 1]]);
                    if cp == 0 {
                        break;
                    }
                    name_u16.push(cp);
                    if name_u16.len() >= name_len {
                        break;
                    }
                }
                if name_u16.len() >= name_len {
                    break;
                }
            }

            let name = String::from_utf16_lossy(&name_u16).to_string();
            let data_byte_offset = if is_dir || first_cluster < 2 {
                None
            } else {
                Some(self.cluster_byte_offset(first_cluster))
            };

            result.push(FileEntry {
                name: name.clone(),
                path: format!("/{name}"),
                size: data_length,
                is_dir,
                is_deleted,
                created,
                modified,
                first_cluster: Some(first_cluster),
                mft_record: None,
                inode_number: None,
                data_byte_offset,
                recovery_chance: RecoveryChance::Unknown,
            });

            i += 1 + secondary_count;
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

    /// Follow the FAT chain from `start_cluster` and write up to `file_size`
    /// bytes to `writer`.
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
                    context: "exFAT read_file write",
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

impl FilesystemParser for ExFatParser {
    fn filesystem_type(&self) -> FilesystemType {
        FilesystemType::ExFat
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
                context: "exFAT read_file",
                reason: "FileEntry has no starting cluster".to_string(),
            })?;
        self.read_chain(cluster, entry.size, writer)
    }

    fn deleted_files(&self) -> Result<Vec<FileEntry>> {
        let raw = self.raw_dir_entries(self.root_cluster)?;
        let all = self.build_entries(&raw, true);
        let mut deleted: Vec<FileEntry> = all.into_iter().filter(|e| e.is_deleted).collect();
        for entry in deleted.iter_mut() {
            entry.recovery_chance = if entry.size == 0 {
                RecoveryChance::Unknown
            } else if entry
                .first_cluster
                .map(|c| c >= 2 && self.is_cluster_free(c))
                .unwrap_or(false)
            {
                // Start cluster is still free — data likely intact.
                RecoveryChance::High
            } else if entry.first_cluster.map(|c| c >= 2).unwrap_or(false) {
                // Cluster already reallocated.
                RecoveryChance::Low
            } else {
                RecoveryChance::Unknown
            };
        }
        Ok(deleted)
    }
}

// ── Timestamp helper ──────────────────────────────────────────────────────────

/// Convert a 32-bit exFAT timestamp to a Unix timestamp (seconds since 1970).
///
/// exFAT timestamp bit layout (LSB first):
///   bits  0-4:  double-seconds (0–29; multiply by 2 for actual seconds)
///   bits  5-10: minutes (0–59)
///   bits 11-15: hours (0–23)
///   bits 16-20: day (1–31)
///   bits 21-24: month (1–12)
///   bits 25-31: year offset from 1980 (0–127)
///
/// Returns `None` when the value is zero (field not set) or any component is
/// out of range.
fn exfat_ts_to_unix(ts: u32) -> Option<u64> {
    if ts == 0 {
        return None;
    }
    let sec2 = ts & 0x1F;
    let min = (ts >> 5) & 0x3F;
    let hour = (ts >> 11) & 0x1F;
    let day = (ts >> 16) & 0x1F;
    let month = (ts >> 21) & 0x0F;
    let year = 1980u32 + (ts >> 25);

    if month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }
    if hour > 23 || min > 59 || sec2 > 29 {
        return None;
    }
    let sec = sec2 * 2;

    const FAT_EPOCH_DAYS: u64 = 3652;
    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let is_leap =
        |y: u32| (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400);

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal exFAT disk image in memory.
    ///
    /// Layout (512-byte sectors, BytesPerSectorShift=9, SectorsPerClusterShift=0):
    ///   Sector 0 — VBR (boot sector)
    ///   Sector 1 — FAT
    ///   Sector 2 — Cluster 2 = root directory
    ///   Sector 3 — Cluster 3 = file data for "HELLO.TXT"
    fn build_image() -> MockBlockDevice {
        let mut dev = MockBlockDevice::zeroed(2048, 512);

        // ── Sector 0: VBR ────────────────────────────────────────────────────
        let mut vbr = [0u8; 512];
        vbr[3..11].copy_from_slice(b"EXFAT   ");
        // FatOffset @ 80 = sector 1
        vbr[80..84].copy_from_slice(&1u32.to_le_bytes());
        // FatLength @ 84 = 1 sector
        vbr[84..88].copy_from_slice(&1u32.to_le_bytes());
        // ClusterHeapOffset @ 88 = sector 2
        vbr[88..92].copy_from_slice(&2u32.to_le_bytes());
        // ClusterCount @ 92 = 2 (clusters 2 and 3)
        vbr[92..96].copy_from_slice(&2u32.to_le_bytes());
        // RootDirectoryFirstCluster @ 96 = 2
        vbr[96..100].copy_from_slice(&2u32.to_le_bytes());
        // BytesPerSectorShift @ 108 = 9 (512 bytes)
        vbr[108] = 9;
        // SectorsPerClusterShift @ 109 = 0 (1 sector per cluster)
        vbr[109] = 0;
        dev.write_sector(0, &vbr);

        // ── Sector 1: FAT ────────────────────────────────────────────────────
        let mut fat = [0u8; 512];
        // Entry 0: media descriptor
        fat[0..4].copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
        // Entry 1: reserved
        fat[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        // Entry 2: end-of-chain (root directory)
        fat[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        // Entry 3: end-of-chain (HELLO.TXT data)
        fat[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        // Entry 4: free (used by deleted GONE.DAT)
        fat[16..20].copy_from_slice(&0u32.to_le_bytes());
        dev.write_sector(1, &fat);

        // ── Sector 2: Root directory (cluster 2) ─────────────────────────────
        let mut dir = [0u8; 512];

        // Entry set 0: live file "HELLO.TXT" (9 chars)
        // -- File primary (0x85)
        dir[0] = 0x85; // ETYPE_FILE_LIVE
        dir[1] = 2;    // secondary_count (stream + name)
        dir[4] = 0x20; // attrs: ARCHIVE
        dir[5] = 0x00;

        // -- Stream Extension (0xC0) at offset 32
        dir[32] = 0xC0; // ETYPE_STREAM_LIVE
        dir[33] = 0x01; // flags: allocation possible
        dir[35] = 9;    // name_length
        // ValidDataLength @ 40-47 = 13 (u64 LE)
        dir[40..48].copy_from_slice(&13u64.to_le_bytes());
        // FirstCluster @ 52-55 = 3
        dir[52..56].copy_from_slice(&3u32.to_le_bytes());
        // DataLength @ 56-63 = 13 (u64 LE)
        dir[56..64].copy_from_slice(&13u64.to_le_bytes());

        // -- File Name (0xC1) at offset 64: "HELLO.TXT" in UTF-16LE
        dir[64] = 0xC1; // ETYPE_NAME_LIVE
        dir[65] = 0x00; // flags
        let name: Vec<u16> = "HELLO.TXT".encode_utf16().collect();
        for (k, &cp) in name.iter().enumerate() {
            let base = 66 + k * 2;
            let bytes = cp.to_le_bytes();
            dir[base] = bytes[0];
            dir[base + 1] = bytes[1];
        }

        // Entry set 1: deleted file "GONE.DAT" (8 chars), starts at offset 96
        // -- File primary (0x05) at offset 96
        dir[96] = 0x05;  // ETYPE_FILE_DEL
        dir[97] = 2;     // secondary_count
        dir[100] = 0x20; // attrs: ARCHIVE

        // -- Stream Extension (0x40) at offset 128
        dir[128] = 0x40; // ETYPE_STREAM_DEL
        dir[129] = 0x01; // flags
        dir[131] = 8;    // name_length
        // ValidDataLength @ 136-143 = 4
        dir[136..144].copy_from_slice(&4u64.to_le_bytes());
        // FirstCluster @ 148-151 = 4 (free in FAT → RecoveryChance::High)
        dir[148..152].copy_from_slice(&4u32.to_le_bytes());
        // DataLength @ 152-159 = 4
        dir[152..160].copy_from_slice(&4u64.to_le_bytes());

        // -- File Name (0x41) at offset 160: "GONE.DAT" in UTF-16LE
        dir[160] = 0x41; // ETYPE_NAME_DEL
        dir[161] = 0x00;
        let del_name: Vec<u16> = "GONE.DAT".encode_utf16().collect();
        for (k, &cp) in del_name.iter().enumerate() {
            let base = 162 + k * 2;
            let bytes = cp.to_le_bytes();
            dir[base] = bytes[0];
            dir[base + 1] = bytes[1];
        }

        // End-of-directory marker at offset 192
        dir[192] = 0x00;

        dev.write_sector(2, &dir);

        // ── Sector 3: File data (cluster 3) ──────────────────────────────────
        let mut data = [0u8; 512];
        data[..13].copy_from_slice(b"Hello, World!");
        dev.write_sector(3, &data);

        dev
    }

    #[test]
    fn detects_exfat() {
        let dev = Arc::new(build_image());
        let parser = ExFatParser::new(dev).unwrap();
        assert_eq!(parser.filesystem_type(), FilesystemType::ExFat);
    }

    #[test]
    fn root_directory_lists_live_file() {
        let dev = Arc::new(build_image());
        let parser = ExFatParser::new(dev).unwrap();
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
        let parser = ExFatParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        assert_eq!(entries.len(), 1);
        let mut buf = Vec::new();
        let written = parser.read_file(&entries[0], &mut buf).unwrap();
        assert_eq!(written, 13);
        assert_eq!(&buf, b"Hello, World!");
    }

    #[test]
    fn data_byte_offset_for_regular_file() {
        // cluster_heap_offset = 2 * 512 = 1024
        // bytes_per_cluster = 512
        // cluster_byte_offset(3) = 1024 + (3-2)*512 = 1536
        let dev = Arc::new(build_image());
        let parser = ExFatParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let file = &entries[0];
        assert_eq!(
            file.data_byte_offset,
            Some(1536),
            "unexpected data_byte_offset for HELLO.TXT"
        );
    }

    #[test]
    fn deleted_files_found_with_recovery_chance() {
        let dev = Arc::new(build_image());
        let parser = ExFatParser::new(dev).unwrap();
        let deleted = parser.deleted_files().unwrap();
        assert_eq!(deleted.len(), 1);
        let d = &deleted[0];
        assert_eq!(d.name, "GONE.DAT");
        assert!(d.is_deleted);
        assert_eq!(d.size, 4);
        // first_cluster=4, FAT[4]=0 (free) → RecoveryChance::High
        assert_eq!(d.recovery_chance, RecoveryChance::High);
    }

    #[test]
    fn rejects_non_exfat_device() {
        let dev = Arc::new(MockBlockDevice::zeroed(512, 512));
        assert!(ExFatParser::new(dev).is_err());
    }

    #[test]
    fn truncated_device_returns_err_not_panic() {
        let dev = Arc::new(MockBlockDevice::new(vec![0u8; 10], 512));
        assert!(ExFatParser::new(dev).is_err());
    }

    #[test]
    fn exfat_ts_zero_is_none() {
        assert_eq!(exfat_ts_to_unix(0), None);
    }

    #[test]
    fn exfat_ts_known_date() {
        // 2000-01-01 00:00:00 UTC = Unix timestamp 946_684_800
        // year offset = 2000-1980 = 20, month=1, day=1, hour=0, min=0, sec2=0
        // ts = (20 << 25) | (1 << 21) | (1 << 16) | 0 | 0 | 0
        //    = 671088640 | 2097152 | 65536
        //    = 673251328
        let ts: u32 = (20u32 << 25) | (1u32 << 21) | (1u32 << 16);
        assert_eq!(exfat_ts_to_unix(ts), Some(946_684_800));
    }

    #[test]
    fn exfat_ts_invalid_month_is_none() {
        // month=0 (bits 21-24 = 0) with non-zero day — should return None
        let ts: u32 = (20u32 << 25) | (0u32 << 21) | (1u32 << 16);
        assert_eq!(exfat_ts_to_unix(ts), None);
    }
}
