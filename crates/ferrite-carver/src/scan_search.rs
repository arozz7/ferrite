//! Signature-matching helpers used by [`Carver::scan_impl`].
//!
//! Kept separate from `scanner.rs` to keep both files under the 600-line limit.

use crate::scanner::CarveHit;
use crate::signature::Signature;

// ── Chunk search ──────────────────────────────────────────────────────────────

/// Return all positions within `data[..report_end]` where `sig.header` begins.
///
/// Uses [`memchr`] on the first fixed (non-wildcard) byte for fast scanning,
/// then verifies the full pattern including `??` wildcard positions.
pub(crate) fn find_all(
    sig: &Signature,
    data: &[u8],
    chunk_abs_offset: u64,
    report_end: usize,
) -> Vec<CarveHit> {
    let header = &sig.header;
    if header.is_empty() || data.is_empty() {
        return vec![];
    }

    // Find the first fixed byte to use as the memchr anchor.
    let Some((anchor_idx, anchor_byte)) = header
        .iter()
        .enumerate()
        .find_map(|(i, b)| b.map(|byte| (i, byte)))
    else {
        return vec![]; // all-wildcard header — refuse to match everything
    };

    let report_end = report_end.min(data.len());
    let mut hits = Vec::new();
    // Search window starts at anchor_idx so we can back-compute the header start.
    let mut search_start = anchor_idx;

    loop {
        if search_start >= report_end + anchor_idx {
            break;
        }
        let scan_end = (report_end + anchor_idx).min(data.len());
        let window = &data[search_start..scan_end];
        let Some(rel) = memchr::memchr(anchor_byte, window) else {
            break;
        };
        // Position of the anchor byte in data[].
        let anchor_pos = search_start + rel;
        // Position where the header would start.
        let pos = anchor_pos.saturating_sub(anchor_idx);

        if pos + header.len() <= data.len()
            && pos < report_end
            && header_matches(header, data, pos)
            && (!sig.pre_validate_zip || zip_local_header_is_file(data, pos))
        {
            hits.push(CarveHit {
                byte_offset: chunk_abs_offset + pos as u64,
                signature: sig.clone(),
            });
        }
        search_start = anchor_pos + 1;
    }

    hits
}

// ── ZIP pre-validator ─────────────────────────────────────────────────────────

/// Returns `true` when the ZIP local file header at `data[pos..]` looks like
/// a real file entry (not a directory entry, not implausible field values).
///
/// ZIP local file header layout (all offsets relative to `pos`):
/// ```text
/// 0-3   PK\x03\x04 magic (already matched by header scan)
/// 4-5   version needed to extract (u16 LE)
/// 6-7   general purpose bit flag (u16 LE)
/// 8-9   compression method (u16 LE)
/// 10-11 last mod file time
/// 12-13 last mod file date
/// 14-17 CRC-32
/// 18-21 compressed size (u32 LE) — may be 0 with data descriptor
/// 22-25 uncompressed size (u32 LE) — may be 0 with data descriptor
/// 26-27 file name length (u16 LE)
/// 28-29 extra field length (u16 LE)
/// 30+   file name bytes
/// ```
///
/// Filters when any of the following hold:
/// - Header too short to parse (< 30 bytes available)
/// - `version_needed` > 63 (no known extractor beyond 6.3)
/// - `compression_method` not in the PKWARE-registered set
/// - `file_name_length` is 0 or > 512
/// - Filename ends with `/` (directory entry — contains no file data)
pub(crate) fn zip_local_header_is_file(data: &[u8], pos: usize) -> bool {
    const MIN_HDR: usize = 30;
    if pos + MIN_HDR > data.len() {
        // Not enough data in this chunk to validate; give benefit of the doubt.
        return true;
    }

    let version_needed = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
    let compression = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
    let fname_len = u16::from_le_bytes([data[pos + 26], data[pos + 27]]) as usize;

    // Reject implausible version (highest ever defined by PKWARE is 6.3 = 63).
    if version_needed > 63 {
        return false;
    }
    // Reject zero-length or absurdly long filenames.
    if fname_len == 0 || fname_len > 512 {
        return false;
    }
    // Reject unknown compression methods.  PKWARE APPNOTE.TXT registered set:
    // 0=Store, 8=Deflate, 9=Deflate64, 12=BZIP2, 14=LZMA, 19=LZ77,
    // 93=Zstd, 95=XZ, 96=JPEG, 97=WavPack, 98=PPMd, 99=AE-x encryption.
    const VALID_METHODS: &[u16] = &[0, 8, 9, 12, 14, 19, 93, 95, 96, 97, 98, 99];
    if !VALID_METHODS.contains(&compression) {
        return false;
    }
    // Reject directory entries (filename ends with '/').
    if pos + MIN_HDR + fname_len <= data.len() && data[pos + MIN_HDR + fname_len - 1] == b'/' {
        return false;
    }

    true
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::Signature;

    /// Build a minimal ZIP local file header at `buf[pos..]`.
    ///
    /// `fname` is the filename string to embed.  All other fields are zeroed
    /// (version=20, compression=8=Deflate) except where the test overrides them.
    fn make_zip_lfh(fname: &str, version_needed: u16, compression: u16) -> Vec<u8> {
        let fname_bytes = fname.as_bytes();
        let mut buf = vec![0u8; 30 + fname_bytes.len()];
        buf[0..4].copy_from_slice(b"PK\x03\x04");
        buf[4..6].copy_from_slice(&version_needed.to_le_bytes());
        buf[8..10].copy_from_slice(&compression.to_le_bytes());
        let flen = fname_bytes.len() as u16;
        buf[26..28].copy_from_slice(&flen.to_le_bytes());
        buf[30..30 + fname_bytes.len()].copy_from_slice(fname_bytes);
        buf
    }

    fn zip_sig() -> Signature {
        Signature {
            name: "ZIP".into(),
            extension: "zip".into(),
            header: vec![Some(0x50), Some(0x4B), Some(0x03), Some(0x04)],
            footer: vec![0x50, 0x4B, 0x05, 0x06],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate_zip: true,
        }
    }

    #[test]
    fn zip_directory_entry_filtered() {
        let data = make_zip_lfh("patch/", 20, 8);
        assert!(
            !zip_local_header_is_file(&data, 0),
            "directory entry ('patch/') should be rejected"
        );
    }

    #[test]
    fn zip_file_entry_kept() {
        let data = make_zip_lfh("readme.txt", 20, 8);
        assert!(
            zip_local_header_is_file(&data, 0),
            "regular file entry should be accepted"
        );
    }

    #[test]
    fn zip_invalid_version_filtered() {
        let data = make_zip_lfh("file.bin", 200, 8);
        assert!(
            !zip_local_header_is_file(&data, 0),
            "version > 63 should be rejected"
        );
    }

    #[test]
    fn zip_unknown_compression_filtered() {
        let data = make_zip_lfh("file.bin", 20, 255);
        assert!(
            !zip_local_header_is_file(&data, 0),
            "unknown compression method should be rejected"
        );
    }

    #[test]
    fn zip_zero_fname_len_filtered() {
        let mut data = make_zip_lfh("x", 20, 8);
        // Overwrite fname_len with 0.
        data[26] = 0;
        data[27] = 0;
        assert!(
            !zip_local_header_is_file(&data, 0),
            "zero filename length should be rejected"
        );
    }

    #[test]
    fn zip_pre_validate_drops_dir_hit_in_find_all() {
        let sig = zip_sig();
        // Build a buffer with a directory-entry header at pos 0.
        let data = make_zip_lfh("patch/", 20, 8);
        let hits = find_all(&sig, &data, 0, data.len());
        assert!(
            hits.is_empty(),
            "find_all should produce no hit for a directory-entry ZIP header"
        );
    }

    #[test]
    fn zip_pre_validate_keeps_file_hit_in_find_all() {
        let sig = zip_sig();
        let data = make_zip_lfh("document.pdf", 20, 8);
        let hits = find_all(&sig, &data, 0, data.len());
        assert_eq!(
            hits.len(),
            1,
            "find_all should produce exactly one hit for a file-entry ZIP header"
        );
    }
}

/// Check whether `header` matches `data` starting at `pos`.
///
/// `None` entries in `header` are wildcards and match any byte.
#[inline]
pub(crate) fn header_matches(header: &[Option<u8>], data: &[u8], pos: usize) -> bool {
    header.iter().enumerate().all(|(i, opt)| match opt {
        None => true,
        Some(b) => data.get(pos + i) == Some(b),
    })
}
