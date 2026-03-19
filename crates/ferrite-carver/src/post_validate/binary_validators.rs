//! File-based structural validators for binary container formats.
//!
//! These validators open the extracted file and seek through its internal
//! structure to verify structural integrity beyond what fixed head+tail
//! buffers can detect.
//!
//! Validators in this file:
//! - [`validate_evtx_file`]  — Windows Event Log chunk magic check
//! - [`validate_riff_file`]  — RIFF/FORM chunk fourCC walk (WAV/AVI/WebP/AIFF)
//! - [`validate_exe_file`]   — PE header + section table bounds check
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
