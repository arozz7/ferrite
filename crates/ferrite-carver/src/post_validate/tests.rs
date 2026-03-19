use super::file_validators::{looks_like_xref, parse_last_startxref};
use super::*;

// ── is_truncated short-circuit ────────────────────────────────────────────

#[test]
fn truncated_flag_returns_truncated_regardless_of_ext() {
    assert_eq!(
        validate_extracted("jpg", &[], &[0xFF, 0xD9], true, 2),
        CarveQuality::Truncated
    );
    assert_eq!(
        validate_extracted("unknown", &[], &[], true, 0),
        CarveQuality::Truncated
    );
}

// ── JPEG ─────────────────────────────────────────────────────────────────

#[test]
fn jpeg_complete_with_eoi_marker() {
    let tail = &[0x00u8, 0x01, 0xFF, 0xD9];
    assert_eq!(
        validate_extracted("jpg", &[], tail, false, 4),
        CarveQuality::Complete
    );
}

#[test]
fn jpeg_corrupt_without_eoi() {
    let tail = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00];
    assert_eq!(
        validate_extracted("jpg", &[], tail, false, 5),
        CarveQuality::Corrupt
    );
}

#[test]
fn jpeg_corrupt_on_empty_data() {
    assert_eq!(
        validate_extracted("jpg", &[], &[], false, 0),
        CarveQuality::Corrupt
    );
}

#[test]
fn jpeg_corrupt_when_only_one_byte() {
    assert_eq!(
        validate_extracted("jpg", &[], &[0xD9], false, 1),
        CarveQuality::Corrupt
    );
}

#[test]
fn jpeg_complete_with_valid_entropy_data() {
    // Valid scan data: byte-stuffed 0xFF (FF 00), normal bytes, then EOI.
    let mut tail = vec![0x12, 0x34, 0xFF, 0x00, 0x56, 0x78, 0xAB];
    tail.extend_from_slice(&[0xFF, 0xD9]); // EOI
    assert_eq!(
        validate_extracted("jpg", &[], &tail, false, tail.len() as u64),
        CarveQuality::Complete
    );
}

#[test]
fn jpeg_complete_with_rst_markers() {
    // Valid scan data containing RST markers (FF D0 – FF D7).
    let mut tail = vec![0x12, 0xFF, 0xD0, 0x34, 0xFF, 0xD7, 0x56];
    tail.extend_from_slice(&[0xFF, 0xD9]);
    assert_eq!(
        validate_extracted("jpg", &[], &tail, false, tail.len() as u64),
        CarveQuality::Complete
    );
}

#[test]
fn jpeg_corrupt_with_invalid_marker_in_scan_data() {
    // Invalid: FF E0 in scan data (APP0 marker — should not appear in
    // entropy data; indicates overwritten sectors from another file).
    let mut tail = vec![0x12, 0x34, 0xFF, 0xE0, 0x56, 0x78];
    tail.extend_from_slice(&[0xFF, 0xD9]);
    assert_eq!(
        validate_extracted("jpg", &[], &tail, false, tail.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn jpeg_corrupt_with_ff_c0_in_scan_data() {
    // Invalid: FF C0 (SOF0 marker) in scan data.
    let mut tail = vec![0x00; 100];
    tail[50] = 0xFF;
    tail[51] = 0xC0;
    tail.extend_from_slice(&[0xFF, 0xD9]);
    assert_eq!(
        validate_extracted("jpg", &[], &tail, false, tail.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn jpeg_complete_with_ff_fill_bytes() {
    // Valid: consecutive FF bytes (fill padding) followed by a valid marker.
    let mut tail = vec![0x12, 0xFF, 0xFF, 0x00, 0x34];
    tail.extend_from_slice(&[0xFF, 0xD9]);
    assert_eq!(
        validate_extracted("jpg", &[], &tail, false, tail.len() as u64),
        CarveQuality::Complete
    );
}

// ── PNG ──────────────────────────────────────────────────────────────────

#[test]
fn png_complete_with_iend() {
    let mut tail = vec![0u8; 4];
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);
    assert_eq!(
        validate_extracted("png", &[], &tail, false, 16),
        CarveQuality::Complete
    );
}

#[test]
fn png_corrupt_missing_iend() {
    let tail = &[
        0x89u8, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        validate_extracted("png", &[], tail, false, 12),
        CarveQuality::Corrupt
    );
}

#[test]
fn png_corrupt_on_empty() {
    assert_eq!(
        validate_extracted("png", &[], &[], false, 0),
        CarveQuality::Corrupt
    );
}

/// Build a valid PNG head with correct CRC-32 values.
fn make_png_head() -> Vec<u8> {
    let mut buf = Vec::new();
    // PNG signature
    buf.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    // IHDR chunk: length=13
    let ihdr_data: [u8; 13] = [
        0x00, 0x00, 0x00, 0x01, // width = 1
        0x00, 0x00, 0x00, 0x01, // height = 1
        0x08, 0x02, 0x00, 0x00, 0x00,
    ];
    buf.extend_from_slice(&13u32.to_be_bytes());
    buf.extend_from_slice(b"IHDR");
    buf.extend_from_slice(&ihdr_data);
    let crc = crc32fast::hash(&buf[12..]); // CRC over "IHDR" + data
    buf.extend_from_slice(&crc.to_be_bytes());
    buf
}

#[test]
fn png_corrupt_idat_declares_size_beyond_file() {
    // Real-world case: a single large IDAT chunk whose declared length
    // would place its end past where IEND sits.  This happens when the
    // carver stops at an IEND that is embedded inside IDAT data from an
    // overlapping sector belonging to a different file.
    //
    // IDAT at offset 114 claims 192_786 bytes of data →
    //   chunk_body_end = 114 + 12 + 192_786 = 192_912
    // but file_size = 191_831, so file_size - 12 = 191_819
    // 192_912 > 191_819 → Corrupt.
    let mut head = make_png_head(); // PNG sig + IHDR (33 bytes)
                                    // Append a fake IDAT header with length 192_786 (0x0002F112).
                                    // The data extends well beyond the head buffer, so the head walk
                                    // will hit the "chunk extends beyond buffer" branch and check sizes.
    head.extend_from_slice(&[0x00, 0x02, 0xF1, 0x12]); // length = 192_786
    head.extend_from_slice(b"IDAT");
    // 4 more bytes so the chunk header (12 bytes) fully fits in the buffer,
    // allowing the walk to read length+type and then hit the overflow check.
    head.extend_from_slice(&[0x78, 0xDA, 0x00, 0x00]); // start of zlib stream

    let mut tail = vec![0u8; 4];
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);

    // file_size = 191_831; IDAT body end = 114+12+192_786 = 192_912 > 191_819
    assert_eq!(
        validate_extracted("png", &head, &tail, false, 191_831),
        CarveQuality::Corrupt
    );
}

#[test]
fn png_complete_idat_fits_within_file() {
    // Positive case: IDAT declared size is consistent with file size.
    // IDAT at the same offset (33+8 = offset depends on head size),
    // but with a length that fits: body_end ≤ file_size - 12.
    let mut head = make_png_head(); // 33 bytes (sig + IHDR)
    let idat_offset = head.len() as u64; // 33
    let idat_data_len: u32 = 1000;
    head.extend_from_slice(&idat_data_len.to_be_bytes());
    head.extend_from_slice(b"IDAT");
    // 4 more bytes so chunk header (12 bytes) fits → overflow check can run.
    head.extend_from_slice(&[0x78, 0xDA, 0x00, 0x00]);
    // chunk_body_end = idat_offset + 12 + 1000 = 33 + 12 + 1000 = 1045
    // file_size = 1057 (= 1045 + 12 for IEND)
    let file_size: u64 = idat_offset + 12 + idat_data_len as u64 + 12;

    let mut tail = vec![0u8; 4];
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);
    assert_eq!(
        validate_extracted("png", &head, &tail, false, file_size),
        CarveQuality::Complete
    );
}

#[test]
fn png_complete_with_valid_ihdr_crc() {
    let head = make_png_head();
    let mut tail = vec![0u8; 4];
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);
    assert_eq!(
        validate_extracted("png", &head, &tail, false, 100),
        CarveQuality::Complete
    );
}

#[test]
fn png_corrupt_with_bad_ihdr_crc() {
    let mut head = make_png_head();
    // Corrupt the IHDR data (flip a byte in the image width).
    head[16] = 0xFF;
    let mut tail = vec![0u8; 4];
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);
    assert_eq!(
        validate_extracted("png", &head, &tail, false, 100),
        CarveQuality::Corrupt
    );
}

#[test]
fn png_corrupt_with_non_alpha_chunk_type() {
    let mut head = make_png_head();
    // Corrupt the IHDR chunk type to contain a non-alpha byte.
    head[12] = 0x00;
    let mut tail = vec![0u8; 4];
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);
    assert_eq!(
        validate_extracted("png", &head, &tail, false, 100),
        CarveQuality::Corrupt
    );
}

/// Build a tail buffer with a valid chunk immediately before IEND.
fn make_png_tail_with_chunk(chunk_type: &[u8; 4], chunk_data: &[u8]) -> Vec<u8> {
    let mut tail = Vec::new();
    // chunk: [length(4)][type(4)][data(N)][CRC(4)]
    tail.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
    tail.extend_from_slice(chunk_type);
    tail.extend_from_slice(chunk_data);
    // CRC covers type + data
    let mut crc_input = Vec::new();
    crc_input.extend_from_slice(chunk_type);
    crc_input.extend_from_slice(chunk_data);
    let crc = crc32fast::hash(&crc_input);
    tail.extend_from_slice(&crc.to_be_bytes());
    // IEND chunk
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);
    tail
}

#[test]
fn png_complete_with_valid_tail_chunk() {
    // A valid tEXt chunk before IEND in the tail buffer.
    let tail = make_png_tail_with_chunk(b"tEXt", b"Comment\x00hello world!!");
    assert_eq!(
        validate_extracted("png", &[], &tail, false, tail.len() as u64),
        CarveQuality::Complete
    );
}

#[test]
fn png_corrupt_with_bad_tail_chunk_crc() {
    // Build a valid tail, then corrupt the chunk data so CRC mismatches.
    let mut tail = make_png_tail_with_chunk(b"tEXt", b"Comment\x00hello world!!");
    // Corrupt a data byte (offset 8 = first byte of data, after length+type).
    tail[8] = 0xFF;
    assert_eq!(
        validate_extracted("png", &[], &tail, false, tail.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn png_complete_when_predecessor_exceeds_tail() {
    // When the preceding chunk is larger than the tail buffer, the reverse
    // walk cannot find it — this is NOT corruption, just a large IDAT.
    // Use a small tail that only contains IEND (no room for a predecessor).
    let tail: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    assert_eq!(
        validate_extracted("png", &[], tail, false, 500_000),
        CarveQuality::Complete
    );
}

#[test]
fn png_corrupt_garbage_before_iend_in_tail() {
    // Simulate sector-level corruption: garbage bytes before IEND.
    // The reverse walk finds no valid chunk boundary → but since the
    // garbage bytes won't accidentally form a valid length+type+CRC
    // chain, no predecessor is found.  This is caught because within
    // the tail buffer we DO find a candidate position where the length
    // matches but the CRC fails.
    let mut tail = vec![0u8; 64];
    // Put some garbage that happens to have a matching length field.
    // data_len for the chunk before IEND: tail is 64+12=76 bytes total.
    // IEND starts at byte 64.  If we place a chunk with data_len=40
    // starting at offset 64-12-40=12, then tail[12..16] = 40 as BE u32.
    let data_len: u32 = 40;
    let chunk_start = 64 - 12 - data_len as usize; // = 12
    tail[chunk_start..chunk_start + 4].copy_from_slice(&data_len.to_be_bytes());
    // Put a valid-looking chunk type.
    tail[chunk_start + 4..chunk_start + 8].copy_from_slice(b"IDAT");
    // But leave the data and CRC as zeros — CRC will NOT match.

    // Append IEND.
    tail.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);

    assert_eq!(
        validate_extracted("png", &[], &tail, false, tail.len() as u64),
        CarveQuality::Corrupt
    );
}

// Post-IDAT garbage chunk type detected via tail buffer (Phase 85).
// Real-world pattern: fragmented drive sector overwrites IDAT pixel data,
// producing a non-ASCII chunk type immediately after the IDAT body.
// Detectable when the IDAT end falls within the tail buffer.

#[test]
fn png_corrupt_garbage_type_after_idat_in_tail() {
    // File layout (file_size = 100_000):
    //   [PNG sig][IDAT hdr(len=60_000)][... body beyond head ...][CRC][garbage][...][IEND]
    //   Head covers bytes 0..8192; tail covers bytes 34_464..100_000.
    //   IDAT chunk_body_end = 8 + 12 + 60_000 = 60_020 → in tail buffer.
    const FILE_SIZE: u64 = 100_000;
    const IDAT_LEN: u32 = 60_000;
    let idat_body_end: u64 = 8 + 12 + IDAT_LEN as u64; // 60_020
    let tail_start: u64 = FILE_SIZE - 65_536; // 34_464
    let ti = (idat_body_end - tail_start) as usize; // 25_556

    let mut head = vec![0u8; 8192];
    head[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    head[8..12].copy_from_slice(&IDAT_LEN.to_be_bytes());
    head[12..16].copy_from_slice(b"IDAT");

    let mut tail = vec![0u8; 65_536];
    // Garbage chunk length (any value)
    tail[ti..ti + 4].copy_from_slice(&132u32.to_be_bytes());
    // Garbage chunk type: non-ASCII bytes matching the real-world case
    tail[ti + 4..ti + 8].copy_from_slice(&[0x21, 0x60, 0x1E, 0xEE]);
    // IEND at the tail end (required by the initial IEND check)
    tail[65_536 - 12..].copy_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);

    assert_eq!(
        validate_extracted("png", &head, &tail, false, FILE_SIZE),
        CarveQuality::Corrupt
    );
}

#[test]
fn png_complete_valid_ascii_chunk_after_idat_in_tail() {
    // Positive case: the chunk immediately following the large IDAT has an
    // ASCII-alpha type (IEND), so the post-IDAT check passes.
    const FILE_SIZE: u64 = 100_000;
    const IDAT_LEN: u32 = 60_000;
    let idat_body_end: u64 = 8 + 12 + IDAT_LEN as u64; // 60_020
    let tail_start: u64 = FILE_SIZE - 65_536; // 34_464
    let ti = (idat_body_end - tail_start) as usize; // 25_556

    let mut head = vec![0u8; 8192];
    head[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    head[8..12].copy_from_slice(&IDAT_LEN.to_be_bytes());
    head[12..16].copy_from_slice(b"IDAT");

    let mut tail = vec![0u8; 65_536];
    // Valid ASCII chunk type after IDAT (e.g., IEND or IDAT continuation)
    tail[ti..ti + 4].copy_from_slice(&0u32.to_be_bytes()); // length = 0
    tail[ti + 4..ti + 8].copy_from_slice(b"IEND"); // valid ASCII type
                                                   // IEND at the tail end
    tail[65_536 - 12..].copy_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]);

    assert_eq!(
        validate_extracted("png", &head, &tail, false, FILE_SIZE),
        CarveQuality::Complete
    );
}

// ── validate_png_file ─────────────────────────────────────────────────────
//
// These tests write real files to a temp directory so the seek-based chunk
// walker can open and read them.

fn write_png_chunks(chunks: &[(&[u8], &[u8])]) -> (tempfile::TempDir, std::path::PathBuf) {
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.png");
    let mut f = std::fs::File::create(&path).expect("create");
    // PNG signature
    f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        .unwrap();
    for (chunk_type, data) in chunks {
        let len = data.len() as u32;
        f.write_all(&len.to_be_bytes()).unwrap();
        f.write_all(chunk_type).unwrap();
        f.write_all(data).unwrap();
        // CRC covers type + data.
        let mut crc_input = Vec::with_capacity(4 + data.len());
        crc_input.extend_from_slice(chunk_type);
        crc_input.extend_from_slice(data);
        let crc = crc32fast::hash(&crc_input);
        f.write_all(&crc.to_be_bytes()).unwrap();
    }
    (dir, path)
}

#[test]
fn validate_png_file_complete_minimal() {
    // Minimal valid PNG: IHDR + IDAT (1 byte, 0 data) + IEND.
    // Use a tiny 1×1 raw IDAT body (deflate-compressed zeros, simplified).
    let ihdr_data = [
        0, 0, 0, 1, // width = 1
        0, 0, 0, 1, // height = 1
        8, // bit depth
        0, // color type = grayscale
        0, 0, 0, // compression, filter, interlace
    ];
    // Minimal valid zlib-compressed single-row pixel (filter byte 0x00 + pixel 0x00).
    let idat_data = [0x08, 0xD7, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01];
    let (_dir, path) =
        write_png_chunks(&[(b"IHDR", &ihdr_data), (b"IDAT", &idat_data), (b"IEND", &[])]);
    assert_eq!(validate_png_file(&path), CarveQuality::Complete);
}

#[test]
fn validate_png_file_corrupt_garbage_type_after_large_idat() {
    // Simulates file 1 from the real-world test run: a large IDAT (body in
    // the dead zone between head and tail buffers) followed by a garbage chunk
    // with non-ASCII type bytes.  The file-walk approach catches it because it
    // seeks past the IDAT body and reads the corrupt chunk type directly.
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("corrupt.png");
    let mut f = std::fs::File::create(&path).expect("create");
    // PNG signature
    f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        .unwrap();
    // IHDR chunk (valid, small — will be CRC-verified by the walker)
    let ihdr_data = [0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0];
    f.write_all(&(13u32).to_be_bytes()).unwrap();
    f.write_all(b"IHDR").unwrap();
    f.write_all(&ihdr_data).unwrap();
    let ihdr_crc = crc32fast::hash(&[b"IHDR".as_ref(), &ihdr_data].concat());
    f.write_all(&ihdr_crc.to_be_bytes()).unwrap();
    // Large IDAT chunk (body = 100_000 zero bytes — bigger than MAX_CRC_BODY).
    // The body is valid (all zeros) but the chunk after it is garbage.
    let idat_len: u32 = 100_000;
    f.write_all(&idat_len.to_be_bytes()).unwrap();
    f.write_all(b"IDAT").unwrap();
    let idat_body = vec![0u8; 100_000];
    f.write_all(&idat_body).unwrap();
    // Deliberately wrong CRC (all zeros) — the walker skips CRC for large chunks.
    f.write_all(&[0u8; 4]).unwrap();
    // Garbage chunk: non-ASCII type bytes + some body + fake IEND at the end.
    f.write_all(&(132u32).to_be_bytes()).unwrap(); // garbage length
    f.write_all(&[0x21, 0x60, 0x1E, 0xEE]).unwrap(); // non-ASCII type
    f.write_all(&vec![0xAB; 132]).unwrap(); // garbage body
    f.write_all(&[0u8; 4]).unwrap(); // garbage CRC
                                     // Valid IEND (unreachable — the walk aborts at the garbage type above)
    f.write_all(&[0u8; 4]).unwrap();
    f.write_all(b"IEND").unwrap();
    f.write_all(&[0u8; 4]).unwrap();
    drop(f);
    assert_eq!(validate_png_file(&path), CarveQuality::Corrupt);
}

#[test]
fn validate_png_file_corrupt_bad_crc_on_small_chunk() {
    // IHDR with a deliberately wrong CRC — caught because IHDR is small.
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bad_crc.png");
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        .unwrap();
    f.write_all(&(13u32).to_be_bytes()).unwrap();
    f.write_all(b"IHDR").unwrap();
    f.write_all(&[0u8; 13]).unwrap();
    f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF]).unwrap(); // wrong CRC
    f.write_all(&[0u8; 4]).unwrap();
    f.write_all(b"IEND").unwrap();
    let iend_crc = crc32fast::hash(b"IEND");
    f.write_all(&iend_crc.to_be_bytes()).unwrap();
    drop(f);
    assert_eq!(validate_png_file(&path), CarveQuality::Corrupt);
}

#[test]
fn validate_png_file_corrupt_missing_iend() {
    // File ends abruptly inside an IDAT body — read_exact returns Err.
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("no_iend.png");
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        .unwrap();
    // IDAT header claiming 10_000 bytes but file ends immediately after the header.
    f.write_all(&(10_000u32).to_be_bytes()).unwrap();
    f.write_all(b"IDAT").unwrap();
    // No body, no IEND.
    drop(f);
    assert_eq!(validate_png_file(&path), CarveQuality::Corrupt);
}

// ── HTML ─────────────────────────────────────────────────────────────────

#[test]
fn html_complete_with_body_content() {
    let html = b"<!DOCTYPE html><html><head><title>Test</title></head>\
        <body><p>This is a paragraph with enough visible text content.</p></body></html>";
    assert_eq!(
        validate_extracted("html", html, html, false, html.len() as u64),
        CarveQuality::Complete
    );
}

#[test]
fn html_corrupt_empty_body() {
    // Kindle-style fragment: valid HTML structure but no visible text.
    let html = br#"<!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Red Rabbit</title></head>
<body class="calibre" aid="2D0">
<div id="filepos468852" class="calibre1" aid="2D1"></div>
</body>
</html>"#;
    assert_eq!(
        validate_extracted("html", html, html, false, html.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn html_corrupt_missing_closing_tag() {
    let html = b"<!DOCTYPE html><html><body><p>Hello world</p></body>";
    assert_eq!(
        validate_extracted("html", html, html, false, html.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn html_complete_uppercase_tags() {
    let html = b"<!DOCTYPE HTML><HTML><HEAD></HEAD>\
        <BODY><P>This paragraph has more than thirty two characters of text.</P></BODY></HTML>";
    assert_eq!(
        validate_extracted("html", html, html, false, html.len() as u64),
        CarveQuality::Complete
    );
}

#[test]
fn html_corrupt_only_whitespace_in_body() {
    let html = b"<!DOCTYPE html><html><body>   \n\t  \n   </body></html>";
    assert_eq!(
        validate_extracted("html", html, html, false, html.len() as u64),
        CarveQuality::Corrupt
    );
}

// ── GIF ──────────────────────────────────────────────────────────────────

#[test]
fn gif_complete_with_trailer() {
    let tail = b"GIF89a\x3B";
    assert_eq!(
        validate_extracted("gif", &[], tail, false, 7),
        CarveQuality::Complete
    );
}

#[test]
fn gif_corrupt_missing_trailer() {
    let tail = b"GIF89a";
    assert_eq!(
        validate_extracted("gif", &[], tail, false, 6),
        CarveQuality::Corrupt
    );
}

#[test]
fn gif_corrupt_on_empty() {
    assert_eq!(
        validate_extracted("gif", &[], &[], false, 0),
        CarveQuality::Corrupt
    );
}

// ── PDF ──────────────────────────────────────────────────────────────────

#[test]
fn pdf_complete_with_eof_marker() {
    let tail = b"%PDF-1.4\n...content...\n%%EOF\n";
    assert_eq!(
        validate_extracted("pdf", &[], tail, false, tail.len() as u64),
        CarveQuality::Complete
    );
}

#[test]
fn pdf_corrupt_without_eof() {
    let tail = b"%PDF-1.4\n...content...";
    assert_eq!(
        validate_extracted("pdf", &[], tail, false, tail.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn pdf_complete_eof_within_last_1kb() {
    let mut tail = vec![0u8; 2000];
    // Put %%EOF at byte 1800 (within last 1 KiB of the 2000-byte tail).
    tail[1800..1805].copy_from_slice(b"%%EOF");
    assert_eq!(
        validate_extracted("pdf", &[], &tail, false, 2000),
        CarveQuality::Complete
    );
}

#[test]
fn pdf_corrupt_eof_outside_last_1kb() {
    let mut tail = vec![0u8; 2000];
    // Put %%EOF at byte 100 (more than 1 KiB from the end — not searched).
    tail[100..105].copy_from_slice(b"%%EOF");
    assert_eq!(
        validate_extracted("pdf", &[], &tail, false, 2000),
        CarveQuality::Corrupt
    );
}

// ── validate_pdf_file ─────────────────────────────────────────────────────
//
// Tests use tempfile to write real files so validate_pdf_file can open them.

fn write_pdf(content: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.pdf");
    std::fs::File::create(&path)
        .expect("create")
        .write_all(content)
        .expect("write");
    (dir, path)
}

#[test]
fn validate_pdf_file_complete_traditional_xref() {
    // Minimal PDF with traditional xref table at a valid offset.
    // xref table starts at byte 9 (right after the header).
    let xref_offset: usize = 9;
    let mut content = b"%PDF-1.4\n".to_vec();
    assert_eq!(content.len(), xref_offset);
    content.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n");
    content.extend_from_slice(b"trailer\n<</Size 1>>\n");
    let sxref_pos = content.len();
    content.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    let _ = sxref_pos; // used for readability
    let (_dir, path) = write_pdf(&content);
    assert_eq!(validate_pdf_file(&path), CarveQuality::Complete);
}

#[test]
fn validate_pdf_file_complete_xref_stream() {
    // PDF whose startxref points to a cross-reference stream object.
    // The object header starts with an integer, e.g. "1 0 obj".
    let xref_offset: usize = 9;
    let mut content = b"%PDF-1.5\n".to_vec();
    assert_eq!(content.len(), xref_offset);
    content.extend_from_slice(b"1 0 obj\n<</Type/XRef/Size 1>>\nstream\nendstream\nendobj\n");
    content.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    let (_dir, path) = write_pdf(&content);
    assert_eq!(validate_pdf_file(&path), CarveQuality::Complete);
}

#[test]
fn validate_pdf_file_corrupt_startxref_zero() {
    // Real-world case 1: startxref = 0 always points to the %PDF header,
    // never to an xref table.
    let content = b"%PDF-1.6\n%binary\ncontent here\nstartxref\n0\n%%EOF\n";
    let (_dir, path) = write_pdf(content);
    assert_eq!(validate_pdf_file(&path), CarveQuality::Corrupt);
}

#[test]
fn validate_pdf_file_corrupt_startxref_points_to_garbage() {
    // Real-world case 2: startxref N is within the file but the bytes there
    // are random binary data, not an xref table or object header.
    let mut content = b"%PDF-1.5\n".to_vec();
    let garbage_offset = content.len();
    // Place random-looking binary data at the xref offset.
    content.extend_from_slice(&[0x47, 0x87, 0x2E, 0x3E, 0x55, 0x6F, 0xB0, 0xBD]);
    content.extend_from_slice(format!("startxref\n{garbage_offset}\n%%EOF\n").as_bytes());
    let (_dir, path) = write_pdf(&content);
    assert_eq!(validate_pdf_file(&path), CarveQuality::Corrupt);
}

#[test]
fn validate_pdf_file_corrupt_startxref_beyond_file() {
    // startxref value larger than the file — clearly invalid.
    let content = b"%PDF-1.4\nstartxref\n999999999\n%%EOF\n";
    let (_dir, path) = write_pdf(content);
    assert_eq!(validate_pdf_file(&path), CarveQuality::Corrupt);
}

#[test]
fn validate_pdf_file_corrupt_no_eof() {
    // File ends without %%EOF.
    let content = b"%PDF-1.4\nxref\n0 0\ntrailer\n<</Size 0>>\nstartxref\n9\n";
    let (_dir, path) = write_pdf(content);
    assert_eq!(validate_pdf_file(&path), CarveQuality::Corrupt);
}

#[test]
fn parse_last_startxref_basic() {
    assert_eq!(parse_last_startxref(b"startxref\n116\n%%EOF"), Some(116));
}

#[test]
fn parse_last_startxref_crlf() {
    assert_eq!(
        parse_last_startxref(b"startxref\r\n1005634\r\n%%EOF"),
        Some(1005634)
    );
}

#[test]
fn parse_last_startxref_zero() {
    assert_eq!(parse_last_startxref(b"startxref\r\n0\r\n%%EOF"), Some(0));
}

#[test]
fn parse_last_startxref_takes_last() {
    // Multiple startxref entries — must return the last value.
    let data = b"startxref\n100\n%%EOF\nstartxref\n200\n%%EOF";
    assert_eq!(parse_last_startxref(data), Some(200));
}

#[test]
fn looks_like_xref_traditional() {
    assert!(looks_like_xref(b"xref\n0 5\n"));
}

#[test]
fn looks_like_xref_stream_object() {
    assert!(looks_like_xref(b"616 0 obj\n<<"));
}

#[test]
fn looks_like_xref_rejects_pdf_header() {
    assert!(!looks_like_xref(b"%PDF-1.6\n"));
}

#[test]
fn looks_like_xref_rejects_binary_garbage() {
    assert!(!looks_like_xref(&[0x47, 0x87, 0x2E, 0x3E, 0x55, 0x6F]));
}

// ── ZIP / EOCD ────────────────────────────────────────────────────────────

#[test]
fn zip_complete_with_eocd() {
    // EOCD with cd_offset = 0 (fits within any file)
    let mut tail = vec![0u8; 32];
    tail[10..14].copy_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
    // cd_offset at EOCD+16 = tail[26..30] = 0 (already zeroed)
    assert_eq!(
        validate_extracted("zip", &[], &tail, false, 32),
        CarveQuality::Complete
    );
}

#[test]
fn zip_corrupt_missing_eocd() {
    let tail = b"PK\x03\x04some zip content without central dir";
    assert_eq!(
        validate_extracted("zip", &[], tail, false, tail.len() as u64),
        CarveQuality::Corrupt
    );
}

#[test]
fn zip_corrupt_cd_offset_beyond_file() {
    // EOCD present but cd_offset points past the extracted file.
    let mut tail = vec![0u8; 32];
    tail[0..4].copy_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
    // cd_offset at EOCD+16 = tail[16..20] = 1_000_000 (way beyond 32-byte file)
    tail[16..20].copy_from_slice(&1_000_000u32.to_le_bytes());
    assert_eq!(
        validate_extracted("zip", &[], &tail, false, 32),
        CarveQuality::Corrupt
    );
}

#[test]
fn zip_complete_cd_offset_within_file() {
    // EOCD present with cd_offset that fits within a 50 000-byte file.
    let mut tail = vec![0u8; 32];
    tail[0..4].copy_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
    tail[16..20].copy_from_slice(&10_000u32.to_le_bytes());
    assert_eq!(
        validate_extracted("zip", &[], &tail, false, 50_000),
        CarveQuality::Complete
    );
}

#[test]
fn zip_complete_on_ole_extension() {
    let mut tail = vec![0u8; 32];
    tail[0..4].copy_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
    assert_eq!(
        validate_extracted("ole", &[], &tail, false, 32),
        CarveQuality::Complete
    );
}

// ── Unknown formats ──────────────────────────────────────────────────────

#[test]
fn unknown_format_returns_unknown() {
    assert_eq!(
        validate_extracted("mp4", &[], &[0u8; 32], false, 32),
        CarveQuality::Unknown
    );
    assert_eq!(
        validate_extracted("mkv", &[], &[0u8; 32], false, 32),
        CarveQuality::Unknown
    );
    assert_eq!(
        validate_extracted("avi", &[], &[0u8; 32], false, 32),
        CarveQuality::Unknown
    );
}
