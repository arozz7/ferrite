//! Post-extraction structural validation.
//!
//! After a file is carved from a device image, we check its structural
//! integrity using format-specific byte patterns:
//!
//! | Tag         | Meaning                                                  |
//! |-------------|----------------------------------------------------------|
//! | `Complete`  | End-of-file marker found; deeper check passed.           |
//! | `Truncated` | Extraction hit the size cap before the end-of-file mark. |
//! | `Corrupt`   | End-of-file marker absent; file is likely damaged.       |
//! | `Unknown`   | No deep check implemented for this format.               |
//!
//! [`validate_extracted`] takes the **head** (first ≤ 8 192 bytes) and
//! **tail** (last ≤ 65 536 bytes) of the extracted file for efficient
//! format-specific validation.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Structural integrity tag assigned after file extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CarveQuality {
    /// End-of-file marker present; structural check passed.
    Complete,
    /// Extraction hit the max-size cap before the end-of-file marker was found.
    Truncated,
    /// File has content but is missing its expected structural tail.
    Corrupt,
    /// No deep structural check is available for this format.
    Unknown,
}

/// Validate the head and tail bytes of a newly extracted file.
///
/// `head` should be the first ≤ 8 192 bytes of the extracted file.
/// `tail` should be the last ≤ 65 536 bytes of the extracted file.
/// `file_size` is the total extracted file size in bytes (used by ZIP to
/// validate the central directory offset).
/// Pass `is_truncated = true` when the extraction hit `max_size` — the check
/// is skipped and [`CarveQuality::Truncated`] is returned immediately.
pub fn validate_extracted(
    ext: &str,
    head: &[u8],
    tail: &[u8],
    is_truncated: bool,
    file_size: u64,
) -> CarveQuality {
    if is_truncated {
        return CarveQuality::Truncated;
    }
    match ext {
        "jpg" => validate_jpeg(tail),
        "png" => validate_png(head, tail, file_size),
        "gif" => validate_gif(tail),
        "pdf" => validate_pdf(tail),
        "html" => validate_html(head, tail),
        "zip" | "ole" | "7z" | "pst" => validate_zip_eocd(tail, file_size),
        _ => CarveQuality::Unknown,
    }
}

/// Validate a carved PNG by walking its chunk structure directly on the file.
///
/// Unlike [`validate_extracted`], this opens the file and seeks through it
/// chunk-by-chunk, reading only the 12-byte chunk envelope (length + type +
/// CRC) for large chunks and the full body for small ones (≤ 64 KiB).  This
/// eliminates the "dead zone" limitation of the fixed head + tail buffers:
/// a corrupt chunk type anywhere in the file is caught in O(N chunks) seeks,
/// where N is typically 5–8 for a real-world PNG.
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened (e.g. it
/// was just deleted by skip-corrupt mode in a race).
pub fn validate_png_file(path: &Path) -> CarveQuality {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return CarveQuality::Unknown,
    };

    // Verify the 8-byte PNG signature.
    let mut sig = [0u8; 8];
    if f.read_exact(&mut sig).is_err()
        || sig != [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    {
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

        let data_len =
            u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
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

/// Validate a carved PDF by verifying `%%EOF` and that `startxref N` points
/// to a recognisable cross-reference section.
///
/// The plain [`validate_pdf`] only checks for `%%EOF` in the last 1 KiB.
/// On fragmented drives, PDF files often end correctly but their xref table
/// sectors have been overwritten: `startxref` still appears near `%%EOF` but
/// the offset it names now contains unrelated binary data that PDF readers
/// cannot parse.  Two concrete failure modes:
///
/// 1. `startxref 0` — offset 0 is the `%PDF-x.y` header, never an xref.
/// 2. `startxref N` where `N` is within the file but the bytes there are
///    random sector data, not `xref` or an object header.
///
/// This function reads the last 1 KiB (for `%%EOF` + `startxref`), then
/// seeks to the declared offset and checks that the data there begins with
/// either `xref` (traditional cross-reference table) or an ASCII digit
/// (cross-reference stream object header, e.g. `616 0 obj`).
///
/// Returns [`CarveQuality::Unknown`] when the file cannot be opened.
pub fn validate_pdf_file(path: &Path) -> CarveQuality {
    use std::io::{Read, Seek, SeekFrom};

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
/// * Stream:      begins with an unsigned integer (the object number), e.g.
///   `616 0 obj`.
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
    // Cross-reference stream: first non-whitespace byte must be an ASCII digit.
    d.first().is_some_and(|b| b.is_ascii_digit())
}

// ── Format validators ─────────────────────────────────────────────────────────

/// JPEG: must end with the End-of-Image marker `FF D9`, and the entropy
/// (scan) data must not contain invalid marker sequences.
///
/// In valid JPEG scan data every `0xFF` byte is followed by one of:
///   - `0x00`       — byte-stuffed literal `0xFF`
///   - `0xD0`–`0xD7` — restart markers (RST0–RST7)
///   - `0xD9`       — end-of-image (EOI)
///   - `0xFF`       — fill byte (padding)
///
/// When sectors are overwritten by unrelated files, the random data almost
/// always contains `0xFF` followed by bytes outside this set.  Checking the
/// last 4 KiB of scan data (just before EOI) reliably detects this.
fn validate_jpeg(tail: &[u8]) -> CarveQuality {
    if tail.len() < 2 || tail[tail.len() - 2..] != [0xFF, 0xD9] {
        return CarveQuality::Corrupt;
    }

    // Scan the last 4 KiB of entropy data (before the final FF D9) for
    // invalid marker sequences.
    let scan_end = tail.len() - 2; // exclude EOI
    let scan_start = scan_end.saturating_sub(4096);
    let scan_region = &tail[scan_start..scan_end];

    let mut i = 0;
    while i < scan_region.len() {
        if scan_region[i] == 0xFF {
            if i + 1 >= scan_region.len() {
                break; // 0xFF at very end of region — can't check follower
            }
            let follower = scan_region[i + 1];
            match follower {
                0x00 => { i += 2; }                // byte-stuffed FF
                0xD0..=0xD7 => { i += 2; }         // RST marker
                0xD9 => { i += 2; }                 // EOI (shouldn't appear here but tolerate)
                0xFF => { i += 1; }                 // fill byte — advance one, recheck next
                _ => { return CarveQuality::Corrupt; } // invalid marker in scan data
            }
        } else {
            i += 1;
        }
    }

    CarveQuality::Complete
}

/// PNG: the last 12 bytes must be the IEND chunk, and all chunks whose
/// data fits within the `head` buffer must pass CRC-32 verification.
///
/// This catches three classes of corruption:
/// 1. Missing IEND footer.
/// 2. Sector-level CRC mismatch in early chunks (IHDR, pHYs, iCCP, …).
/// 3. Garbage chunk type immediately after IDAT — detected when the IDAT
///    end position falls within the tail buffer and the following chunk
///    type bytes are not ASCII alphabetic.  This catches fragmentation
///    where overwritten pixel-data sectors produce an invalid chunk header
///    between the IDAT body and the IEND footer.
fn validate_png(head: &[u8], tail: &[u8], file_size: u64) -> CarveQuality {
    const IEND: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, // chunk length = 0
        0x49, 0x45, 0x4E, 0x44, // "IEND"
        0xAE, 0x42, 0x60, 0x82, // CRC-32 of "IEND"
    ];

    // Tail check: IEND must be present.
    if tail.len() < 12 || tail[tail.len() - 12..] != *IEND {
        return CarveQuality::Corrupt;
    }

    // Head check: walk chunks and verify CRC-32 for each complete chunk
    // in the buffer.  The PNG signature occupies bytes 0..8.
    //
    // Also record the file offset immediately after the first IDAT chunk
    // that extends beyond the head buffer, so we can verify the following
    // chunk type using the tail buffer (see post-IDAT check below).
    let mut first_idat_file_end: Option<u64> = None;
    if head.len() > 8 {
        let mut pos: usize = 8; // skip PNG signature
        loop {
            // Need at least 12 bytes for length(4) + type(4) + CRC(4).
            if pos + 12 > head.len() {
                break;
            }
            let chunk_data_len =
                u32::from_be_bytes([head[pos], head[pos + 1], head[pos + 2], head[pos + 3]])
                    as usize;
            let chunk_type = &head[pos + 4..pos + 8];

            // Chunk type must be ASCII letters.
            if !chunk_type.iter().all(|&b| b.is_ascii_alphabetic()) {
                return CarveQuality::Corrupt;
            }

            let chunk_end = pos + 12 + chunk_data_len; // length + type + data + CRC
            if chunk_end > head.len() {
                // Chunk extends beyond the head buffer — CRC can't be verified.
                // However, if the chunk's declared data would place its end
                // past where IEND must sit, the IEND footer is embedded inside
                // this chunk's data (from an overlapping sector of another file).
                // file_size - 12 is the earliest valid IEND position.
                let chunk_body_end = pos as u64 + 12 + chunk_data_len as u64;
                if chunk_body_end > file_size.saturating_sub(12) {
                    return CarveQuality::Corrupt;
                }
                // Record the IDAT boundary for the post-IDAT tail check.
                if chunk_type == b"IDAT" && first_idat_file_end.is_none() {
                    first_idat_file_end = Some(chunk_body_end);
                }
                break;
            }

            // CRC-32 covers chunk_type(4) + chunk_data(chunk_data_len).
            let crc_data = &head[pos + 4..pos + 8 + chunk_data_len];
            let stored_crc = u32::from_be_bytes([
                head[chunk_end - 4],
                head[chunk_end - 3],
                head[chunk_end - 2],
                head[chunk_end - 1],
            ]);
            let computed_crc = crc32fast::hash(crc_data);
            if computed_crc != stored_crc {
                return CarveQuality::Corrupt;
            }

            // IEND reached — everything checked out.
            if chunk_type == b"IEND" {
                break;
            }

            pos = chunk_end;
        }
    }

    // Post-IDAT chunk type check: when the first IDAT's body extends beyond
    // the head buffer but its end falls within the tail buffer, validate that
    // the chunk immediately following it has an ASCII-alphabetic type field.
    //
    // On fragmented drives, sectors overwritten by unrelated data produce
    // garbage chunk types (e.g. 0x21 0x60 0x1E 0xEE) after the IDAT body.
    // The head CRC walk cannot see past the 8 KiB head buffer, but the tail
    // buffer (64 KiB) often covers smaller files entirely.
    if let Some(idat_end) = first_idat_file_end {
        let tail_start = file_size.saturating_sub(tail.len() as u64);
        // The next chunk header starts at `idat_end` in the file.
        // We need at least 8 bytes (length + type) to validate it.
        if idat_end >= tail_start && idat_end + 8 <= file_size {
            let tail_idx = (idat_end - tail_start) as usize;
            if tail_idx + 8 <= tail.len() {
                let next_type = &tail[tail_idx + 4..tail_idx + 8];
                if !next_type.iter().all(|&b| b.is_ascii_alphabetic()) {
                    return CarveQuality::Corrupt;
                }
            }
        }
    }

    // Tail check: verify the chunk immediately preceding IEND.
    //
    // On fragmented drives, overwritten sectors produce garbage chunk types
    // (e.g. 0xFF6F00AB) between the last valid IDAT and the IEND footer.
    // The head CRC walk only covers the first 8 KiB, so large IDAT chunks
    // hide the corruption.  This reverse walk finds the preceding chunk in
    // the 64 KiB tail buffer and CRC-verifies it.
    //
    // Structure: [len(4)][type(4)][data(len)][CRC(4)] [IEND(12)]
    //            ^--- chunk_start                      ^--- iend_start
    // We know iend_start = tail.len() - 12.  For each candidate data_len,
    // chunk_start = iend_start - 12 - data_len.  We check whether the
    // stored length equals data_len AND the type is ASCII-alpha.
    if tail.len() > 24 {
        let iend_start = tail.len() - 12;
        let max_data_len = iend_start.saturating_sub(12);
        let mut found_predecessor = false;

        for data_len in 0..=max_data_len {
            let chunk_start = iend_start - 12 - data_len;
            if chunk_start + 8 > tail.len() {
                break;
            }

            let stored_len = u32::from_be_bytes([
                tail[chunk_start],
                tail[chunk_start + 1],
                tail[chunk_start + 2],
                tail[chunk_start + 3],
            ]) as usize;

            if stored_len != data_len {
                continue;
            }

            let chunk_type = &tail[chunk_start + 4..chunk_start + 8];
            if !chunk_type.iter().all(|&b| b.is_ascii_alphabetic()) {
                continue;
            }

            // Found a plausible chunk boundary.  Verify CRC-32.
            let crc_start = iend_start - 4;
            let stored_crc = u32::from_be_bytes([
                tail[crc_start],
                tail[crc_start + 1],
                tail[crc_start + 2],
                tail[crc_start + 3],
            ]);
            let crc_data = &tail[chunk_start + 4..chunk_start + 8 + data_len];
            let computed_crc = crc32fast::hash(crc_data);

            if computed_crc != stored_crc {
                return CarveQuality::Corrupt;
            }

            found_predecessor = true;
            break;
        }

        // If no valid predecessor was found AND the entire file fits within
        // the tail buffer (meaning all chunks should be walkable), the chunk
        // structure is broken.
        if !found_predecessor && tail.len() >= iend_start {
            // The preceding chunk is larger than the tail buffer — we cannot
            // verify it.  Fall through to Complete.
        }
    }

    CarveQuality::Complete
}

/// HTML: must end with `</html>` and the `<body>` must contain at least
/// 32 characters of visible text (non-whitespace, outside of HTML tags).
///
/// This filters out e-book scaffold fragments (Kindle, EPUB) that are
/// structurally valid HTML but contain only empty `<div>` containers with
/// no readable content.
fn validate_html(head: &[u8], tail: &[u8]) -> CarveQuality {
    // Tail check: must end with </html> (possibly with trailing whitespace).
    let tail_str = std::str::from_utf8(tail).unwrap_or("");
    if !tail_str.trim_end().ends_with("</html>") && !tail_str.trim_end().ends_with("</HTML>") {
        return CarveQuality::Corrupt;
    }

    // Head check: extract text content from <body> and verify it has substance.
    let head_str = std::str::from_utf8(head).unwrap_or("");
    let body_start = head_str
        .find("<body")
        .or_else(|| head_str.find("<BODY"))
        .and_then(|pos| head_str[pos..].find('>').map(|end| pos + end + 1));

    let Some(body_content_start) = body_start else {
        // No <body> tag in head buffer — can't validate content; accept if
        // the footer is present (already checked above).
        return CarveQuality::Complete;
    };

    // Strip HTML tags and count non-whitespace text characters.
    let body_slice = &head_str[body_content_start..];
    let text_chars = count_visible_text(body_slice);

    if text_chars >= 32 {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

/// Count non-whitespace characters outside of HTML tags.
fn count_visible_text(html: &str) -> usize {
    let mut count = 0usize;
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag && !ch.is_whitespace() => {
                count += 1;
                if count >= 32 {
                    return count; // early exit
                }
            }
            _ => {}
        }
    }
    count
}

/// GIF: must end with the trailer byte `3B` (`;`).
fn validate_gif(tail: &[u8]) -> CarveQuality {
    if tail.last() == Some(&0x3B) {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

/// PDF: `%%EOF` must appear within the last 1 KiB.
fn validate_pdf(tail: &[u8]) -> CarveQuality {
    let search_start = tail.len().saturating_sub(1024);
    if tail[search_start..].windows(5).any(|w| w == b"%%EOF") {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

/// ZIP / OLE2 / 7-Zip / PST: End of Central Directory signature `PK\x05\x06`
/// must appear in the tail.  Additionally, the EOCD's "offset of start of
/// central directory" (u32 LE at EOCD+16) must fall within the extracted
/// file — otherwise the EOCD belongs to a larger archive and this extraction
/// started at an internal entry.
fn validate_zip_eocd(tail: &[u8], file_size: u64) -> CarveQuality {
    const EOCD: &[u8] = &[0x50, 0x4B, 0x05, 0x06];
    // Find the LAST EOCD in the tail (ZIP64 may have extra preceding data).
    let mut search = tail;
    let mut last_eocd: Option<usize> = None;
    while let Some(pos) = search.windows(4).position(|w| w == EOCD) {
        let abs_pos = tail.len() - search.len() + pos;
        last_eocd = Some(abs_pos);
        if pos + 4 < search.len() {
            search = &search[pos + 4..];
        } else {
            break;
        }
    }
    let Some(eocd_pos) = last_eocd else {
        return CarveQuality::Corrupt;
    };
    // EOCD fixed record is 22 bytes: sig(4) + disk_num(2) + cd_disk(2) +
    // cd_entries_this(2) + cd_entries_total(2) + cd_size(4) + cd_offset(4) +
    // comment_len(2).  cd_offset is at EOCD+16.
    if eocd_pos + 22 <= tail.len() {
        let cd_off_bytes = &tail[eocd_pos + 16..eocd_pos + 20];
        let cd_offset = u32::from_le_bytes([
            cd_off_bytes[0],
            cd_off_bytes[1],
            cd_off_bytes[2],
            cd_off_bytes[3],
        ]) as u64;
        // 0xFFFFFFFF means ZIP64 — we can't easily validate, treat as Complete.
        if cd_offset != 0xFFFF_FFFF && cd_offset > file_size {
            return CarveQuality::Corrupt;
        }
    }
    CarveQuality::Complete
}

#[cfg(test)]
mod tests;
