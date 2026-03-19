//! File-based structural validators for image and document formats.
//!
//! These validators open the extracted file and seek through its internal
//! structure, bypassing the "dead zone" blind spot of fixed head+tail buffers.
//! Each validator reads only the bytes it needs (chunk headers, page type
//! bytes, cross-reference offsets) using `Seek`, keeping I/O minimal.
//!
//! Validators in this file:
//! - [`validate_png_file`]  — PNG chunk CRC walk
//! - [`validate_pdf_file`]  — PDF `startxref` seek + xref check
//! - [`validate_sqlite_file`] — SQLite page type majority check

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::CarveQuality;

// ── validate_png_file ──────────────────────────────────────────────────────

/// Validate a carved PNG by walking its chunk structure directly on the file.
///
/// Unlike [`super::validate_extracted`] with `"png"`, this opens the file and
/// seeks through it chunk-by-chunk, reading only the 12-byte chunk envelope
/// (length + type + CRC) for large chunks and the full body for small ones
/// (≤ 64 KiB).  This eliminates the "dead zone" limitation of the fixed head
/// + tail buffers: a corrupt chunk type anywhere in the file is caught in
///   O(N chunks) seeks, where N is typically 5–8 for a real-world PNG.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened (e.g. it
/// was just deleted by skip-corrupt mode in a race).
pub fn validate_png_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    // Verify the 8-byte PNG signature.
    let mut sig = [0u8; 8];
    if f.read_exact(&mut sig).is_err() || sig != [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return CarveQuality::Corrupt;
    }

    // Body size threshold: read and CRC-verify chunks whose data fits here;
    // seek over larger chunks (e.g. IDAT pixel data).
    const MAX_CRC_BODY: usize = 65_536;

    loop {
        // Read chunk length (4 B) + type (4 B).
        let mut hdr = [0u8; 8];
        if f.read_exact(&mut hdr).is_err() {
            return CarveQuality::Corrupt; // unexpected EOF before IEND
        }

        let data_len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let chunk_type = &hdr[4..8];

        // Every PNG chunk type must be 4 ASCII alphabetic bytes.
        if !chunk_type.iter().all(|&b| b.is_ascii_alphabetic()) {
            return CarveQuality::Corrupt;
        }

        if chunk_type == b"IEND" {
            // IEND must have a zero-length body.
            return if data_len == 0 {
                CarveQuality::Complete
            } else {
                CarveQuality::Corrupt
            };
        }

        if data_len <= MAX_CRC_BODY {
            // Small chunk: read the body and verify CRC-32.
            let mut body = vec![0u8; data_len];
            if f.read_exact(&mut body).is_err() {
                return CarveQuality::Corrupt;
            }
            let mut crc_bytes = [0u8; 4];
            if f.read_exact(&mut crc_bytes).is_err() {
                return CarveQuality::Corrupt;
            }
            let stored_crc = u32::from_be_bytes(crc_bytes);
            // CRC input = chunk_type(4) + data(data_len).
            let mut crc_input = Vec::with_capacity(4 + data_len);
            crc_input.extend_from_slice(chunk_type);
            crc_input.extend_from_slice(&body);
            if crc32fast::hash(&crc_input) != stored_crc {
                return CarveQuality::Corrupt;
            }
        } else {
            // Large chunk (typically IDAT): seek past body + CRC (4 B).
            let skip = data_len as u64 + 4;
            if f.seek(SeekFrom::Current(skip as i64)).is_err() {
                return CarveQuality::Corrupt;
            }
        }
    }
}

// ── validate_pdf_file ──────────────────────────────────────────────────────

/// Validate a carved PDF by verifying `%%EOF` and that `startxref N` points
/// to a recognisable cross-reference section.
///
/// The plain [`super::validate_extracted`] with `"pdf"` only checks for
/// `%%EOF` in the last 1 KiB.  On fragmented drives, PDF files often end
/// correctly but their xref table sectors have been overwritten: `startxref`
/// still appears near `%%EOF` but the offset it names now contains unrelated
/// binary data that PDF readers cannot parse.  Two concrete failure modes:
///
/// 1. `startxref 0` — offset 0 is the `%PDF-x.y` header, never an xref.
/// 2. `startxref N` where `N` is within the file but the bytes there are
///    random sector data, not `xref` or an object header.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_pdf_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    let file_size = match f.seek(SeekFrom::End(0)) {
        Ok(s) => s,
        Err(_) => return CarveQuality::Unknown,
    };

    // Read the last 1 KiB to locate %%EOF and startxref.
    let read_start = file_size.saturating_sub(1024);
    if f.seek(SeekFrom::Start(read_start)).is_err() {
        return CarveQuality::Unknown;
    }
    let mut tail = [0u8; 1024];
    let n = match f.read(&mut tail) {
        Ok(n) => n,
        Err(_) => return CarveQuality::Unknown,
    };
    let tail = &tail[..n];

    // %%EOF must be present.
    if !tail.windows(5).any(|w| w == b"%%EOF") {
        return CarveQuality::Corrupt;
    }

    // Parse the value after the LAST `startxref` keyword.
    let xref_offset = match parse_last_startxref(tail) {
        Some(v) => v,
        None => return CarveQuality::Corrupt,
    };

    // Offset 0 is always the PDF header, never a valid xref position.
    // Any offset at or beyond the file is also invalid.
    if xref_offset == 0 || xref_offset >= file_size {
        return CarveQuality::Corrupt;
    }

    // Seek to the claimed xref position and check the leading bytes.
    if f.seek(SeekFrom::Start(xref_offset)).is_err() {
        return CarveQuality::Corrupt;
    }
    let mut xref_head = [0u8; 16];
    let read = match f.read(&mut xref_head) {
        Ok(n) => n,
        Err(_) => return CarveQuality::Corrupt,
    };

    if looks_like_xref(&xref_head[..read]) {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

/// Parse the numeric value after the last `startxref` keyword in `data`.
pub(crate) fn parse_last_startxref(data: &[u8]) -> Option<u64> {
    const KW: &[u8] = b"startxref";
    let mut last: Option<u64> = None;
    let mut pos = 0;
    while pos + KW.len() <= data.len() {
        if let Some(rel) = data[pos..].windows(KW.len()).position(|w| w == KW) {
            let abs = pos + rel;
            let after = &data[abs + KW.len()..];
            // Skip line-ending / whitespace characters.
            let skip = after
                .iter()
                .position(|b| !matches!(b, b'\n' | b'\r' | b' ' | b'\t'))
                .unwrap_or(after.len());
            let after = &after[skip..];
            let digits_end = after
                .iter()
                .position(|b| !b.is_ascii_digit())
                .unwrap_or(after.len());
            if digits_end > 0 {
                if let Ok(s) = std::str::from_utf8(&after[..digits_end]) {
                    if let Ok(v) = s.parse::<u64>() {
                        last = Some(v);
                    }
                }
            }
            pos = abs + KW.len();
        } else {
            break;
        }
    }
    last
}

/// Returns `true` if `data` looks like the start of a PDF cross-reference
/// section — either a traditional xref table or a cross-reference stream.
///
/// * Traditional: begins with `xref` (optionally preceded by whitespace).
/// * Stream:      begins with an unsigned integer (the object number).
pub(crate) fn looks_like_xref(data: &[u8]) -> bool {
    // Skip leading whitespace (CR/LF/space).
    let start = data
        .iter()
        .position(|b| !matches!(b, b'\n' | b'\r' | b' ' | b'\t'))
        .unwrap_or(data.len());
    let d = &data[start..];
    if d.starts_with(b"xref") {
        return true;
    }
    d.first().is_some_and(|b| b.is_ascii_digit())
}

// ── validate_sqlite_file ───────────────────────────────────────────────────

/// Validate a carved SQLite database by inspecting its internal page structure.
///
/// SQLite stores a 100-byte file header in page 1.  Every subsequent page
/// begins with a single-byte *page type* field that must be one of:
///   - `0x00` — overflow / free page
///   - `0x02` — interior index B-tree page
///   - `0x05` — interior table B-tree page
///   - `0x0a` — leaf index B-tree page
///   - `0x0d` — leaf table B-tree page
///
/// When the carver finds the SQLite magic at a disk offset that is not the
/// real start of a database (e.g. a journaled page copy at a 4 KiB cluster
/// boundary), the pages that follow contain whatever disk data happened to be
/// there.  Their first bytes are almost never valid B-tree page type codes.
///
/// This validator:
/// 1. Confirms the 16-byte magic and reads page size + page count.
/// 2. Checks the byte at file offset 100 (schema table B-tree type).
/// 3. Samples pages 2–6 and requires **more than half** to have a valid type
///    byte.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_sqlite_file(path: &Path) -> CarveQuality {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    // Read the 100-byte SQLite file header.
    let mut hdr = [0u8; 100];
    if f.read_exact(&mut hdr).is_err() {
        return CarveQuality::Corrupt;
    }

    // Verify magic.
    if &hdr[..16] != b"SQLite format 3\x00" {
        return CarveQuality::Corrupt;
    }

    // Page size: big-endian u16 at offset 16; value 1 means 65536.
    let raw_ps = u16::from_be_bytes([hdr[16], hdr[17]]);
    let page_size: u64 = if raw_ps == 1 { 65_536 } else { raw_ps as u64 };
    if page_size < 512 {
        return CarveQuality::Corrupt;
    }

    // Page count: big-endian u32 at offset 28.
    let page_count = u32::from_be_bytes([hdr[28], hdr[29], hdr[30], hdr[31]]) as u64;
    if page_count == 0 {
        return CarveQuality::Corrupt;
    }

    const VALID_TYPES: [u8; 5] = [0x00, 0x02, 0x05, 0x0a, 0x0d];

    // Byte at file offset 100 = schema table B-tree page type.
    // We have already read 100 bytes; just read one more.
    let mut schema_type = [0u8; 1];
    if f.read_exact(&mut schema_type).is_err() {
        return CarveQuality::Corrupt;
    }
    if !VALID_TYPES.contains(&schema_type[0]) {
        return CarveQuality::Corrupt;
    }

    // Sample pages 2 through min(page_count, 6) for valid B-tree page types.
    // Pages are 0-indexed here: page index 1 = the 2nd page.
    let pages_to_check = page_count.min(6);
    let mut valid_count = 0u32;
    let mut checked = 0u32;
    for p in 1..pages_to_check {
        let offset = p * page_size;
        if f.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }
        let mut pt = [0u8; 1];
        if f.read_exact(&mut pt).is_err() {
            break;
        }
        checked += 1;
        if VALID_TYPES.contains(&pt[0]) {
            valid_count += 1;
        }
    }

    // Require strictly more than half of the sampled pages to be valid.
    if checked >= 2 && valid_count * 2 <= checked {
        return CarveQuality::Corrupt;
    }

    CarveQuality::Complete
}
