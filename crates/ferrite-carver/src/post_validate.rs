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
//! [`validate_extracted`] takes the **tail** of the extracted file (last
//! ≤ 65 536 bytes) rather than the full content, so it is efficient even
//! for large video files.

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

/// Validate the tail bytes of a newly extracted file.
///
/// `tail` should be the last ≤ 65 536 bytes of the extracted file.
/// `file_size` is the total extracted file size in bytes (used by ZIP to
/// validate the central directory offset).
/// Pass `is_truncated = true` when the extraction hit `max_size` — the check
/// is skipped and [`CarveQuality::Truncated`] is returned immediately.
pub fn validate_extracted(
    ext: &str,
    tail: &[u8],
    is_truncated: bool,
    file_size: u64,
) -> CarveQuality {
    if is_truncated {
        return CarveQuality::Truncated;
    }
    match ext {
        "jpg" => validate_jpeg(tail),
        "png" => validate_png(tail),
        "gif" => validate_gif(tail),
        "pdf" => validate_pdf(tail),
        "zip" | "ole" | "7z" | "pst" => validate_zip_eocd(tail, file_size),
        _ => CarveQuality::Unknown,
    }
}

// ── Format validators ─────────────────────────────────────────────────────────

/// JPEG: must end with the End-of-Image marker `FF D9`.
fn validate_jpeg(tail: &[u8]) -> CarveQuality {
    if tail.len() >= 2 && tail[tail.len() - 2..] == [0xFF, 0xD9] {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
}

/// PNG: the last 12 bytes must be the IEND chunk with its fixed CRC.
///
/// `00 00 00 00  49 45 4E 44  AE 42 60 82`
fn validate_png(tail: &[u8]) -> CarveQuality {
    const IEND: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, // chunk length = 0
        0x49, 0x45, 0x4E, 0x44, // "IEND"
        0xAE, 0x42, 0x60, 0x82, // CRC-32 of "IEND"
    ];
    if tail.len() >= 12 && tail[tail.len() - 12..] == *IEND {
        CarveQuality::Complete
    } else {
        CarveQuality::Corrupt
    }
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_truncated short-circuit ────────────────────────────────────────────

    #[test]
    fn truncated_flag_returns_truncated_regardless_of_ext() {
        assert_eq!(
            validate_extracted("jpg", &[0xFF, 0xD9], true, 2),
            CarveQuality::Truncated
        );
        assert_eq!(
            validate_extracted("unknown", &[], true, 0),
            CarveQuality::Truncated
        );
    }

    // ── JPEG ─────────────────────────────────────────────────────────────────

    #[test]
    fn jpeg_complete_with_eoi_marker() {
        let tail = &[0x00u8, 0x01, 0xFF, 0xD9];
        assert_eq!(
            validate_extracted("jpg", tail, false, 4),
            CarveQuality::Complete
        );
    }

    #[test]
    fn jpeg_corrupt_without_eoi() {
        let tail = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        assert_eq!(
            validate_extracted("jpg", tail, false, 5),
            CarveQuality::Corrupt
        );
    }

    #[test]
    fn jpeg_corrupt_on_empty_data() {
        assert_eq!(
            validate_extracted("jpg", &[], false, 0),
            CarveQuality::Corrupt
        );
    }

    #[test]
    fn jpeg_corrupt_when_only_one_byte() {
        assert_eq!(
            validate_extracted("jpg", &[0xD9], false, 1),
            CarveQuality::Corrupt
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
            validate_extracted("png", &tail, false, 16),
            CarveQuality::Complete
        );
    }

    #[test]
    fn png_corrupt_missing_iend() {
        let tail = &[
            0x89u8, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            validate_extracted("png", tail, false, 12),
            CarveQuality::Corrupt
        );
    }

    #[test]
    fn png_corrupt_on_empty() {
        assert_eq!(
            validate_extracted("png", &[], false, 0),
            CarveQuality::Corrupt
        );
    }

    // ── GIF ──────────────────────────────────────────────────────────────────

    #[test]
    fn gif_complete_with_trailer() {
        let tail = b"GIF89a\x3B";
        assert_eq!(
            validate_extracted("gif", tail, false, 7),
            CarveQuality::Complete
        );
    }

    #[test]
    fn gif_corrupt_missing_trailer() {
        let tail = b"GIF89a";
        assert_eq!(
            validate_extracted("gif", tail, false, 6),
            CarveQuality::Corrupt
        );
    }

    #[test]
    fn gif_corrupt_on_empty() {
        assert_eq!(
            validate_extracted("gif", &[], false, 0),
            CarveQuality::Corrupt
        );
    }

    // ── PDF ──────────────────────────────────────────────────────────────────

    #[test]
    fn pdf_complete_with_eof_marker() {
        let tail = b"%PDF-1.4\n...content...\n%%EOF\n";
        assert_eq!(
            validate_extracted("pdf", tail, false, tail.len() as u64),
            CarveQuality::Complete
        );
    }

    #[test]
    fn pdf_corrupt_without_eof() {
        let tail = b"%PDF-1.4\n...content...";
        assert_eq!(
            validate_extracted("pdf", tail, false, tail.len() as u64),
            CarveQuality::Corrupt
        );
    }

    #[test]
    fn pdf_complete_eof_within_last_1kb() {
        let mut tail = vec![0u8; 2000];
        // Put %%EOF at byte 1800 (within last 1 KiB of the 2000-byte tail).
        tail[1800..1805].copy_from_slice(b"%%EOF");
        assert_eq!(
            validate_extracted("pdf", &tail, false, 2000),
            CarveQuality::Complete
        );
    }

    #[test]
    fn pdf_corrupt_eof_outside_last_1kb() {
        let mut tail = vec![0u8; 2000];
        // Put %%EOF at byte 100 (more than 1 KiB from the end — not searched).
        tail[100..105].copy_from_slice(b"%%EOF");
        assert_eq!(
            validate_extracted("pdf", &tail, false, 2000),
            CarveQuality::Corrupt
        );
    }

    // ── ZIP / EOCD ────────────────────────────────────────────────────────────

    #[test]
    fn zip_complete_with_eocd() {
        // EOCD with cd_offset = 0 (fits within any file)
        let mut tail = vec![0u8; 32];
        tail[10..14].copy_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
        // cd_offset at EOCD+16 = tail[26..30] = 0 (already zeroed)
        assert_eq!(
            validate_extracted("zip", &tail, false, 32),
            CarveQuality::Complete
        );
    }

    #[test]
    fn zip_corrupt_missing_eocd() {
        let tail = b"PK\x03\x04some zip content without central dir";
        assert_eq!(
            validate_extracted("zip", tail, false, tail.len() as u64),
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
            validate_extracted("zip", &tail, false, 32),
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
            validate_extracted("zip", &tail, false, 50_000),
            CarveQuality::Complete
        );
    }

    #[test]
    fn zip_complete_on_ole_extension() {
        let mut tail = vec![0u8; 32];
        tail[0..4].copy_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
        assert_eq!(
            validate_extracted("ole", &tail, false, 32),
            CarveQuality::Complete
        );
    }

    // ── Unknown formats ──────────────────────────────────────────────────────

    #[test]
    fn unknown_format_returns_unknown() {
        assert_eq!(
            validate_extracted("mp4", &[0u8; 32], false, 32),
            CarveQuality::Unknown
        );
        assert_eq!(
            validate_extracted("mkv", &[0u8; 32], false, 32),
            CarveQuality::Unknown
        );
        assert_eq!(
            validate_extracted("avi", &[0u8; 32], false, 32),
            CarveQuality::Unknown
        );
    }
}
