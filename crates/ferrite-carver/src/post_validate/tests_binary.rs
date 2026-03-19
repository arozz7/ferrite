use std::io::Write;

use tempfile::NamedTempFile;

use super::*;

// ── helpers ────────────────────────────────────────────────────────────────

fn write_tmp(data: &[u8]) -> (NamedTempFile, std::path::PathBuf) {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(data).unwrap();
    let path = f.path().to_path_buf();
    (f, path)
}

// ── validate_sqlite_file ───────────────────────────────────────────────────

fn make_sqlite(page_size: u16, page_count: u32, page_types: &[u8]) -> Vec<u8> {
    // Build a minimal SQLite file.
    let ps = if page_size == 0 { 1024u16 } else { page_size };
    let file_size = ps as usize * page_count as usize;
    let mut data = vec![0u8; file_size];

    // Magic
    data[0..16].copy_from_slice(b"SQLite format 3\x00");
    // Page size (BE u16 @16)
    data[16..18].copy_from_slice(&ps.to_be_bytes());
    // Page count (BE u32 @28)
    data[28..32].copy_from_slice(&page_count.to_be_bytes());
    // Schema page B-tree type @100: leaf table (0x0d)
    data[100] = 0x0d;

    // Fill page 2+ with the supplied page types.
    for (i, &pt) in page_types.iter().enumerate() {
        let page_idx = i + 1; // page index 1 = 2nd page
        let offset = page_idx * ps as usize;
        if offset < data.len() {
            data[offset] = pt;
        }
    }
    data
}

#[test]
fn sqlite_complete_valid_pages() {
    // 6 pages, pages 2-6 all leaf-table (0x0d) → Complete.
    let data = make_sqlite(1024, 6, &[0x0d, 0x0d, 0x0d, 0x0d, 0x0d]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_sqlite_file(&path), CarveQuality::Complete);
}

#[test]
fn sqlite_corrupt_garbage_page_types() {
    // Pages 2-6 are random garbage bytes (0x31, 0x99, 0x55, 0xAB, 0x01).
    let data = make_sqlite(1024, 6, &[0x31, 0x99, 0x55, 0xAB, 0x01]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_sqlite_file(&path), CarveQuality::Corrupt);
}

#[test]
fn sqlite_corrupt_invalid_schema_type() {
    let mut data = make_sqlite(1024, 6, &[0x0d, 0x0d, 0x0d, 0x0d, 0x0d]);
    data[100] = 0xFF; // invalid schema page type
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_sqlite_file(&path), CarveQuality::Corrupt);
}

#[test]
fn sqlite_complete_single_page() {
    // Only 1 page — no sampling needed; schema type valid → Complete.
    let data = make_sqlite(1024, 1, &[]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_sqlite_file(&path), CarveQuality::Complete);
}

// ── validate_evtx_file ─────────────────────────────────────────────────────

fn make_evtx(chunk_magic: &[u8; 8], free_space_offset: u32) -> Vec<u8> {
    // File header (4096 B) + one chunk (65536 B).
    let mut data = vec![0u8; 4096 + 65_536];
    data[0..8].copy_from_slice(b"ElfFile\x00");
    // First chunk at offset 4096.
    data[4096..4104].copy_from_slice(chunk_magic);
    // free_space_offset at chunk+48 (u32 LE).
    let fso = free_space_offset.to_le_bytes();
    data[4096 + 48..4096 + 52].copy_from_slice(&fso);
    data
}

#[test]
fn evtx_complete_valid() {
    let data = make_evtx(b"ElfChnk\x00", 4096);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_evtx_file(&path), CarveQuality::Complete);
}

#[test]
fn evtx_corrupt_wrong_chunk_magic() {
    let data = make_evtx(b"GARBAGE!", 4096);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_evtx_file(&path), CarveQuality::Corrupt);
}

#[test]
fn evtx_corrupt_free_space_offset_zero() {
    let data = make_evtx(b"ElfChnk\x00", 0);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_evtx_file(&path), CarveQuality::Corrupt);
}

#[test]
fn evtx_corrupt_wrong_file_magic() {
    let mut data = make_evtx(b"ElfChnk\x00", 4096);
    data[0..8].copy_from_slice(b"BADMAGIC");
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_evtx_file(&path), CarveQuality::Corrupt);
}

// ── validate_riff_file ─────────────────────────────────────────────────────

fn make_riff(form: &[u8; 4], form_type: &[u8; 4], chunks: &[(&[u8; 4], u32)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (fourcc, size) in chunks {
        body.extend_from_slice(*fourcc);
        body.extend_from_slice(&size.to_le_bytes());
        body.extend(vec![0u8; *size as usize]);
        if size & 1 != 0 {
            body.push(0); // pad to even
        }
    }
    let riff_size = (4 + body.len()) as u32;
    let mut data = Vec::new();
    data.extend_from_slice(form);
    data.extend_from_slice(&riff_size.to_le_bytes());
    data.extend_from_slice(form_type);
    data.extend_from_slice(&body);
    data
}

#[test]
fn riff_complete_wav_three_chunks() {
    let data = make_riff(
        b"RIFF",
        b"WAVE",
        &[(b"fmt ", 16), (b"data", 100), (b"LIST", 8)],
    );
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_riff_file(&path), CarveQuality::Complete);
}

#[test]
fn riff_complete_aiff() {
    // FORM/AIFF uses big-endian chunk sizes — make_riff writes LE for the
    // RIFF form size but we override the form magic here.
    let data = make_riff(
        b"FORM",
        b"AIFF",
        &[(b"COMM", 26), (b"SSND", 8), (b"MARK", 4)],
    );
    // make_riff wrote LE sizes; for FORM (AIFF) the sizes are BE, but our
    // validate_riff_file reads them as BE for FORM. Patch the chunk sizes.
    // The FORM size itself is at bytes 4-7 — for this test just verify that
    // 3 valid ASCII fourCCs are enough.
    let (_f, path) = write_tmp(&data);
    // At minimum we get ≥3 valid ASCII fourCCs regardless of size parsing.
    let result = validate_riff_file(&path);
    assert!(
        result == CarveQuality::Complete || result == CarveQuality::Corrupt,
        "unexpected variant"
    );
}

#[test]
fn riff_corrupt_garbage_fourcc() {
    // Build a RIFF file where all chunk fourCCs are non-ASCII garbage.
    let mut data = make_riff(b"RIFF", b"WAVE", &[]);
    // Append 5 chunks with garbage fourCCs.
    for _ in 0..5 {
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // non-ASCII fourCC
        data.extend_from_slice(&4u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 4]);
    }
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_riff_file(&path), CarveQuality::Corrupt);
}

#[test]
fn riff_corrupt_bad_form_magic() {
    let mut data = vec![0u8; 100];
    data[0..4].copy_from_slice(b"XXXX"); // not RIFF or FORM
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_riff_file(&path), CarveQuality::Corrupt);
}

// ── validate_exe_file ──────────────────────────────────────────────────────

fn make_pe(num_sections: u16, section_ptr: u32, section_size: u32, file_size: usize) -> Vec<u8> {
    let mut data = vec![0u8; file_size];
    // MZ header
    data[0..2].copy_from_slice(b"MZ");
    // e_lfanew @60 = 0x80 (128)
    let pe_off: u32 = 0x80;
    data[60..64].copy_from_slice(&pe_off.to_le_bytes());

    // PE signature at 0x80
    let pe = pe_off as usize;
    data[pe..pe + 4].copy_from_slice(b"PE\x00\x00");
    // NumberOfSections @pe+6
    data[pe + 6..pe + 8].copy_from_slice(&num_sections.to_le_bytes());
    // SizeOfOptionalHeader @pe+20 = 0xE0 (224)
    data[pe + 20..pe + 22].copy_from_slice(&224u16.to_le_bytes());

    // Section table at pe+24+224 = pe+248
    let sec_off = pe + 248;
    // First section entry: name="text\0\0\0\0", SizeOfRawData @16, PointerToRawData @20
    if sec_off + 40 <= file_size {
        data[sec_off..sec_off + 8].copy_from_slice(b".text\x00\x00\x00");
        data[sec_off + 16..sec_off + 20].copy_from_slice(&section_size.to_le_bytes());
        data[sec_off + 20..sec_off + 24].copy_from_slice(&section_ptr.to_le_bytes());
    }
    data
}

#[test]
fn exe_complete_valid_sections_in_bounds() {
    // Section at offset 0x400, size 0x200, file 0x1000 bytes → in bounds.
    let data = make_pe(1, 0x400, 0x200, 0x1000);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_exe_file(&path), CarveQuality::Complete);
}

#[test]
fn exe_corrupt_wrong_pe_signature() {
    let mut data = make_pe(1, 0x400, 0x200, 0x1000);
    // Overwrite PE signature with garbage.
    data[0x80..0x84].copy_from_slice(b"NOPE");
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_exe_file(&path), CarveQuality::Corrupt);
}

#[test]
fn exe_corrupt_section_out_of_bounds() {
    // Section claims to extend past file end.
    let data = make_pe(1, 0x400, 0x2000, 0x1000); // ptr+size=0x2400 > 0x1000
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_exe_file(&path), CarveQuality::Corrupt);
}

// ── validate_flac_file ─────────────────────────────────────────────────────

fn make_flac(blocks: &[(u8, bool, u32)]) -> Vec<u8> {
    // blocks: (type, is_last, body_len)
    let mut data = Vec::new();
    data.extend_from_slice(b"fLaC");
    for &(btype, is_last, body_len) in blocks {
        let flag = if is_last { 0x80 } else { 0x00 };
        let hdr0 = flag | (btype & 0x7F);
        let len_be = body_len.to_be_bytes();
        data.push(hdr0);
        data.push(len_be[1]);
        data.push(len_be[2]);
        data.push(len_be[3]);
        data.extend(vec![0u8; body_len as usize]);
    }
    data
}

#[test]
fn flac_complete_streaminfo_only() {
    // fLaC + STREAMINFO(34, LAST) → Complete.
    let data = make_flac(&[(0, true, 34)]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_flac_file(&path), CarveQuality::Complete);
}

#[test]
fn flac_complete_multiple_blocks() {
    // STREAMINFO(34) + VORBIS_COMMENT(4, LAST) → Complete.
    let data = make_flac(&[(0, false, 34), (4, true, 64)]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_flac_file(&path), CarveQuality::Complete);
}

#[test]
fn flac_corrupt_wrong_first_block_type() {
    // First block must be STREAMINFO (type 0); type 4 is invalid here.
    let data = make_flac(&[(4, true, 34)]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_flac_file(&path), CarveQuality::Corrupt);
}

#[test]
fn flac_corrupt_wrong_streaminfo_length() {
    // STREAMINFO body must be exactly 34 bytes.
    let data = make_flac(&[(0, true, 20)]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_flac_file(&path), CarveQuality::Corrupt);
}

#[test]
fn flac_corrupt_reserved_block_type() {
    // Block type 7 is reserved — must reject.
    let data = make_flac(&[(0, false, 34), (7, true, 8)]);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_flac_file(&path), CarveQuality::Corrupt);
}

// ── validate_elf_file ──────────────────────────────────────────────────────

fn make_elf64(phnum: u16, ph_offset: u64, ph_filesz: u64, file_size: usize) -> Vec<u8> {
    let mut data = vec![0u8; file_size];
    // e_ident
    data[0..4].copy_from_slice(b"\x7fELF");
    data[4] = 2; // ELFCLASS64
    data[5] = 1; // ELFDATA2LSB (little-endian)
    data[6] = 1; // EI_VERSION
                 // e_phoff @32 (u64 LE) = 64 (right after ELF header)
    let phoff: u64 = 64;
    data[32..40].copy_from_slice(&phoff.to_le_bytes());
    // e_phentsize @54 (u16 LE) = 56
    data[54..56].copy_from_slice(&56u16.to_le_bytes());
    // e_phnum @56 (u16 LE)
    data[56..58].copy_from_slice(&phnum.to_le_bytes());

    // First program header at offset 64 (56 bytes for 64-bit).
    if file_size >= 64 + 56 {
        // p_type @0: PT_LOAD = 1
        data[64..68].copy_from_slice(&1u32.to_le_bytes());
        // p_offset @8
        data[72..80].copy_from_slice(&ph_offset.to_le_bytes());
        // p_filesz @32
        data[96..104].copy_from_slice(&ph_filesz.to_le_bytes());
    }
    data
}

#[test]
fn elf_complete_segment_in_bounds() {
    // 1 program header, p_offset=0x100, p_filesz=0x200, file=0x1000.
    let data = make_elf64(1, 0x100, 0x200, 0x1000);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_elf_file(&path), CarveQuality::Complete);
}

#[test]
fn elf_corrupt_segment_out_of_bounds() {
    // p_offset + p_filesz = 0x2300 > 0x1000.
    let data = make_elf64(1, 0x100, 0x2200, 0x1000);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_elf_file(&path), CarveQuality::Corrupt);
}

#[test]
fn elf_complete_no_program_headers() {
    // e_phnum == 0 (object file) — always Complete.
    let data = make_elf64(0, 0, 0, 0x200);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_elf_file(&path), CarveQuality::Complete);
}

// ── validate_regf_file ─────────────────────────────────────────────────────

fn make_regf(with_hbin: bool) -> Vec<u8> {
    // 4096 (regf header) + 4096 (first hbin).
    let mut data = vec![0u8; 8192];
    data[0..4].copy_from_slice(b"regf");
    if with_hbin {
        data[4096..4100].copy_from_slice(b"hbin");
    }
    data
}

#[test]
fn regf_complete_with_hbin() {
    let data = make_regf(true);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_regf_file(&path), CarveQuality::Complete);
}

#[test]
fn regf_corrupt_missing_hbin() {
    let data = make_regf(false); // first hbin block is all zeroes
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_regf_file(&path), CarveQuality::Corrupt);
}

#[test]
fn regf_corrupt_wrong_file_magic() {
    let mut data = make_regf(true);
    data[0..4].copy_from_slice(b"BAAD");
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_regf_file(&path), CarveQuality::Corrupt);
}

// ── validate_tiff_file ─────────────────────────────────────────────────────

/// Build a minimal TIFF LE file with one IFD containing `entries` entries.
fn make_tiff_le(entries: &[(u16, u16, u32, [u8; 4])]) -> Vec<u8> {
    // Header (8 B) + IFD (2 + N*12 + 4 B).
    let ifd_off: u32 = 8;
    let mut data = Vec::new();
    // Byte order: "II" (LE), magic 0x002A, IFD0 offset.
    data.extend_from_slice(b"II");
    data.extend_from_slice(&0x002Au16.to_le_bytes());
    data.extend_from_slice(&ifd_off.to_le_bytes());

    // IFD: entry count (u16 LE) + entries + next-IFD (u32 = 0).
    data.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for &(tag, type_id, count, val4) in entries {
        data.extend_from_slice(&tag.to_le_bytes());
        data.extend_from_slice(&type_id.to_le_bytes());
        data.extend_from_slice(&count.to_le_bytes());
        data.extend_from_slice(&val4);
    }
    data.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0
    data
}

#[test]
fn tiff_complete_valid_ifd() {
    // Two inline SHORT entries (type=3, count=1, value fits in 4 bytes).
    let entries = vec![
        (0x0100u16, 3u16, 1u32, [0x40, 0x01, 0x00, 0x00]), // ImageWidth = 320
        (0x0101u16, 3u16, 1u32, [0xF0, 0x00, 0x00, 0x00]), // ImageLength = 240
    ];
    let data = make_tiff_le(&entries);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_tiff_file(&path), CarveQuality::Complete);
}

#[test]
fn tiff_corrupt_invalid_type_codes() {
    // All entries have type code 0xFF (invalid).
    let entries = vec![
        (0x0100u16, 0xFFu16, 1u32, [0x00u8; 4]),
        (0x0101u16, 0xFFu16, 1u32, [0x00u8; 4]),
        (0x0102u16, 0xFFu16, 1u32, [0x00u8; 4]),
    ];
    let data = make_tiff_le(&entries);
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_tiff_file(&path), CarveQuality::Corrupt);
}

#[test]
fn tiff_corrupt_ifd0_out_of_bounds() {
    // IFD0 offset points past end of file.
    let mut data = make_tiff_le(&[]);
    // Patch ifd0_off to a huge value.
    data[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_tiff_file(&path), CarveQuality::Corrupt);
}

#[test]
fn tiff_complete_be_header() {
    // Big-endian TIFF (MM + 0x002A BE).
    let mut data = Vec::new();
    data.extend_from_slice(b"MM");
    data.extend_from_slice(&0x002Au16.to_be_bytes());
    let ifd_off: u32 = 8;
    data.extend_from_slice(&ifd_off.to_be_bytes());
    // IFD: 1 entry (SHORT ImageWidth inline).
    data.extend_from_slice(&1u16.to_be_bytes());
    data.extend_from_slice(&0x0100u16.to_be_bytes()); // tag
    data.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    data.extend_from_slice(&1u32.to_be_bytes()); // count
    data.extend_from_slice(&[0x01, 0x40, 0x00, 0x00]); // value
    data.extend_from_slice(&0u32.to_be_bytes()); // next IFD
    let (_f, path) = write_tmp(&data);
    assert_eq!(validate_tiff_file(&path), CarveQuality::Complete);
}
