//! Minimum-viable NTFS read-only parser.
//!
//! Parses the boot sector to locate the MFT, then scans MFT records to
//! enumerate files and directories.  Supports resident and non-resident DATA
//! attributes (run-list decoding for non-resident reads).

use std::io::Write;
use std::sync::Arc;

use tracing::trace;

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::{read_bytes, read_u16_le, read_u64_le};
use crate::ntfs_helpers::{
    apply_fixup, mft_record_count, parse_file_info, parse_standard_info, read_run_list, ATTR_DATA,
    ATTR_END, FILE_SIG,
};
use crate::{FileEntry, FilesystemParser, FilesystemType};

// â”€â”€ Constants â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

const NTFS_OEM_ID: &[u8] = b"NTFS    ";

// MFT record number for the root directory
const ROOT_MFT_RECORD: u64 = 5;

// Maximum MFT records to scan (safety cap for very large volumes)
const MAX_SCAN_RECORDS: u64 = 65_536;

// â”€â”€ Public struct â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Read-only NTFS filesystem parser.
pub struct NtfsParser {
    device: Arc<dyn BlockDevice>,
    cluster_size: u64,
    mft_start_byte: u64,
    mft_record_size: u64,
    mft_record_count: u64,
}

impl NtfsParser {
    /// Parse the NTFS boot sector from `device` and return an initialised parser.
    pub fn new(device: Arc<dyn BlockDevice>) -> Result<Self> {
        let boot = read_bytes(device.as_ref(), 0, 512)?;

        if &boot[3..11] != NTFS_OEM_ID {
            return Err(FilesystemError::InvalidStructure {
                context: "NTFS boot sector",
                reason: "OEM ID \"NTFS    \" not found at offset 3".to_string(),
            });
        }

        let bytes_per_sector = read_u16_le(&boot, 11)? as u64;
        let sectors_per_cluster = boot[13] as u64;
        let mft_cluster = read_u64_le(&boot, 48)?;

        if bytes_per_sector == 0 || sectors_per_cluster == 0 {
            return Err(FilesystemError::InvalidStructure {
                context: "NTFS BPB",
                reason: "bytes_per_sector or sectors_per_cluster is zero".to_string(),
            });
        }

        let cluster_size = bytes_per_sector * sectors_per_cluster;
        let mft_start_byte = mft_cluster * cluster_size;

        // Byte 64: clusters_per_mft_record.
        // If < 128 (positive when interpreted as i8): value is in clusters.
        // If >= 128 (negative as i8): record size = 2^|value| bytes.
        let cpmr = boot[64] as i8;
        let mft_record_size = if cpmr > 0 {
            cpmr as u64 * cluster_size
        } else {
            1u64 << cpmr.unsigned_abs()
        };

        if mft_record_size == 0 {
            return Err(FilesystemError::InvalidStructure {
                context: "NTFS boot sector",
                reason: "computed mft_record_size is zero".to_string(),
            });
        }

        // Determine how many MFT records exist by reading record 0 ($MFT itself)
        // and extracting its DATA attribute size.
        let mft_record_count =
            mft_record_count(&device, mft_start_byte, mft_record_size, cluster_size)
                .unwrap_or(MAX_SCAN_RECORDS)
                .min(MAX_SCAN_RECORDS);

        trace!(
            cluster_size,
            mft_start_byte,
            mft_record_size,
            mft_record_count,
            "NTFS parser initialised"
        );

        Ok(Self {
            device,
            cluster_size,
            mft_start_byte,
            mft_record_size,
            mft_record_count,
        })
    }

    // â”€â”€ MFT helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn read_mft_record(&self, record_number: u64) -> Result<Vec<u8>> {
        let offset = self.mft_start_byte + record_number * self.mft_record_size;
        let raw = read_bytes(self.device.as_ref(), offset, self.mft_record_size as usize)?;
        Ok(apply_fixup(raw))
    }

    /// Scan all MFT records and return parsed [`FileEntry`]s.
    ///
    /// `filter` is called with `(parent_ref, in_use, is_dir)` and only entries
    /// returning `true` are included.
    fn scan<F>(&self, filter: F) -> Result<Vec<FileEntry>>
    where
        F: Fn(u64, bool, bool) -> bool,
    {
        let mut entries = Vec::new();

        for i in 0..self.mft_record_count {
            let raw = match self.read_mft_record(i) {
                Ok(r) => r,
                Err(_) => continue,
            };

            if raw.len() < 48 || &raw[0..4] != FILE_SIG {
                continue;
            }

            // Safety: raw.len() >= 48 checked above.
            let flags = u16::from_le_bytes([raw[22], raw[23]]);
            let in_use = flags & 1 != 0;
            let is_dir = flags & 2 != 0;

            if let Some((name, parent_ref, data_size, first_lcn)) = parse_file_info(&raw) {
                if filter(parent_ref, in_use, is_dir) {
                    let data_byte_offset = if is_dir {
                        None
                    } else {
                        first_lcn.map(|lcn| lcn * self.cluster_size)
                    };
                    let (created, modified) = parse_standard_info(&raw).unwrap_or((None, None));
                    entries.push(FileEntry {
                        name: name.clone(),
                        path: format!("/{name}"),
                        size: data_size,
                        is_dir,
                        is_deleted: !in_use,
                        created,
                        modified,
                        first_cluster: None,
                        mft_record: Some(i),
                        inode_number: None,
                        data_byte_offset,
                    });
                }
            }
        }

        Ok(entries)
    }
}

// â”€â”€ FilesystemParser impl â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

impl FilesystemParser for NtfsParser {
    fn filesystem_type(&self) -> FilesystemType {
        FilesystemType::Ntfs
    }

    fn root_directory(&self) -> Result<Vec<FileEntry>> {
        // Root directory entries are children of MFT record 5 (".")
        self.scan(|parent_ref, in_use, _| in_use && parent_ref == ROOT_MFT_RECORD)
    }

    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>> {
        // Resolve the path one component at a time starting from root.
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return self.root_directory();
        }

        let mut parent = ROOT_MFT_RECORD;
        for part in &parts {
            let children = self.scan(|pr, in_use, is_dir| in_use && is_dir && pr == parent)?;
            let found = children
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
                .map(|e| e.mft_record.unwrap_or(0));
            match found {
                Some(rec) => parent = rec,
                None => return Err(FilesystemError::NotFound(path.to_string())),
            }
        }

        self.scan(|pr, in_use, _| in_use && pr == parent)
    }

    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64> {
        let record_num = entry.mft_record.ok_or(FilesystemError::InvalidStructure {
            context: "read_file",
            reason: "FileEntry has no MFT record number".to_string(),
        })?;

        let raw = self.read_mft_record(record_num)?;
        if raw.len() < 48 || &raw[0..4] != FILE_SIG {
            return Err(FilesystemError::InvalidStructure {
                context: "MFT record",
                reason: "FILE signature missing".to_string(),
            });
        }

        // Safety: raw.len() >= 48 checked above.
        let first_attr_offset = u16::from_le_bytes([raw[20], raw[21]]) as usize;
        let mut pos = first_attr_offset;

        while pos + 8 <= raw.len() {
            let type_id = u32::from_le_bytes([raw[pos], raw[pos + 1], raw[pos + 2], raw[pos + 3]]);
            if type_id == ATTR_END {
                break;
            }
            let attr_len =
                u32::from_le_bytes([raw[pos + 4], raw[pos + 5], raw[pos + 6], raw[pos + 7]])
                    as usize;
            if attr_len == 0 || pos + attr_len > raw.len() {
                break;
            }

            if type_id == ATTR_DATA {
                let non_resident = raw[pos + 8];
                if non_resident == 0 {
                    // Resident: data is inside the MFT record.
                    // Guard: resident header requires at least 22 bytes past attr start.
                    if pos + 22 > raw.len() {
                        break;
                    }
                    let val_len = u32::from_le_bytes([
                        raw[pos + 16],
                        raw[pos + 17],
                        raw[pos + 18],
                        raw[pos + 19],
                    ]) as usize;
                    let val_off = u16::from_le_bytes([raw[pos + 20], raw[pos + 21]]) as usize;
                    let start = pos + val_off;
                    let end = start + val_len;
                    if end <= raw.len() {
                        writer.write_all(&raw[start..end]).map_err(|e| {
                            FilesystemError::InvalidStructure {
                                context: "read_file write (resident)",
                                reason: e.to_string(),
                            }
                        })?;
                        return Ok(val_len as u64);
                    }
                } else {
                    // Non-resident: decode run list.
                    if pos + 0x40 > raw.len() {
                        break;
                    }
                    // Safety: pos + 0x40 <= raw.len() checked above.
                    let data_size = u64::from_le_bytes([
                        raw[pos + 0x30],
                        raw[pos + 0x31],
                        raw[pos + 0x32],
                        raw[pos + 0x33],
                        raw[pos + 0x34],
                        raw[pos + 0x35],
                        raw[pos + 0x36],
                        raw[pos + 0x37],
                    ]);
                    let rl_off = u16::from_le_bytes([raw[pos + 0x20], raw[pos + 0x21]]) as usize;
                    let rl_start = pos + rl_off;
                    if rl_start >= raw.len() {
                        break;
                    }
                    let written = read_run_list(
                        self.device.as_ref(),
                        &raw[rl_start..],
                        self.cluster_size,
                        data_size,
                        writer,
                    )?;
                    return Ok(written);
                }
            }

            pos += attr_len;
        }

        Err(FilesystemError::NotFound(
            "DATA attribute not found".to_string(),
        ))
    }

    fn deleted_files(&self) -> Result<Vec<FileEntry>> {
        self.scan(|parent_ref, in_use, _| !in_use && parent_ref == ROOT_MFT_RECORD)
    }

    fn enumerate_files(&self) -> Result<Vec<FileEntry>> {
        // Return every non-directory MFT record (in-use or deleted), across all directories.
        self.scan(|_, _, is_dir| !is_dir)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;
    use crate::ntfs_helpers::{first_lcn_from_run_list, ATTR_FILE_NAME};

    // â”€â”€ Image builder helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Build a minimal NTFS boot sector.
    ///
    /// - bytes_per_sector   = 512
    /// - sectors_per_cluster = 1  â†’  cluster_size = 512
    /// - mft_cluster_number  = 8  â†’  MFT starts at byte 4096
    /// - clusters_per_mft_record = 0xF6 (-10)  â†’  record_size = 1024
    fn boot_sector() -> [u8; 512] {
        let mut s = [0u8; 512];
        s[3..11].copy_from_slice(b"NTFS    ");
        s[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes_per_sector
        s[13] = 1; // sectors_per_cluster
        s[48..56].copy_from_slice(&8u64.to_le_bytes()); // mft_cluster
        s[64] = 0xF6_u8; // clusters_per_mft_record â†’ 1024 bytes/record
        s[510] = 0x55;
        s[511] = 0xAA;
        s
    }

    /// Build a 1024-byte MFT record with one $FILE_NAME attribute.
    ///
    /// usa_count is set to 1 (no sector fixup replacements required).
    fn build_mft_record(
        in_use: bool,
        is_dir: bool,
        parent_ref: u64,
        name: &str,
        file_data: &[u8],
    ) -> Vec<u8> {
        let mut rec = vec![0u8; 1024];

        // â”€â”€ Fixed header â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        rec[0..4].copy_from_slice(b"FILE");
        // usa_offset = 0x30, usa_count = 1 (no fixup replacements)
        rec[4..6].copy_from_slice(&0x30u16.to_le_bytes());
        rec[6..8].copy_from_slice(&1u16.to_le_bytes());
        // Flags
        let mut flags = 0u16;
        if in_use {
            flags |= 1;
        }
        if is_dir {
            flags |= 2;
        }
        rec[22..24].copy_from_slice(&flags.to_le_bytes());
        // Allocated size
        rec[28..32].copy_from_slice(&1024u32.to_le_bytes());

        // â”€â”€ Attributes start at 0x38 â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        let mut pos = 0x38usize;

        // $FILE_NAME (type 0x30, resident)
        let name_utf16: Vec<u16> = name.encode_utf16().collect();
        let name_bytes: Vec<u8> = name_utf16.iter().flat_map(|&c| c.to_le_bytes()).collect();
        // value = 0x42 + name_bytes.len()
        let fn_val_len = 0x42usize + name_bytes.len();
        // attr header = 0x18 bytes, total length 8-byte-aligned
        let fn_attr_len = ((0x18 + fn_val_len) + 7) & !7;

        rec[pos..pos + 4].copy_from_slice(&ATTR_FILE_NAME.to_le_bytes());
        rec[pos + 4..pos + 8].copy_from_slice(&(fn_attr_len as u32).to_le_bytes());
        rec[pos + 8] = 0; // resident
        rec[pos + 16..pos + 20].copy_from_slice(&(fn_val_len as u32).to_le_bytes());
        rec[pos + 20..pos + 22].copy_from_slice(&0x18u16.to_le_bytes()); // val_offset

        // Value content starts at pos+0x18
        let v = pos + 0x18;
        rec[v..v + 8].copy_from_slice(&parent_ref.to_le_bytes()); // parent ref
        rec[v + 0x40] = name_utf16.len() as u8; // name_length
        rec[v + 0x41] = 1; // namespace = Win32
        rec[v + 0x42..v + 0x42 + name_bytes.len()].copy_from_slice(&name_bytes);

        pos += fn_attr_len;

        // $DATA (type 0x80, resident) â€” only if non-empty
        if !file_data.is_empty() {
            let data_val_len = file_data.len();
            let data_attr_len = ((0x18 + data_val_len) + 7) & !7;

            rec[pos..pos + 4].copy_from_slice(&ATTR_DATA.to_le_bytes());
            rec[pos + 4..pos + 8].copy_from_slice(&(data_attr_len as u32).to_le_bytes());
            rec[pos + 8] = 0; // resident
            rec[pos + 16..pos + 20].copy_from_slice(&(data_val_len as u32).to_le_bytes());
            rec[pos + 20..pos + 22].copy_from_slice(&0x18u16.to_le_bytes());
            rec[pos + 0x18..pos + 0x18 + data_val_len].copy_from_slice(file_data);

            pos += data_attr_len;
        }

        // End-of-attributes marker
        rec[pos..pos + 4].copy_from_slice(&ATTR_END.to_le_bytes());
        // Record used size
        rec[24..28].copy_from_slice(&((pos + 8) as u32).to_le_bytes());

        // Update first_attr_offset
        rec[20..22].copy_from_slice(&0x38u16.to_le_bytes());

        rec
    }

    /// Lay out a minimal NTFS image in a MockBlockDevice (16 KiB).
    ///
    /// MFT starts at cluster 8 (sector 8, byte 4096).
    /// Record size = 1024 bytes (two 512-byte sectors each).
    /// Records 0â€“5 are system placeholders; record 5 = root; record 6 = test file.
    fn build_image() -> MockBlockDevice {
        const SECTOR: usize = 512;
        const MFT_START: usize = 4096; // cluster 8 * 512
        const REC_SIZE: usize = 1024;

        let mut dev = MockBlockDevice::zeroed(16384, SECTOR as u32);

        // Boot sector
        let boot = boot_sector();
        dev.write_sector(0, &boot);

        // Write a dummy FILE record for MFT records 0â€“4 (system files)
        for i in 0u64..5 {
            let offset_bytes = MFT_START + i as usize * REC_SIZE;
            let rec = build_mft_record(true, true, 5, &format!("$sys{i}"), &[]);
            let sector = (offset_bytes / SECTOR) as u64;
            dev.write_sector(sector, &rec[..SECTOR]);
            dev.write_sector(sector + 1, &rec[SECTOR..]);
        }

        // MFT record 5: root directory "." (parent = itself)
        {
            let offset_bytes = MFT_START + 5 * REC_SIZE;
            let rec = build_mft_record(true, true, 5, ".", &[]);
            let sector = (offset_bytes / SECTOR) as u64;
            dev.write_sector(sector, &rec[..SECTOR]);
            dev.write_sector(sector + 1, &rec[SECTOR..]);
        }

        // MFT record 6: test file "hello.txt" (parent = 5)
        {
            let offset_bytes = MFT_START + 6 * REC_SIZE;
            let rec = build_mft_record(true, false, 5, "hello.txt", b"Hello, NTFS!");
            let sector = (offset_bytes / SECTOR) as u64;
            dev.write_sector(sector, &rec[..SECTOR]);
            dev.write_sector(sector + 1, &rec[SECTOR..]);
        }

        dev
    }

    // â”€â”€ Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn detects_ntfs() {
        let dev = Arc::new(build_image());
        let parser = NtfsParser::new(dev).unwrap();
        assert_eq!(parser.filesystem_type(), FilesystemType::Ntfs);
    }

    #[test]
    fn root_directory_contains_test_file() {
        let dev = Arc::new(build_image());
        let parser = NtfsParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"hello.txt"),
            "expected 'hello.txt' in root, got: {names:?}"
        );
    }

    #[test]
    fn read_file_resident_data() {
        let dev = Arc::new(build_image());
        let parser = NtfsParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let file = entries
            .iter()
            .find(|e| e.name == "hello.txt")
            .expect("hello.txt not found");
        let mut buf = Vec::new();
        let written = parser.read_file(file, &mut buf).unwrap();
        assert_eq!(written, 12);
        assert_eq!(&buf, b"Hello, NTFS!");
    }

    #[test]
    fn rejects_non_ntfs() {
        let dev = Arc::new(MockBlockDevice::zeroed(512, 512));
        assert!(NtfsParser::new(dev).is_err());
    }

    #[test]
    fn fixup_passthrough_when_usa_count_one() {
        // usa_count = 1 means no sectors need fixing â€” record should be returned unchanged.
        let raw = vec![0u8; 1024];
        let result = apply_fixup(raw.clone());
        assert_eq!(result, raw);
    }

    #[test]
    fn run_list_decode_single_run() {
        // Build a one-cluster run at LCN 10, containing "DATA" followed by zeros.
        let cluster_size: u64 = 512;
        let lcn: u64 = 2; // sector 2

        let mut dev = MockBlockDevice::zeroed(2048, 512);
        let mut sector = [0u8; 512];
        sector[..4].copy_from_slice(b"DATA");
        dev.write_sector(lcn, &sector);

        // Run list: header=0x11 (1 byte length, 1 byte offset), len=1, off=+2
        let run_list = [0x11u8, 0x01, 0x02, 0x00];
        let dev_ref: Arc<dyn BlockDevice> = Arc::new(dev);

        let mut buf = Vec::new();
        let written =
            read_run_list(dev_ref.as_ref(), &run_list, cluster_size, 4, &mut buf).unwrap();
        assert_eq!(written, 4);
        assert_eq!(&buf, b"DATA");
    }

    #[test]
    fn resident_file_data_byte_offset_is_none() {
        // Resident files live inside the MFT record â€” no physical cluster offset.
        let dev = Arc::new(build_image());
        let parser = NtfsParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let file = entries.iter().find(|e| e.name == "hello.txt").unwrap();
        assert!(
            file.data_byte_offset.is_none(),
            "resident file should have data_byte_offset = None"
        );
    }

    #[test]
    fn first_lcn_from_run_list_single_run() {
        // header 0x11: 1 length byte, 1 offset byte â†’ len=1, off=5 (LCN 5)
        let run_list = [0x11u8, 0x01, 0x05, 0x00];
        assert_eq!(first_lcn_from_run_list(&run_list), Some(5));
    }

    #[test]
    fn first_lcn_from_run_list_sparse_returns_none() {
        // Sparse run: off_bytes = 0 (upper nibble of header is 0)
        let run_list = [0x01u8, 0x01, 0x00];
        assert_eq!(first_lcn_from_run_list(&run_list), None);
    }

    #[test]
    fn first_lcn_from_run_list_empty_returns_none() {
        assert_eq!(first_lcn_from_run_list(&[]), None);
        assert_eq!(first_lcn_from_run_list(&[0x00]), None);
    }

    /// Feeding a 10-byte device to NtfsParser::new must return Err, not panic.
    #[test]
    fn truncated_boot_sector_returns_err_not_panic() {
        let dev = Arc::new(MockBlockDevice::new(vec![0u8; 10], 512));
        let result = NtfsParser::new(dev);
        assert!(result.is_err(), "expected Err on 10-byte device, got Ok");
    }

    /// A device large enough to read the boot sector but with an invalid OEM ID
    /// must return Err(InvalidStructure), not panic.
    #[test]
    fn invalid_oem_id_returns_err() {
        let data = vec![0u8; 512]; // all zeros â€” OEM ID will not match "NTFS    "
        let dev = Arc::new(MockBlockDevice::new(data, 512));
        let result = NtfsParser::new(dev);
        assert!(result.is_err(), "expected Err for invalid NTFS OEM ID");
    }
}
