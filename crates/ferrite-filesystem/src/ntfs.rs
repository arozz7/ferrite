//! Minimum-viable NTFS read-only parser.
//!
//! Parses the boot sector to locate the MFT, then scans MFT records to
//! enumerate files and directories.  Supports resident and non-resident DATA
//! attributes (run-list decoding for non-resident reads).

use std::io::Write;
use std::sync::Arc;

use tracing::{debug, trace, warn};

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::read_bytes;
use crate::{FileEntry, FilesystemParser, FilesystemType};

// ── Constants ─────────────────────────────────────────────────────────────────

const NTFS_OEM_ID: &[u8] = b"NTFS    ";
const FILE_SIG: &[u8; 4] = b"FILE";

// Attribute type identifiers
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

// MFT record number for the root directory
const ROOT_MFT_RECORD: u64 = 5;

// Maximum MFT records to scan (safety cap for very large volumes)
const MAX_SCAN_RECORDS: u64 = 65_536;

// ── Public struct ─────────────────────────────────────────────────────────────

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

        let bytes_per_sector = u16::from_le_bytes(boot[11..13].try_into().unwrap()) as u64;
        let sectors_per_cluster = boot[13] as u64;
        let mft_cluster = u64::from_le_bytes(boot[48..56].try_into().unwrap());

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

    // ── MFT helpers ───────────────────────────────────────────────────────────

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

            let flags = u16::from_le_bytes(raw[22..24].try_into().unwrap());
            let in_use = flags & 1 != 0;
            let is_dir = flags & 2 != 0;

            if let Some((name, parent_ref, data_size, first_lcn)) = parse_file_info(&raw) {
                if filter(parent_ref, in_use, is_dir) {
                    // Convert first LCN to a byte offset within the device.
                    // `None` for directories and resident (tiny) files.
                    let data_byte_offset = if is_dir {
                        None
                    } else {
                        first_lcn.map(|lcn| lcn * self.cluster_size)
                    };
                    entries.push(FileEntry {
                        name: name.clone(),
                        path: format!("/{name}"),
                        size: data_size,
                        is_dir,
                        is_deleted: !in_use,
                        created: None,
                        modified: None,
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

// ── FilesystemParser impl ─────────────────────────────────────────────────────

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

        let first_attr_offset = u16::from_le_bytes(raw[20..22].try_into().unwrap()) as usize;
        let mut pos = first_attr_offset;

        while pos + 8 <= raw.len() {
            let type_id = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap());
            if type_id == ATTR_END {
                break;
            }
            let attr_len = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if attr_len == 0 || pos + attr_len > raw.len() {
                break;
            }

            if type_id == ATTR_DATA {
                let non_resident = raw[pos + 8];
                if non_resident == 0 {
                    // Resident: data is inside the MFT record.
                    let val_len =
                        u32::from_le_bytes(raw[pos + 16..pos + 20].try_into().unwrap()) as usize;
                    let val_off =
                        u16::from_le_bytes(raw[pos + 20..pos + 22].try_into().unwrap()) as usize;
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
                    let data_size =
                        u64::from_le_bytes(raw[pos + 0x30..pos + 0x38].try_into().unwrap());
                    let rl_off = u16::from_le_bytes(raw[pos + 0x20..pos + 0x22].try_into().unwrap())
                        as usize;
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

// ── Attribute / record helpers ────────────────────────────────────────────────

/// Apply the NTFS update-sequence fixup to a raw MFT record.
///
/// Returns the corrected record.  If the sequence check fails (e.g. the record
/// is corrupt or unformatted), the record is returned unmodified with a warning.
fn apply_fixup(mut record: Vec<u8>) -> Vec<u8> {
    if record.len() < 8 {
        return record;
    }
    let usa_offset = u16::from_le_bytes(record[4..6].try_into().unwrap()) as usize;
    let usa_count = u16::from_le_bytes(record[6..8].try_into().unwrap()) as usize;

    if usa_count < 2 || usa_offset + usa_count * 2 > record.len() {
        return record;
    }

    let seq = u16::from_le_bytes(record[usa_offset..usa_offset + 2].try_into().unwrap());

    for i in 1..usa_count {
        let sector_end = i * 512 - 2;
        if sector_end + 2 > record.len() {
            break;
        }
        let actual = u16::from_le_bytes(record[sector_end..sector_end + 2].try_into().unwrap());
        if actual != seq {
            warn!(
                record_seq = seq,
                sector = i,
                found = actual,
                "NTFS fixup mismatch — record may be corrupt"
            );
            continue;
        }
        // Replace with saved value
        let saved_offset = usa_offset + i * 2;
        let saved = [record[saved_offset], record[saved_offset + 1]];
        record[sector_end] = saved[0];
        record[sector_end + 1] = saved[1];
    }

    record
}

/// Extract `(win32_name, parent_mft_ref, data_size, first_lcn)` from a FILE record.
///
/// Prefers the Win32 namespace (namespace = 1 or 3) for the filename.
/// `first_lcn` is `None` for resident (tiny) files or when the run-list is absent.
/// Returns `None` when no `$FILE_NAME` attribute is found.
fn parse_file_info(raw: &[u8]) -> Option<(String, u64, u64, Option<u64>)> {
    let first_attr = u16::from_le_bytes(raw[20..22].try_into().ok()?) as usize;
    let mut pos = first_attr;

    let mut best_name: Option<(String, u64)> = None; // (name, parent_ref)
    let mut data_size: u64 = 0;
    let mut first_lcn: Option<u64> = None;

    while pos + 8 <= raw.len() {
        let type_id = u32::from_le_bytes(raw[pos..pos + 4].try_into().ok()?);
        if type_id == ATTR_END {
            break;
        }
        let attr_len = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().ok()?) as usize;
        if attr_len == 0 || pos + attr_len > raw.len() {
            break;
        }

        if type_id == ATTR_FILE_NAME && raw[pos + 8] == 0 {
            // Resident $FILE_NAME
            let val_off = u16::from_le_bytes(raw[pos + 20..pos + 22].try_into().ok()?) as usize;
            let val_start = pos + val_off;
            if val_start + 0x42 <= raw.len() {
                let parent_ref = u64::from_le_bytes(raw[val_start..val_start + 8].try_into().ok()?)
                    & 0x0000_FFFF_FFFF_FFFF;
                let name_len = raw[val_start + 0x40] as usize;
                let namespace = raw[val_start + 0x41];
                let name_start = val_start + 0x42;
                let name_end = name_start + name_len * 2;
                if name_end <= raw.len() {
                    let name_utf16: Vec<u16> = raw[name_start..name_end]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let name = String::from_utf16_lossy(&name_utf16).to_string();

                    // Prefer Win32 (1) or Win32&DOS (3) namespace over POSIX (0) / DOS (2)
                    let is_preferred = matches!(namespace, 1 | 3);
                    if best_name.is_none() || is_preferred {
                        best_name = Some((name, parent_ref));
                    }
                }
            }
        } else if type_id == ATTR_DATA {
            if raw[pos + 8] == 0 {
                // Resident — data lives inside the MFT record; no LCN
                let val_len = u32::from_le_bytes(raw[pos + 16..pos + 20].try_into().ok()?) as u64;
                if data_size == 0 {
                    data_size = val_len;
                }
            } else {
                // Non-resident: real size at +0x30, run-list at +0x20 (relative offset)
                if pos + 0x40 <= raw.len() {
                    let real_size =
                        u64::from_le_bytes(raw[pos + 0x30..pos + 0x38].try_into().ok()?);
                    if data_size == 0 {
                        data_size = real_size;
                    }
                    let rl_off =
                        u16::from_le_bytes(raw[pos + 0x20..pos + 0x22].try_into().ok()?) as usize;
                    let rl_start = pos + rl_off;
                    if rl_start < raw.len() && first_lcn.is_none() {
                        first_lcn = first_lcn_from_run_list(&raw[rl_start..]);
                    }
                }
            }
        }

        pos += attr_len;
    }

    let (name, parent_ref) = best_name?;
    Some((name, parent_ref, data_size, first_lcn))
}

/// Extract the first LCN (Logical Cluster Number) from an NTFS run-list.
///
/// Only decodes the first run entry.  Returns `None` if the run-list is empty,
/// starts with a terminator, or the offset field is absent (sparse run).
fn first_lcn_from_run_list(run_list: &[u8]) -> Option<u64> {
    if run_list.is_empty() || run_list[0] == 0 {
        return None;
    }
    let header = run_list[0];
    let len_bytes = (header & 0x0F) as usize;
    let off_bytes = ((header >> 4) & 0x0F) as usize;
    if off_bytes == 0 {
        return None; // sparse run — no physical location
    }
    if 1 + len_bytes + off_bytes > run_list.len() {
        return None;
    }
    let mut run_off: i64 = 0;
    for i in 0..off_bytes {
        run_off |= (run_list[1 + len_bytes + i] as i64) << (i * 8);
    }
    // Sign-extend
    if run_list[1 + len_bytes + off_bytes - 1] & 0x80 != 0 {
        run_off |= -1i64 << (off_bytes * 8);
    }
    if run_off <= 0 {
        None
    } else {
        Some(run_off as u64)
    }
}

/// Decode an NTFS data run list and stream the file content to `writer`.
fn read_run_list(
    device: &dyn BlockDevice,
    run_list: &[u8],
    cluster_size: u64,
    data_size: u64,
    writer: &mut dyn Write,
) -> Result<u64> {
    let mut pos = 0usize;
    let mut prev_lcn: i64 = 0;
    let mut written: u64 = 0;

    while pos < run_list.len() {
        let header = run_list[pos];
        if header == 0 {
            break;
        }
        pos += 1;

        let len_bytes = (header & 0x0F) as usize;
        let off_bytes = ((header >> 4) & 0x0F) as usize;

        if pos + len_bytes + off_bytes > run_list.len() {
            break;
        }

        // Run length (unsigned)
        let mut run_len: u64 = 0;
        for i in 0..len_bytes {
            run_len |= (run_list[pos + i] as u64) << (i * 8);
        }
        pos += len_bytes;

        // Run offset (signed, relative to previous LCN)
        let mut run_off: i64 = 0;
        if off_bytes > 0 {
            for i in 0..off_bytes {
                run_off |= (run_list[pos + i] as i64) << (i * 8);
            }
            // Sign-extend
            if run_list[pos + off_bytes - 1] & 0x80 != 0 {
                run_off |= -1i64 << (off_bytes * 8);
            }
            prev_lcn += run_off;
            pos += off_bytes;
        } else {
            pos += off_bytes;
        }

        if off_bytes == 0 {
            // Sparse run — write zeros
            let zeros_needed = (run_len * cluster_size).min(data_size - written);
            let zeros = vec![0u8; zeros_needed as usize];
            writer
                .write_all(&zeros)
                .map_err(|e| FilesystemError::InvalidStructure {
                    context: "read_run_list sparse write",
                    reason: e.to_string(),
                })?;
            written += zeros_needed;
        } else {
            let lcn = prev_lcn as u64;
            for c in 0..run_len {
                if written >= data_size {
                    break;
                }
                let offset = (lcn + c) * cluster_size;
                let data = read_bytes(device, offset, cluster_size as usize)?;
                let remaining = (data_size - written) as usize;
                let to_write = data.len().min(remaining);
                writer.write_all(&data[..to_write]).map_err(|e| {
                    FilesystemError::InvalidStructure {
                        context: "read_run_list write",
                        reason: e.to_string(),
                    }
                })?;
                written += to_write as u64;
            }
        }

        if written >= data_size {
            break;
        }
    }

    Ok(written)
}

/// Determine how many MFT records exist by reading MFT record 0's DATA attribute.
fn mft_record_count(
    device: &Arc<dyn BlockDevice>,
    mft_start_byte: u64,
    mft_record_size: u64,
    _cluster_size: u64,
) -> Option<u64> {
    let raw_rec = read_bytes(device.as_ref(), mft_start_byte, mft_record_size as usize).ok()?;
    let raw = apply_fixup(raw_rec);

    if raw.len() < 48 || &raw[0..4] != FILE_SIG {
        return None;
    }

    let first_attr = u16::from_le_bytes(raw[20..22].try_into().ok()?) as usize;
    let mut pos = first_attr;

    while pos + 8 <= raw.len() {
        let type_id = u32::from_le_bytes(raw[pos..pos + 4].try_into().ok()?);
        if type_id == ATTR_END {
            break;
        }
        let attr_len = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().ok()?) as usize;
        if attr_len == 0 || pos + attr_len > raw.len() {
            break;
        }

        if type_id == ATTR_DATA && raw[pos + 8] != 0 {
            // Non-resident DATA: real size at +0x30
            if pos + 0x38 <= raw.len() {
                let mft_bytes = u64::from_le_bytes(raw[pos + 0x30..pos + 0x38].try_into().ok()?);
                debug!(mft_bytes, mft_record_size, "derived MFT size from record 0");
                return Some(mft_bytes / mft_record_size);
            }
        }

        pos += attr_len;
    }

    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    // ── Image builder helpers ─────────────────────────────────────────────────

    /// Build a minimal NTFS boot sector.
    ///
    /// - bytes_per_sector   = 512
    /// - sectors_per_cluster = 1  →  cluster_size = 512
    /// - mft_cluster_number  = 8  →  MFT starts at byte 4096
    /// - clusters_per_mft_record = 0xF6 (-10)  →  record_size = 1024
    fn boot_sector() -> [u8; 512] {
        let mut s = [0u8; 512];
        s[3..11].copy_from_slice(b"NTFS    ");
        s[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes_per_sector
        s[13] = 1; // sectors_per_cluster
        s[48..56].copy_from_slice(&8u64.to_le_bytes()); // mft_cluster
        s[64] = 0xF6_u8; // clusters_per_mft_record → 1024 bytes/record
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

        // ── Fixed header ──────────────────────────────────────────────────────
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

        // ── Attributes start at 0x38 ──────────────────────────────────────────
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

        // $DATA (type 0x80, resident) — only if non-empty
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
    /// Records 0–5 are system placeholders; record 5 = root; record 6 = test file.
    fn build_image() -> MockBlockDevice {
        const SECTOR: usize = 512;
        const MFT_START: usize = 4096; // cluster 8 * 512
        const REC_SIZE: usize = 1024;

        let mut dev = MockBlockDevice::zeroed(16384, SECTOR as u32);

        // Boot sector
        let boot = boot_sector();
        dev.write_sector(0, &boot);

        // Write a dummy FILE record for MFT records 0–4 (system files)
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

    // ── Tests ─────────────────────────────────────────────────────────────────

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
        // usa_count = 1 means no sectors need fixing — record should be returned unchanged.
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
        // Resident files live inside the MFT record — no physical cluster offset.
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
        // header 0x11: 1 length byte, 1 offset byte → len=1, off=5 (LCN 5)
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
}
