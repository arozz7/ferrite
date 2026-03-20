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
//!
//! File-based (seek) validators live in sub-modules and are re-exported:
//! - [`file_validators`]: PNG, PDF, SQLite
//! - [`binary_validators`]: EVTX, RIFF, EXE
//! - [`structural_validators`]: FLAC, ELF, REGF, TIFF

mod binary_validators;
mod file_validators;
mod structural_validators;

pub use file_validators::{validate_pdf_file, validate_png_file, validate_sqlite_file};
// parse_last_startxref and looks_like_xref remain pub(crate) for direct test access.
pub use binary_validators::{validate_evtx_file, validate_exe_file, validate_riff_file};
pub use structural_validators::{
    validate_elf_file, validate_flac_file, validate_regf_file, validate_tiff_file,
};

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
        "jpg" => validate_jpeg(head, tail),
        "png" => validate_png(head, tail, file_size),
        "gif" => validate_gif(tail),
        "pdf" => validate_pdf(tail),
        "html" => validate_html(head, tail),
        "zip" | "ole" | "7z" | "pst" | "epub" => validate_zip_eocd(tail, file_size),
        _ => CarveQuality::Unknown,
    }
}

// ── Head+tail format validators ────────────────────────────────────────────────
// NOTE: File-based (seek) validators live in file_validators.rs and
// binary_validators.rs and are re-exported at the top of this module.

/// JPEG: must end with the End-of-Image marker `FF D9`, the entropy
/// (scan) data must not contain invalid marker sequences, and no second
/// SOI (`FF D8`) may appear outside the first APP segment.
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
///
/// The embedded-SOI check catches a second failure mode: fragmented files
/// where the first cluster contains one JPEG's header and a later cluster
/// (typically at a 512-byte-aligned offset) contains a completely different
/// JPEG body.  The carver merges them into one file that no viewer can
/// decode.  A second `FF D8` after the first APP0/APP1 segment is
/// conclusive evidence of this fragmentation.
fn validate_jpeg(head: &[u8], tail: &[u8]) -> CarveQuality {
    if tail.len() < 2 || tail[tail.len() - 2..] != [0xFF, 0xD9] {
        return CarveQuality::Corrupt;
    }

    // ── Embedded-SOI / fragmentation check ──────────────────────────────
    // A JPEG Exif APP1 block (FF E1) or JFIF APP0 block (FF E0) may
    // legitimately contain a thumbnail with its own SOI.  Skip over the
    // first APP segment before searching so we don't false-positive on
    // that thumbnail.
    if head.len() >= 6 {
        // Bytes [2..3] = first marker after SOI; bytes [4..5] = segment length.
        let first_marker = head[3];
        let app_seg_end = if first_marker == 0xE0 || first_marker == 0xE1 {
            let seg_len = u16::from_be_bytes([head[4], head[5]]) as usize;
            // Segment length includes the 2-byte length field, starts at byte 2.
            (2 + seg_len).min(head.len())
        } else {
            4 // no recognised APP segment — skip past the opening SOI only
        };
        // Any SOI after the first APP segment is a fragment boundary.
        if head[app_seg_end..].windows(2).any(|w| w == [0xFF, 0xD8]) {
            return CarveQuality::Corrupt;
        }
    }

    // ── Entropy scan ────────────────────────────────────────────────────
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
                0x00 => {
                    i += 2;
                } // byte-stuffed FF
                0xD0..=0xD7 => {
                    i += 2;
                } // RST marker
                0xD9 => {
                    i += 2;
                } // EOI (shouldn't appear here but tolerate)
                0xFF => {
                    i += 1;
                } // fill byte — advance one, recheck next
                _ => {
                    return CarveQuality::Corrupt;
                } // invalid marker in scan data
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
#[cfg(test)]
mod tests_binary;
