//! NTFS record-parsing helpers split from `ntfs.rs`.
//!
//! Contains: `apply_fixup`, `parse_file_info`, `first_lcn_from_run_list`,
//! `read_run_list`, `mft_record_count`.

use std::io::Write;
use std::sync::Arc;

use tracing::{debug, warn};

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::read_bytes;

/// Maximum bytes written per sparse-run iteration to avoid large heap allocations.
const MAX_SPARSE_ZEROS: u64 = 64 * 1024;

// Re-export constants used by both ntfs.rs and here.
pub(crate) const FILE_SIG: &[u8; 4] = b"FILE";
pub(crate) const ATTR_STD_INFO: u32 = 0x10;
pub(crate) const ATTR_FILE_NAME: u32 = 0x30;
pub(crate) const ATTR_DATA: u32 = 0x80;
pub(crate) const ATTR_END: u32 = 0xFFFF_FFFF;

// ── Record helpers ────────────────────────────────────────────────────────────

/// Apply the NTFS update-sequence fixup to a raw MFT record.
///
/// Returns the corrected record.  If the sequence check fails (e.g. the record
/// is corrupt or unformatted), the record is returned unmodified with a warning.
pub(crate) fn apply_fixup(mut record: Vec<u8>) -> Vec<u8> {
    if record.len() < 8 {
        return record;
    }
    // Safety: record.len() >= 8 checked above.
    let usa_offset = u16::from_le_bytes([record[4], record[5]]) as usize;
    let usa_count = u16::from_le_bytes([record[6], record[7]]) as usize;

    if usa_count < 2 || usa_offset + usa_count * 2 > record.len() {
        return record;
    }

    // Safety: usa_offset + 4 <= record.len() guaranteed by the count check above.
    let seq = u16::from_le_bytes([record[usa_offset], record[usa_offset + 1]]);

    for i in 1..usa_count {
        let sector_end = i * 512 - 2;
        if sector_end + 2 > record.len() {
            break;
        }

        // Safety: sector_end + 2 <= record.len() checked above.
        let actual = u16::from_le_bytes([record[sector_end], record[sector_end + 1]]);
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

/// Parse `$STANDARD_INFORMATION` (attribute type `0x10`) from a raw FILE record.
///
/// Returns `(created_unix_secs, modified_unix_secs)`.  Either value is `None`
/// when the corresponding Windows FILETIME field is zero or predates the Unix
/// epoch (1970-01-01).
pub(crate) fn parse_standard_info(raw: &[u8]) -> Option<(Option<u64>, Option<u64>)> {
    // 100-nanosecond intervals between 1601-01-01 and 1970-01-01.
    const FILETIME_EPOCH: u64 = 116_444_736_000_000_000;

    if raw.len() < 22 {
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

        if type_id == ATTR_STD_INFO && raw[pos + 8] == 0 {
            // Resident $STANDARD_INFORMATION.  Value offset at bytes 20-21 of
            // the attribute header; value must be at least 16 bytes to hold
            // the created and modified FILETIME fields.
            if pos + 22 > raw.len() {
                break;
            }
            let val_off = u16::from_le_bytes(raw[pos + 20..pos + 22].try_into().ok()?) as usize;
            let val_start = pos + val_off;
            if val_start + 16 > raw.len() {
                break;
            }
            let created_ft = u64::from_le_bytes(raw[val_start..val_start + 8].try_into().ok()?);
            let modified_ft =
                u64::from_le_bytes(raw[val_start + 8..val_start + 16].try_into().ok()?);

            let to_unix = |ft: u64| ft.checked_sub(FILETIME_EPOCH).map(|d| d / 10_000_000);
            return Some((to_unix(created_ft), to_unix(modified_ft)));
        }

        pos += attr_len;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal raw FILE record with a resident $STANDARD_INFORMATION
    /// attribute whose created and modified FILETIME equal `created_ft` and
    /// `modified_ft` respectively.
    fn make_si_record(created_ft: u64, modified_ft: u64) -> Vec<u8> {
        let mut raw = vec![0u8; 256];
        raw[0..4].copy_from_slice(b"FILE");
        // usa_offset = 0x30, usa_count = 1 (no sector fixup needed)
        raw[4..6].copy_from_slice(&0x30u16.to_le_bytes());
        raw[6..8].copy_from_slice(&1u16.to_le_bytes());
        // First attribute starts at 0x38
        raw[20..22].copy_from_slice(&0x38u16.to_le_bytes());

        // $STANDARD_INFORMATION at offset 0x38
        //   attr header = 24 bytes, value = 48 bytes  →  total = 72 (0x48)
        let si = 0x38usize;
        raw[si..si + 4].copy_from_slice(&ATTR_STD_INFO.to_le_bytes()); // type
        raw[si + 4..si + 8].copy_from_slice(&0x48u32.to_le_bytes()); // attr_len = 72
        raw[si + 8] = 0; // resident
        raw[si + 16..si + 20].copy_from_slice(&48u32.to_le_bytes()); // val_len
        raw[si + 20..si + 22].copy_from_slice(&24u16.to_le_bytes()); // val_off = 24
                                                                     // value[0..8]  = created  FILETIME
                                                                     // value[8..16] = modified FILETIME
        let v = si + 24;
        raw[v..v + 8].copy_from_slice(&created_ft.to_le_bytes());
        raw[v + 8..v + 16].copy_from_slice(&modified_ft.to_le_bytes());

        // ATTR_END marker
        let end = si + 0x48;
        raw[end..end + 4].copy_from_slice(&ATTR_END.to_le_bytes());
        raw
    }

    #[test]
    fn parse_standard_info_extracts_timestamps() {
        // Unix timestamp 946_684_800 = 2000-01-01 00:00:00 UTC
        // Windows FILETIME = 946_684_800 * 10_000_000 + 116_444_736_000_000_000
        const UNIX_TS: u64 = 946_684_800;
        const FT: u64 = UNIX_TS * 10_000_000 + 116_444_736_000_000_000;

        let raw = make_si_record(FT, FT);
        let result = parse_standard_info(&raw);
        assert!(result.is_some(), "expected Some");
        let (created, modified) = result.unwrap();
        assert_eq!(created, Some(UNIX_TS));
        assert_eq!(modified, Some(UNIX_TS));
    }

    #[test]
    fn parse_standard_info_zero_filetime_yields_none() {
        // FILETIME of 0 predates Unix epoch — both values must be None.
        let raw = make_si_record(0, 0);
        let (created, modified) = parse_standard_info(&raw).unwrap();
        assert_eq!(created, None);
        assert_eq!(modified, None);
    }

    #[test]
    fn parse_standard_info_missing_returns_none() {
        // A record with no $STANDARD_INFORMATION attribute (ATTR_END immediately).
        let mut raw = vec![0u8; 256];
        raw[0..4].copy_from_slice(b"FILE");
        raw[4..6].copy_from_slice(&0x30u16.to_le_bytes());
        raw[6..8].copy_from_slice(&1u16.to_le_bytes());
        raw[20..22].copy_from_slice(&0x38u16.to_le_bytes());
        raw[0x38..0x3C].copy_from_slice(&ATTR_END.to_le_bytes());
        assert_eq!(parse_standard_info(&raw), None);
    }
}
/// Extract `(win32_name, parent_mft_ref, data_size, first_lcn)` from a FILE record.
///
/// Prefers the Win32 namespace (namespace = 1 or 3) for the filename.
/// `first_lcn` is `None` for resident (tiny) files or when the run-list is absent.
/// Returns `None` when no `$FILE_NAME` attribute is found.
pub(crate) fn parse_file_info(raw: &[u8]) -> Option<(String, u64, u64, Option<u64>)> {
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
pub(crate) fn first_lcn_from_run_list(run_list: &[u8]) -> Option<u64> {
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
pub(crate) fn read_run_list(
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
            // Sparse run — write zeros in capped chunks to avoid large allocations.
            let zeros_needed = (run_len * cluster_size).min(data_size - written);
            let chunk = vec![0u8; MAX_SPARSE_ZEROS.min(zeros_needed) as usize];
            let mut remaining = zeros_needed;
            while remaining > 0 {
                let n = MAX_SPARSE_ZEROS.min(remaining) as usize;
                writer
                    .write_all(&chunk[..n])
                    .map_err(|e| FilesystemError::InvalidStructure {
                        context: "read_run_list sparse write",
                        reason: e.to_string(),
                    })?;
                remaining -= n as u64;
            }
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
pub(crate) fn mft_record_count(
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
