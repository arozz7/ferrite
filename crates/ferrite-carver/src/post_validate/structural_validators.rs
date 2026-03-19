//! File-based structural validators for audio, binary, and image formats.
//!
//! These validators open the extracted file and seek through its internal
//! structure to verify structural integrity beyond what fixed head+tail
//! buffers can detect.
//!
//! Validators in this file:
//! - [`validate_flac_file`]  — FLAC metadata block chain walk
//! - [`validate_elf_file`]   — ELF program header bounds check
//! - [`validate_regf_file`]  — Registry hive hbin block check
//! - [`validate_tiff_file`]  — TIFF IFD chain walk (covers all TIFF-based RAW)

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::CarveQuality;

// ── Byte-order helpers ─────────────────────────────────────────────────────

#[inline]
fn read_u16(b: &[u8], le: bool) -> u16 {
    if le {
        u16::from_le_bytes([b[0], b[1]])
    } else {
        u16::from_be_bytes([b[0], b[1]])
    }
}

#[inline]
fn read_u32(b: &[u8], le: bool) -> u32 {
    if le {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
}

#[inline]
fn read_u64(b: &[u8], le: bool) -> u64 {
    let arr: [u8; 8] = b[0..8].try_into().unwrap_or([0u8; 8]);
    if le {
        u64::from_le_bytes(arr)
    } else {
        u64::from_be_bytes(arr)
    }
}

// ── validate_flac_file ─────────────────────────────────────────────────────

/// Validate a carved FLAC audio file by walking its metadata block chain.
///
/// FLAC structure after the `fLaC` magic:
/// - Metadata blocks: 4-byte header each.
///   - Bit 7: `LAST_METADATA_BLOCK` flag.
///   - Bits 6–0: block type (`0`–`6` are defined; `7`–`126` reserved; `127`
///     is invalid).
///   - Bits 23–8: block length (u24 BE).
/// - First block MUST be `STREAMINFO` (type 0) with length exactly 34.
/// - Walk continues until the LAST bit is set or 16 blocks are checked.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_flac_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    let file_size = match f.seek(SeekFrom::End(0)) {
        Ok(s) => s,
        Err(_) => return CarveQuality::Unknown,
    };
    if f.seek(SeekFrom::Start(0)).is_err() {
        return CarveQuality::Corrupt;
    }

    // Verify fLaC magic.
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return CarveQuality::Corrupt;
    }
    if &magic != b"fLaC" {
        return CarveQuality::Corrupt;
    }

    let mut pos: u64 = 4;
    let mut block_count = 0u32;
    let mut first_block = true;

    loop {
        if pos + 4 > file_size {
            return CarveQuality::Corrupt;
        }
        if f.seek(SeekFrom::Start(pos)).is_err() {
            return CarveQuality::Corrupt;
        }
        let mut blk_hdr = [0u8; 4];
        if f.read_exact(&mut blk_hdr).is_err() {
            return CarveQuality::Corrupt;
        }

        let is_last = blk_hdr[0] & 0x80 != 0;
        let block_type = blk_hdr[0] & 0x7F;
        let block_len = u32::from_be_bytes([0, blk_hdr[1], blk_hdr[2], blk_hdr[3]]) as u64;

        // Types 7–126 are reserved; 127 is the invalid/invalid sentinel.
        if block_type > 6 {
            return CarveQuality::Corrupt;
        }

        if first_block {
            // First block must be STREAMINFO (type 0) with fixed length 34.
            if block_type != 0 || block_len != 34 {
                return CarveQuality::Corrupt;
            }
            first_block = false;
        }

        block_count += 1;
        pos = match pos.checked_add(4 + block_len) {
            Some(next) if next <= file_size => next,
            _ => return CarveQuality::Corrupt,
        };

        if is_last || block_count >= 16 {
            break;
        }
    }

    CarveQuality::Complete
}

// ── validate_elf_file ──────────────────────────────────────────────────────

/// Validate a carved ELF binary (executable, shared library, or object file).
///
/// Checks:
/// 1. ELF magic `\x7fELF` + valid `EI_CLASS` (1=32-bit, 2=64-bit) and
///    `EI_DATA` (1=LE, 2=BE).
/// 2. `e_phnum` in `[0, 256]` and `e_phentsize` == 32 (32-bit) or 56 (64-bit).
/// 3. For up to 16 program headers: `p_offset + p_filesz ≤ file_size`.
///    Requires more than half of checked headers to pass.
///
/// Object files (`.o`) may have `e_phnum == 0`; these are accepted as
/// [`CarveQuality::Complete`] without sampling program headers.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_elf_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    let file_size = match f.seek(SeekFrom::End(0)) {
        Ok(s) => s,
        Err(_) => return CarveQuality::Unknown,
    };
    if f.seek(SeekFrom::Start(0)).is_err() {
        return CarveQuality::Corrupt;
    }

    // Read 16-byte ident.
    let mut ident = [0u8; 16];
    if f.read_exact(&mut ident).is_err() {
        return CarveQuality::Corrupt;
    }
    if &ident[0..4] != b"\x7fELF" {
        return CarveQuality::Corrupt;
    }

    let class = ident[4]; // 1=32-bit, 2=64-bit
    let data = ident[5]; // 1=LE, 2=BE
    if class != 1 && class != 2 {
        return CarveQuality::Corrupt;
    }
    if data != 1 && data != 2 {
        return CarveQuality::Corrupt;
    }
    let le = data == 1;

    // Read full ELF header to get phoff, phentsize, phnum.
    let hdr_size: u64 = if class == 1 { 52 } else { 64 };
    if f.seek(SeekFrom::Start(0)).is_err() {
        return CarveQuality::Corrupt;
    }
    let mut hdr = vec![0u8; hdr_size as usize];
    if f.read_exact(&mut hdr).is_err() {
        return CarveQuality::Corrupt;
    }

    let (phoff, phentsize, phnum): (u64, u64, u64) = if class == 1 {
        // 32-bit: e_phoff @28 (u32), e_phentsize @42 (u16), e_phnum @44 (u16)
        (
            read_u32(&hdr[28..32], le) as u64,
            read_u16(&hdr[42..44], le) as u64,
            read_u16(&hdr[44..46], le) as u64,
        )
    } else {
        // 64-bit: e_phoff @32 (u64), e_phentsize @54 (u16), e_phnum @56 (u16)
        (
            read_u64(&hdr[32..40], le),
            read_u16(&hdr[54..56], le) as u64,
            read_u16(&hdr[56..58], le) as u64,
        )
    };

    // Object files may have no program headers — structurally valid.
    if phnum == 0 {
        return CarveQuality::Complete;
    }
    if phnum > 256 {
        return CarveQuality::Corrupt;
    }

    let expected_phentsize: u64 = if class == 1 { 32 } else { 56 };
    if phentsize != expected_phentsize {
        return CarveQuality::Corrupt;
    }
    if phoff.saturating_add(phnum * phentsize) > file_size {
        return CarveQuality::Corrupt;
    }

    // Sample up to 16 program headers.
    let to_check = phnum.min(16) as u32;
    let mut valid = 0u32;

    for i in 0..to_check as u64 {
        let off = phoff + i * phentsize;
        if f.seek(SeekFrom::Start(off)).is_err() {
            break;
        }
        let mut ph = vec![0u8; phentsize as usize];
        if f.read_exact(&mut ph).is_err() {
            break;
        }

        // p_offset and p_filesz positions differ between 32-bit and 64-bit.
        let (p_offset, p_filesz) = if class == 1 {
            // 32-bit: p_offset @4, p_filesz @16
            (
                read_u32(&ph[4..8], le) as u64,
                read_u32(&ph[16..20], le) as u64,
            )
        } else {
            // 64-bit: p_offset @8, p_filesz @32
            (read_u64(&ph[8..16], le), read_u64(&ph[32..40], le))
        };

        if p_filesz == 0 || p_offset.saturating_add(p_filesz) <= file_size {
            valid += 1;
        }
    }

    if valid * 2 > to_check {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

// ── validate_regf_file ─────────────────────────────────────────────────────

/// Validate a carved Windows Registry hive (REGF format).
///
/// Structure:
/// - File header: `regf` magic at offset 0.
/// - `hbin` blocks from offset 4096 onward (each multiple of 4096 bytes).
///
/// Checks the `regf` header magic and the `hbin` magic at offset 4096.
/// A real hive always has at least one `hbin` block immediately after the
/// 4096-byte file header.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_regf_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    let file_size = match f.seek(SeekFrom::End(0)) {
        Ok(s) => s,
        Err(_) => return CarveQuality::Unknown,
    };
    if f.seek(SeekFrom::Start(0)).is_err() {
        return CarveQuality::Corrupt;
    }

    // Verify regf magic.
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return CarveQuality::Corrupt;
    }
    if &magic != b"regf" {
        return CarveQuality::Corrupt;
    }

    // First hbin must be at offset 4096 (immediately after the file header).
    if file_size < 4096 + 4 {
        return CarveQuality::Corrupt;
    }
    if f.seek(SeekFrom::Start(4096)).is_err() {
        return CarveQuality::Corrupt;
    }
    let mut hbin_magic = [0u8; 4];
    if f.read_exact(&mut hbin_magic).is_err() {
        return CarveQuality::Corrupt;
    }
    if &hbin_magic != b"hbin" {
        return CarveQuality::Corrupt;
    }

    CarveQuality::Complete
}

// ── validate_tiff_file ─────────────────────────────────────────────────────

/// Validate a carved TIFF or TIFF-based RAW file by walking its IFD chain.
///
/// Covers: TIFF (LE/BE), NEF, ARW, CR2, RW2, ORF, PEF, SR2, DCR — all of
/// which use the TIFF IFD container format.
///
/// Checks:
/// 1. Byte-order mark (`II` = little-endian, `MM` = big-endian) and TIFF
///    magic word (`0x002A`; or `0x0055` for Panasonic RW2).
/// 2. IFD0 offset must be non-zero and within the file.
/// 3. For each IFD (up to 16, cycle-detected): entry count in `[1, 1000]`;
///    each entry's type code in `[1, 12]`; if the value is stored externally
///    (`data_bytes > 4`), the external pointer + data size must be ≤ file
///    size.
/// 4. Requires more than half of all visited IFD entries to be structurally
///    valid.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_tiff_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    let file_size = match f.seek(SeekFrom::End(0)) {
        Ok(s) => s,
        Err(_) => return CarveQuality::Unknown,
    };
    if f.seek(SeekFrom::Start(0)).is_err() {
        return CarveQuality::Corrupt;
    }

    // Read 8-byte TIFF header.
    let mut hdr = [0u8; 8];
    if f.read_exact(&mut hdr).is_err() {
        return CarveQuality::Corrupt;
    }

    let le = match [hdr[0], hdr[1]] {
        [0x49, 0x49] => true,  // "II" little-endian
        [0x4D, 0x4D] => false, // "MM" big-endian
        _ => return CarveQuality::Corrupt,
    };

    // Magic word: 0x002A for standard TIFF, 0x0055 for Panasonic RW2.
    let magic_word = read_u16(&hdr[2..4], le);
    if magic_word != 0x002A && magic_word != 0x0055 {
        return CarveQuality::Corrupt;
    }

    let ifd0_off = read_u32(&hdr[4..8], le) as u64;
    if ifd0_off == 0 || ifd0_off >= file_size {
        return CarveQuality::Corrupt;
    }

    let mut total_entries = 0u32;
    let mut valid_entries = 0u32;
    let mut ifd_queue: Vec<u64> = vec![ifd0_off];
    let mut visited = std::collections::HashSet::new();

    while let Some(ifd_off) = ifd_queue.pop() {
        if ifd_off == 0 || ifd_off >= file_size || visited.contains(&ifd_off) || visited.len() >= 16
        {
            continue;
        }
        visited.insert(ifd_off);

        if f.seek(SeekFrom::Start(ifd_off)).is_err() {
            continue;
        }
        let mut cnt_b = [0u8; 2];
        if f.read_exact(&mut cnt_b).is_err() {
            continue;
        }
        let entry_count = read_u16(&cnt_b, le) as u64;
        if entry_count == 0 || entry_count > 1000 {
            continue;
        }

        for i in 0..entry_count.min(64) {
            let ep = ifd_off + 2 + i * 12;
            if ep + 12 > file_size {
                break;
            }
            if f.seek(SeekFrom::Start(ep)).is_err() {
                break;
            }
            let mut entry = [0u8; 12];
            if f.read_exact(&mut entry).is_err() {
                break;
            }

            let tag = read_u16(&entry[0..2], le);
            let type_id = read_u16(&entry[2..4], le);
            let count = read_u32(&entry[4..8], le) as u64;

            total_entries += 1;

            // Type code must be 1–12; anything else is invalid.
            let type_sz: u64 = match type_id {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 => 4,
                5 | 10 | 12 => 8,
                _ => {
                    // Invalid type code — counts as an invalid entry.
                    continue;
                }
            };

            let data_bytes = count.saturating_mul(type_sz);
            if data_bytes > 4 {
                // External pointer: must be within file.
                let ext_off = read_u32(&entry[8..12], le) as u64;
                if ext_off.saturating_add(data_bytes) <= file_size {
                    valid_entries += 1;
                }
                // Follow SubIFD pointer (tag 0x014A).
                if tag == 0x014A && ext_off > 0 && ext_off < file_size {
                    ifd_queue.push(ext_off);
                }
            } else {
                // Inline value — always valid once type is confirmed.
                valid_entries += 1;
            }
        }

        // Follow next-IFD link.
        let next_pos = ifd_off + 2 + entry_count * 12;
        if next_pos + 4 <= file_size {
            if f.seek(SeekFrom::Start(next_pos)).is_err() {
                continue;
            }
            let mut nb = [0u8; 4];
            if f.read_exact(&mut nb).is_err() {
                continue;
            }
            let next = read_u32(&nb, le) as u64;
            if next > 0 && next < file_size {
                ifd_queue.push(next);
            }
        }
    }

    if total_entries == 0 {
        return CarveQuality::Corrupt;
    }

    if valid_entries * 2 > total_entries {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}
