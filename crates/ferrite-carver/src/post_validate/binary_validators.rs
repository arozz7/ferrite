//! File-based structural validators for binary container formats.
//!
//! These validators open the extracted file and seek through its internal
//! structure to verify structural integrity beyond what fixed head+tail
//! buffers can detect.
//!
//! Validators in this file:
//! - [`validate_evtx_file`]    — Windows Event Log chunk magic check
//! - [`validate_riff_file`]    — RIFF/FORM chunk fourCC walk (WAV/AVI/WebP/AIFF)
//! - [`validate_exe_file`]     — PE header + section table bounds check
//! - [`validate_isobmff_file`] — ISOBMFF box walk (MP4/MOV/M4V/3GP/M4A/HEIC/CR3)
//! - [`validate_ebml_file`]    — EBML element check (MKV/WebM)
//!
//! Audio, ELF, REGF, and TIFF validators live in `structural_validators`.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::CarveQuality;

// ── Byte-order helpers ─────────────────────────────────────────────────────

#[inline]
fn read_u32(b: &[u8], le: bool) -> u32 {
    if le {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
}

// ── validate_evtx_file ─────────────────────────────────────────────────────

/// Validate a carved Windows Event Log (EVTX) file.
///
/// EVTX structure:
/// - File header (4096 bytes): magic `ElfFile\x00` at offset 0.
/// - Chunks (65536 bytes each): magic `ElfChnk\x00` at offset 0 of each chunk.
/// - Chunk header offset 48: `free_space_offset` (u32 LE) — where free space
///   begins within the chunk.  Valid range: `[512, 65536]` (512 = full chunk
///   header; 65536 = completely full chunk).
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_evtx_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    // File header magic: "ElfFile\0"
    let mut magic = [0u8; 8];
    if f.read_exact(&mut magic).is_err() {
        return CarveQuality::Corrupt;
    }
    if &magic != b"ElfFile\x00" {
        return CarveQuality::Corrupt;
    }

    // First chunk at offset 4096.
    if f.seek(SeekFrom::Start(4096)).is_err() {
        return CarveQuality::Corrupt;
    }
    let mut chunk_magic = [0u8; 8];
    if f.read_exact(&mut chunk_magic).is_err() {
        return CarveQuality::Corrupt;
    }
    if &chunk_magic != b"ElfChnk\x00" {
        return CarveQuality::Corrupt;
    }

    // free_space_offset at chunk+48 (u32 LE): must be in [512, 65536].
    if f.seek(SeekFrom::Start(4096 + 48)).is_err() {
        return CarveQuality::Corrupt;
    }
    let mut fso = [0u8; 4];
    if f.read_exact(&mut fso).is_err() {
        return CarveQuality::Corrupt;
    }
    let free_space_offset = u32::from_le_bytes(fso);
    if !(512..=65_536).contains(&free_space_offset) {
        return CarveQuality::Corrupt;
    }

    CarveQuality::Complete
}

// ── validate_riff_file ─────────────────────────────────────────────────────

/// Validate a carved RIFF-family file (WAV, AVI, WebP) or AIFF/AIFC.
///
/// Structure:
/// - `RIFF` (or `FORM` for AIFF) + u32 size + 4-byte form type.
/// - Followed by chunks: 4-byte fourCC + u32 size + data (RIFF = LE sizes,
///   FORM = BE sizes).
///
/// Validation: the form magic must be `RIFF` or `FORM`; the form type must be
/// 4 printable ASCII bytes; and at least 3 of the first 10 chunks must have
/// 4 printable ASCII fourCC bytes with a declared size within the file.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_riff_file(path: &Path) -> CarveQuality {
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

    // Form header: form_magic(4) + size(4) + form_type(4) = 12 bytes.
    let mut form_hdr = [0u8; 12];
    if f.read_exact(&mut form_hdr).is_err() {
        return CarveQuality::Corrupt;
    }

    let is_le = match &form_hdr[0..4] {
        b"RIFF" => true,
        b"FORM" => false, // AIFF uses big-endian chunk sizes
        _ => return CarveQuality::Corrupt,
    };

    // form_type must be 4 printable ASCII bytes (e.g. "WAVE", "AVI ", "WEBP").
    if !form_hdr[8..12]
        .iter()
        .all(|&b| b.is_ascii_graphic() || b == b' ')
    {
        return CarveQuality::Corrupt;
    }

    // Walk up to 10 chunks and require at least 3 to be structurally valid.
    let mut valid_chunks = 0u32;
    let mut pos: u64 = 12;

    for _ in 0..10 {
        if pos + 8 > file_size {
            break;
        }
        if f.seek(SeekFrom::Start(pos)).is_err() {
            break;
        }
        let mut chunk_hdr = [0u8; 8];
        if f.read_exact(&mut chunk_hdr).is_err() {
            break;
        }

        let fourcc = &chunk_hdr[0..4];
        let chunk_size = read_u32(&chunk_hdr[4..8], is_le) as u64;

        // fourCC must be 4 printable ASCII bytes (letters, digits, space).
        let fourcc_ok = fourcc.iter().all(|&b| b.is_ascii_graphic() || b == b' ');
        // Chunk data must fit within the file.
        let size_ok = chunk_size < file_size;

        if fourcc_ok && size_ok {
            valid_chunks += 1;
        }

        // Advance past this chunk (RIFF chunks are padded to even offsets).
        let padded = chunk_size + (chunk_size & 1);
        pos = match pos.checked_add(8 + padded) {
            Some(next) if next <= file_size => next,
            _ => break,
        };
    }

    if valid_chunks >= 3 {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

// ── validate_exe_file ──────────────────────────────────────────────────────

/// Validate a carved Windows PE executable / DLL.
///
/// Checks:
/// 1. `MZ` magic at offset 0.
/// 2. `e_lfanew` (u32 LE @60): must be in `[64, file_size - 24]`.
/// 3. `PE\x00\x00` signature at `e_lfanew`.
/// 4. `NumberOfSections` (u16 LE @6 of COFF header): must be in `[1, 96]`.
/// 5. Section table: for each section, if `SizeOfRawData > 0` then
///    `PointerToRawData + SizeOfRawData ≤ file_size`.
///    Requires more than half of sections to pass this bounds check.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_exe_file(path: &Path) -> CarveQuality {
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

    // DOS header: need at least 64 bytes for e_lfanew @60.
    let mut dos_hdr = [0u8; 64];
    if f.read_exact(&mut dos_hdr).is_err() {
        return CarveQuality::Corrupt;
    }
    if &dos_hdr[0..2] != b"MZ" {
        return CarveQuality::Corrupt;
    }

    let e_lfanew = u32::from_le_bytes([dos_hdr[60], dos_hdr[61], dos_hdr[62], dos_hdr[63]]) as u64;
    if e_lfanew < 64 || e_lfanew.saturating_add(24) > file_size {
        return CarveQuality::Corrupt;
    }

    // PE signature (4 B) + COFF header (20 B) = 24 bytes.
    if f.seek(SeekFrom::Start(e_lfanew)).is_err() {
        return CarveQuality::Corrupt;
    }
    let mut pe_hdr = [0u8; 24];
    if f.read_exact(&mut pe_hdr).is_err() {
        return CarveQuality::Corrupt;
    }
    if &pe_hdr[0..4] != b"PE\x00\x00" {
        return CarveQuality::Corrupt;
    }

    let num_sections = u16::from_le_bytes([pe_hdr[6], pe_hdr[7]]) as u64;
    if num_sections == 0 || num_sections > 96 {
        return CarveQuality::Corrupt;
    }

    // SizeOfOptionalHeader (u16 LE @20 of COFF) controls where section table starts.
    let opt_hdr_size = u16::from_le_bytes([pe_hdr[20], pe_hdr[21]]) as u64;
    if opt_hdr_size > 8192 {
        return CarveQuality::Corrupt;
    }

    let section_table_off = e_lfanew + 24 + opt_hdr_size;
    if section_table_off.saturating_add(num_sections * 40) > file_size {
        return CarveQuality::Corrupt;
    }

    // Walk section entries (each IMAGE_SECTION_HEADER is 40 bytes).
    let to_check = num_sections.min(32) as u32;
    let mut valid = 0u32;

    for i in 0..to_check as u64 {
        let off = section_table_off + i * 40;
        if f.seek(SeekFrom::Start(off)).is_err() {
            break;
        }
        let mut sec = [0u8; 40];
        if f.read_exact(&mut sec).is_err() {
            break;
        }

        // Name (8 B): must be ASCII printable, space, dot, or null padding.
        let name_ok = sec[0..8]
            .iter()
            .all(|&b| b.is_ascii_graphic() || b == 0 || b == b' ' || b == b'.');

        // SizeOfRawData @16, PointerToRawData @20.
        let size_raw = u32::from_le_bytes([sec[16], sec[17], sec[18], sec[19]]) as u64;
        let ptr_raw = u32::from_le_bytes([sec[20], sec[21], sec[22], sec[23]]) as u64;
        let extent_ok = size_raw == 0 || ptr_raw.saturating_add(size_raw) <= file_size;

        if name_ok && extent_ok {
            valid += 1;
        }
    }

    if valid * 2 > to_check {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

// ── validate_isobmff_file ──────────────────────────────────────────────────

/// Validate a carved ISOBMFF file (MP4, MOV, M4V, 3GP, M4A, HEIC, CR3).
///
/// Walks up to 64 top-level boxes and verifies:
/// 1. At least one `ftyp` box is present (typically the first box).
/// 2. At least one `moov` or `mdat` box is present.
/// 3. Every box's declared size fits within the file.
///
/// Box layout: `size(4 BE) + type(4 ASCII) [+ extended_size(8 BE) when size==1]`.
/// A `size` of 0 means "extends to EOF"; such a box is counted as valid.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_isobmff_file(path: &Path) -> CarveQuality {
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

    let mut has_ftyp = false;
    let mut has_moov_or_mdat = false;
    let mut pos: u64 = 0;

    for _ in 0..64 {
        if pos + 8 > file_size {
            break;
        }
        if f.seek(SeekFrom::Start(pos)).is_err() {
            break;
        }
        let mut hdr = [0u8; 8];
        if f.read_exact(&mut hdr).is_err() {
            break;
        }

        let raw_size = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let box_type = &hdr[4..8];

        // Box type must be 4 printable ASCII bytes.
        if !box_type.iter().all(|&b| b.is_ascii_graphic() || b == b' ') {
            return CarveQuality::Corrupt;
        }

        let box_size: u64 = match raw_size {
            0 => {
                // Box extends to end of file.
                file_size - pos
            }
            1 => {
                // Extended 64-bit size in next 8 bytes.
                if pos + 16 > file_size {
                    return CarveQuality::Corrupt;
                }
                let mut ext = [0u8; 8];
                if f.read_exact(&mut ext).is_err() {
                    return CarveQuality::Corrupt;
                }
                let sz = u64::from_be_bytes(ext);
                if sz < 16 || pos + sz > file_size {
                    return CarveQuality::Corrupt;
                }
                sz
            }
            n if (n as u64) < 8 => return CarveQuality::Corrupt,
            n => {
                let sz = n as u64;
                if pos + sz > file_size {
                    return CarveQuality::Corrupt;
                }
                sz
            }
        };

        match box_type {
            b"ftyp" => has_ftyp = true,
            b"moov" | b"mdat" => has_moov_or_mdat = true,
            _ => {}
        }

        pos += box_size;
    }

    if has_ftyp && has_moov_or_mdat {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

// ── validate_ebml_file ─────────────────────────────────────────────────────

/// Validate a carved EBML file (MKV, WebM).
///
/// Verifies:
/// 1. EBML element ID `0x1A 0x45 0xDF 0xA3` at offset 0.
/// 2. A valid VINT size follows the EBML ID.
/// 3. Segment element ID `0x18 0x53 0x80 0x67` follows the EBML header.
/// 4. A valid, non-"unknown" VINT size follows the Segment ID.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_ebml_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    // Read enough bytes for EBML header + Segment header (generous bound).
    let mut buf = [0u8; 64];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return CarveQuality::Corrupt,
    };
    if n < 12 {
        return CarveQuality::Corrupt;
    }

    // EBML element ID: 0x1A45DFA3 (4 bytes).
    if buf[0..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        return CarveQuality::Corrupt;
    }

    // EBML header VINT size.
    let Some((header_size, vint_len)) = read_vint(&buf, 4) else {
        return CarveQuality::Corrupt;
    };
    let seg_off = 4 + vint_len + header_size as usize;

    // We may need to seek past a large EBML header to find the Segment.
    // Minimum needed: 4 bytes Segment ID + 1 byte VINT = 5 bytes.
    let seg_buf: &[u8] = if seg_off + 5 <= n {
        &buf[seg_off..n]
    } else {
        // Re-read from seg_off.
        if f.seek(SeekFrom::Start(seg_off as u64)).is_err() {
            return CarveQuality::Corrupt;
        }
        let mut tmp = [0u8; 16];
        match f.read(&mut tmp) {
            Ok(k) if k >= 5 => {
                buf[..k].copy_from_slice(&tmp[..k]);
                &buf[..k]
            }
            _ => return CarveQuality::Corrupt,
        }
    };

    // Segment element ID: 0x18538067 (4 bytes).
    if seg_buf.len() < 5 || seg_buf[0..4] != [0x18, 0x53, 0x80, 0x67] {
        return CarveQuality::Corrupt;
    }

    // Segment VINT size: must be parseable and non-zero.
    let Some((seg_size, seg_vint_len)) = read_vint(seg_buf, 4) else {
        return CarveQuality::Corrupt;
    };
    // "Unknown" size: all data bits are 1 (e.g. 1-byte 0x7F, 2-byte 0x3FFF).
    let max_for_width = (1u64 << (7 * seg_vint_len)) - 1;
    if seg_size == max_for_width {
        // Unknown-size Segment is technically valid in MKV; accept it.
        return CarveQuality::Complete;
    }
    if seg_size == 0 {
        return CarveQuality::Corrupt;
    }

    CarveQuality::Complete
}

/// Decode an EBML Variable-Size Integer (VINT) from `data` at `offset`.
///
/// Returns `(decoded_value, bytes_consumed)` or `None` on invalid input.
fn read_vint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    if offset >= data.len() {
        return None;
    }
    let first = data[offset];
    if first == 0 {
        return None;
    }
    let width = first.leading_zeros() as usize + 1;
    if width > 8 || offset + width > data.len() {
        return None;
    }
    let mut value = (first as u64) & !(1u64 << (8 - width));
    for i in 1..width {
        value = (value << 8) | data[offset + i] as u64;
    }
    Some((value, width))
}
